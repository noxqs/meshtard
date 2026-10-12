//! FBX export: binary FBX 7.4 (7400), the version Blender and most tools read (Blender
//! does not read ASCII FBX). The record layout, null records, header and footer
//! follow Blender's `encode_bin.py` (the footer's fixed bytes are the ones every
//! FBX SDK file carries); objects follow the FBX 7.4 document structure (header
//! extension, `GlobalSettings`, `Definitions`, `Objects`, `Connections`).
//!
//! - Options after SketchUp's FBX exporter (help.sketchup.com, "Exporting FBX
//!   files"): "Export Two-Sided Faces", "Export Texture Maps", "Swap YZ Coordinates
//!   (Y Is Up)", "Units" and "Separate Disconnected Faces"; faces are always
//!   triangulated ("Export Only Current Selection" is done by the app).
//! - One mesh model with one geometry (welded vertices, per-corner normals, one UV
//!   layer, a material index per triangle), or with "Separate Disconnected Faces" one
//!   per connected part (triangles sharing a corner, named `<name>_1`, `<name>_2`, ...
//!   in the order of their first triangle), and one material per used model material
//!   (diffuse color, opacity, diffuse texture file written next to the `.fbx`). The
//!   group and component hierarchy is flattened.
//! - `GlobalSettings` records the axes (Z up, or Y up with the swap) and the unit as
//!   `UnitScaleFactor` (centimeters per unit).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write as _;

use glam::DVec3;

use super::{ExportOptions, Files, groups, texture_files};
use crate::model::Model;

/// An FBX property value.
enum P {
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Str(String),
    Raw(Vec<u8>),
    F64s(Vec<f64>),
    I32s(Vec<i32>),
}

/// An FBX node to write.
struct N {
    name: &'static str,
    props: Vec<P>,
    children: Vec<N>,
}

fn n(name: &'static str, props: Vec<P>) -> N {
    N {
        name,
        props,
        children: Vec::new(),
    }
}

fn nc(name: &'static str, props: Vec<P>, children: Vec<N>) -> N {
    N {
        name,
        props,
        children,
    }
}

fn s(x: &str) -> P {
    P::Str(x.to_owned())
}

const NULL_RECORD: [u8; 13] = [0; 13];

fn array(out: &mut Vec<u8>, ty: u8, count: usize, raw: Vec<u8>) {
    out.push(ty);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    // Arrays over 128 bytes are zlib-compressed, as the FBX SDK and Blender do.
    if raw.len() > 128 {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        let packed = z
            .write_all(&raw)
            .and_then(|()| z.finish())
            .expect("writing to memory");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
    } else {
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&raw);
    }
}

fn write_prop(out: &mut Vec<u8>, p: &P) {
    match p {
        P::Bool(b) => out.extend_from_slice(&[b'C', u8::from(*b)]),
        P::I32(x) => {
            out.push(b'I');
            out.extend_from_slice(&x.to_le_bytes());
        }
        P::I64(x) => {
            out.push(b'L');
            out.extend_from_slice(&x.to_le_bytes());
        }
        P::F64(x) => {
            out.push(b'D');
            out.extend_from_slice(&x.to_le_bytes());
        }
        P::Str(x) => {
            out.push(b'S');
            out.extend_from_slice(&(x.len() as u32).to_le_bytes());
            out.extend_from_slice(x.as_bytes());
        }
        P::Raw(x) => {
            out.push(b'R');
            out.extend_from_slice(&(x.len() as u32).to_le_bytes());
            out.extend_from_slice(x);
        }
        P::F64s(v) => array(
            out,
            b'd',
            v.len(),
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        ),
        P::I32s(v) => array(
            out,
            b'i',
            v.len(),
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        ),
    }
}

/// Writes a node record; `last` is whether it ends its parent's list (a node with
/// neither properties nor children gets a null record unless last, as Blender does).
fn write_node(out: &mut Vec<u8>, node: &N, last: bool) {
    let start = out.len();
    out.extend_from_slice(&[0; 12]);
    out.push(node.name.len() as u8);
    out.extend_from_slice(node.name.as_bytes());
    let props_start = out.len();
    for p in &node.props {
        write_prop(out, p);
    }
    let props_len = out.len() - props_start;
    write_children(out, &node.children, node.props.is_empty() && !last);
    let end = out.len() as u32;
    out[start..start + 4].copy_from_slice(&end.to_le_bytes());
    out[start + 4..start + 8].copy_from_slice(&(node.props.len() as u32).to_le_bytes());
    out[start + 8..start + 12].copy_from_slice(&(props_len as u32).to_le_bytes());
}

fn write_children(out: &mut Vec<u8>, children: &[N], null_when_empty: bool) {
    for (i, c) in children.iter().enumerate() {
        write_node(out, c, i + 1 == children.len());
    }
    if !children.is_empty() || null_when_empty {
        out.extend_from_slice(&NULL_RECORD);
    }
}

/// A `Properties70` entry.
fn pp(name: &str, ty: &str, label: &str, flags: &str, values: Vec<P>) -> N {
    let mut props = vec![s(name), s(ty), s(label), s(flags)];
    props.extend(values);
    n("P", props)
}

fn p_int(name: &str, v: i32) -> N {
    pp(name, "int", "Integer", "", vec![P::I32(v)])
}

fn p_double(name: &str, v: f64) -> N {
    pp(name, "double", "Number", "", vec![P::F64(v)])
}

fn p_color(name: &str, c: [f64; 3]) -> N {
    pp(name, "Color", "", "A", c.map(P::F64).into())
}

/// `name\0\x01class`, the binary form of an object name.
fn object_name(name: &str, class: &str) -> P {
    P::Str(format!("{name}\u{0}\u{1}{class}"))
}

/// A triangle to write: material slot, corners, texture coordinates.
type Tri = (usize, [DVec3; 3], [[f64; 2]; 3]);

fn find(parent: &mut [usize], mut a: usize) -> usize {
    while parent[a] != a {
        parent[a] = parent[parent[a]];
        a = parent[a];
    }
    a
}

/// "Separate Disconnected Faces": the triangles grouped by shared corners (welded
/// positions), each group's triangles in order, groups ordered by their first one.
fn connected_parts(tris: &[Tri]) -> Vec<Vec<usize>> {
    let mut index = HashMap::<[u64; 3], usize>::new();
    let mut parent: Vec<usize> = Vec::new();
    let corners: Vec<[usize; 3]> = tris
        .iter()
        .map(|(_, pts, _)| {
            pts.map(|p| {
                let next = parent.len();
                let v = *index.entry(p.to_array().map(f64::to_bits)).or_insert(next);
                if v == next {
                    parent.push(next);
                }
                v
            })
        })
        .collect();
    for c in &corners {
        for &v in &c[1..] {
            let (a, b) = (find(&mut parent, c[0]), find(&mut parent, v));
            if a != b {
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut parts: Vec<Vec<usize>> = Vec::new();
    let mut of_root = BTreeMap::<usize, usize>::new();
    for (t, c) in corners.iter().enumerate() {
        let root = find(&mut parent, c[0]);
        let k = *of_root.entry(root).or_insert_with(|| {
            parts.push(Vec::new());
            parts.len() - 1
        });
        parts[k].push(t);
    }
    parts
}

/// The `Geometry` and `Model` nodes of one mesh: welded positions, triangles with
/// per-corner normals and UVs, a material index (`local`) per triangle.
fn part_nodes(
    tris: &[Tri],
    part: &[usize],
    local: &BTreeMap<usize, i32>,
    name: &str,
    geom_id: i64,
    model_id: i64,
) -> (N, N) {
    let mut index = HashMap::<[u64; 3], i32>::new();
    let mut uv_at = HashMap::<[u64; 2], i32>::new();
    let (mut vertices, mut polys, mut normals, mut uvs, mut uv_index, mut mat_index) = (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    for &t in part {
        let (slot, pts, uv) = &tris[t];
        let normal = (pts[1] - pts[0]).cross(pts[2] - pts[0]).normalize_or_zero();
        for (k, (pos, uv)) in pts.iter().zip(uv).enumerate() {
            let next = (vertices.len() / 3) as i32;
            let vi = *index
                .entry(pos.to_array().map(f64::to_bits))
                .or_insert_with(|| {
                    vertices.extend_from_slice(&pos.to_array());
                    next
                });
            polys.push(if k == 2 { !vi } else { vi });
            normals.extend_from_slice(&normal.to_array());
            let next_uv = (uvs.len() / 2) as i32;
            let ui = *uv_at.entry(uv.map(f64::to_bits)).or_insert_with(|| {
                uvs.extend_from_slice(uv);
                next_uv
            });
            uv_index.push(ui);
        }
        mat_index.push(local.get(slot).copied().unwrap_or(0));
    }
    let layer_element = |ty: &str| {
        nc(
            "LayerElement",
            vec![],
            vec![n("Type", vec![s(ty)]), n("TypedIndex", vec![P::I32(0)])],
        )
    };
    let geometry = nc(
        "Geometry",
        vec![P::I64(geom_id), object_name(name, "Geometry"), s("Mesh")],
        vec![
            n("GeometryVersion", vec![P::I32(124)]),
            n("Vertices", vec![P::F64s(vertices)]),
            n("PolygonVertexIndex", vec![P::I32s(polys)]),
            nc(
                "LayerElementNormal",
                vec![P::I32(0)],
                vec![
                    n("Version", vec![P::I32(101)]),
                    n("Name", vec![s("")]),
                    n("MappingInformationType", vec![s("ByPolygonVertex")]),
                    n("ReferenceInformationType", vec![s("Direct")]),
                    n("Normals", vec![P::F64s(normals)]),
                ],
            ),
            nc(
                "LayerElementUV",
                vec![P::I32(0)],
                vec![
                    n("Version", vec![P::I32(101)]),
                    n("Name", vec![s("UVMap")]),
                    n("MappingInformationType", vec![s("ByPolygonVertex")]),
                    n("ReferenceInformationType", vec![s("IndexToDirect")]),
                    n("UV", vec![P::F64s(uvs)]),
                    n("UVIndex", vec![P::I32s(uv_index)]),
                ],
            ),
            nc(
                "LayerElementMaterial",
                vec![P::I32(0)],
                vec![
                    n("Version", vec![P::I32(101)]),
                    n("Name", vec![s("")]),
                    n("MappingInformationType", vec![s("ByPolygon")]),
                    n("ReferenceInformationType", vec![s("IndexToDirect")]),
                    n("Materials", vec![P::I32s(mat_index)]),
                ],
            ),
            nc(
                "Layer",
                vec![P::I32(0)],
                vec![
                    n("Version", vec![P::I32(100)]),
                    layer_element("LayerElementNormal"),
                    layer_element("LayerElementMaterial"),
                    layer_element("LayerElementUV"),
                ],
            ),
        ],
    );
    let model_node = nc(
        "Model",
        vec![P::I64(model_id), object_name(name, "Model"), s("Mesh")],
        vec![
            n("Version", vec![P::I32(232)]),
            nc(
                "Properties70",
                vec![],
                vec![
                    p_int("DefaultAttributeIndex", 0),
                    pp(
                        "Lcl Translation",
                        "Lcl Translation",
                        "",
                        "A",
                        vec![P::F64(0.0), P::F64(0.0), P::F64(0.0)],
                    ),
                ],
            ),
            n("Shading", vec![P::Bool(true)]),
            n("Culling", vec![s("CullingOff")]),
        ],
    );

    (geometry, model_node)
}

/// Encodes the model as binary FBX 7.4.
pub fn to_fbx(model: &Model, opts: &ExportOptions) -> Files {
    let (mats, groups) = groups(model, opts);

    // Triangles: material slot, corners, texture coordinates.
    let mut tris: Vec<Tri> = Vec::new();
    for (slot, g) in groups.iter().enumerate() {
        for t in &g.triangles {
            let pts = t.map(|(p, _)| g.positions[p]);
            let uv = t.map(|(_, uv)| g.uvs.get(uv).map_or([0.0; 2], |t| [t.x, t.y]));
            tris.push((slot, pts, uv));
        }
    }
    let parts = if opts.separate {
        connected_parts(&tris)
    } else {
        vec![(0..tris.len()).collect()]
    };

    let mut next_id = 1_000_000i64;
    let mut id = || {
        next_id += 1;
        next_id
    };
    let name = if opts.base_name.is_empty() {
        "model"
    } else {
        opts.base_name.as_str()
    };
    let mat_ids: Vec<i64> = mats.iter().map(|_| id()).collect();
    let mut objects = Vec::new();
    let mut links = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        let (model_id, geom_id) = (id(), id());
        let part_name = if parts.len() == 1 {
            name.to_owned()
        } else {
            format!("{name}_{}", k + 1)
        };
        // The part's materials, in slot order: the geometry's material indices
        // follow the order of the model's material connections.
        let used: BTreeSet<usize> = part.iter().map(|&t| tris[t].0).collect();
        let local: BTreeMap<usize, i32> = used
            .iter()
            .enumerate()
            .map(|(i, &slot)| (slot, i as i32))
            .collect();
        let (geometry, model_node) = part_nodes(&tris, part, &local, &part_name, geom_id, model_id);
        objects.push(geometry);
        objects.push(model_node);
        links.push(n("C", vec![s("OO"), P::I64(model_id), P::I64(0)]));
        links.push(n("C", vec![s("OO"), P::I64(geom_id), P::I64(model_id)]));
        for &slot in &used {
            links.push(n(
                "C",
                vec![s("OO"), P::I64(mat_ids[slot]), P::I64(model_id)],
            ));
        }
    }
    let mut textures = 0;
    for (m, &mid) in mats.iter().zip(&mat_ids) {
        let c = m.color.map(|x| f64::from(x) / 255.0);
        objects.push(nc(
            "Material",
            vec![P::I64(mid), object_name(&m.name, "Material"), s("")],
            vec![
                n("Version", vec![P::I32(102)]),
                n("ShadingModel", vec![s("lambert")]),
                n("MultiLayer", vec![P::I32(0)]),
                nc(
                    "Properties70",
                    vec![],
                    vec![
                        p_color("DiffuseColor", c),
                        pp("DiffuseFactor", "Number", "", "A", vec![P::F64(1.0)]),
                        p_color("AmbientColor", [0.0; 3]),
                        p_color("TransparentColor", [1.0; 3]),
                        pp(
                            "TransparencyFactor",
                            "Number",
                            "",
                            "A",
                            vec![P::F64(1.0 - m.opacity)],
                        ),
                        p_double("Opacity", m.opacity),
                    ],
                ),
            ],
        ));
        if let Some((file, _)) = &m.texture {
            textures += 1;
            let (tid, vid) = (id(), id());
            objects.push(nc(
                "Texture",
                vec![P::I64(tid), object_name(&m.name, "Texture"), s("")],
                vec![
                    n("Type", vec![s("TextureVideoClip")]),
                    n("Version", vec![P::I32(202)]),
                    n("TextureName", vec![object_name(&m.name, "Texture")]),
                    nc(
                        "Properties70",
                        vec![],
                        vec![pp("UseMaterial", "bool", "", "", vec![P::I32(1)])],
                    ),
                    n("Media", vec![object_name(&m.name, "Video")]),
                    n("FileName", vec![s(file)]),
                    n("RelativeFilename", vec![s(file)]),
                    n("ModelUVTranslation", vec![P::F64(0.0), P::F64(0.0)]),
                    n("ModelUVScaling", vec![P::F64(1.0), P::F64(1.0)]),
                    n("Texture_Alpha_Source", vec![s("None")]),
                    n("Cropping", vec![P::I32(0), P::I32(0), P::I32(0), P::I32(0)]),
                ],
            ));
            objects.push(nc(
                "Video",
                vec![P::I64(vid), object_name(&m.name, "Video"), s("Clip")],
                vec![
                    n("Type", vec![s("Clip")]),
                    nc(
                        "Properties70",
                        vec![],
                        vec![pp("Path", "KString", "XRefUrl", "", vec![s(file)])],
                    ),
                    n("UseMipMap", vec![P::I32(0)]),
                    n("Filename", vec![s(file)]),
                    n("RelativeFilename", vec![s(file)]),
                ],
            ));
            links.push(n(
                "C",
                vec![s("OP"), P::I64(tid), P::I64(mid), s("DiffuseColor")],
            ));
            links.push(n("C", vec![s("OO"), P::I64(vid), P::I64(tid)]));
        }
    }

    let (up, front, front_sign) = if opts.y_up { (1, 2, 1) } else { (2, 1, -1) };
    let unit_cm = 2.54 / opts.units.per_inch();
    let global = nc(
        "GlobalSettings",
        vec![],
        vec![
            n("Version", vec![P::I32(1000)]),
            nc(
                "Properties70",
                vec![],
                vec![
                    p_int("UpAxis", up),
                    p_int("UpAxisSign", 1),
                    p_int("FrontAxis", front),
                    p_int("FrontAxisSign", front_sign),
                    p_int("CoordAxis", 0),
                    p_int("CoordAxisSign", 1),
                    p_int("OriginalUpAxis", up),
                    p_int("OriginalUpAxisSign", 1),
                    p_double("UnitScaleFactor", unit_cm),
                    p_double("OriginalUnitScaleFactor", unit_cm),
                    pp(
                        "AmbientColor",
                        "ColorRGB",
                        "Color",
                        "",
                        vec![P::F64(0.0), P::F64(0.0), P::F64(0.0)],
                    ),
                    pp(
                        "DefaultCamera",
                        "KString",
                        "",
                        "",
                        vec![s("Producer Perspective")],
                    ),
                    pp("TimeMode", "enum", "", "", vec![P::I32(11)]),
                    pp("TimeSpanStart", "KTime", "Time", "", vec![P::I64(0)]),
                    pp(
                        "TimeSpanStop",
                        "KTime",
                        "Time",
                        "",
                        vec![P::I64(46_186_158_000)],
                    ),
                    p_double("CustomFrameRate", -1.0),
                ],
            ),
        ],
    );
    let count = |ty: &str, k: i32| nc("ObjectType", vec![s(ty)], vec![n("Count", vec![P::I32(k)])]);
    let n_mats = mats.len() as i32;
    let n_parts = parts.len() as i32;
    let definitions = nc(
        "Definitions",
        vec![],
        vec![
            n("Version", vec![P::I32(100)]),
            n(
                "Count",
                vec![P::I32(1 + 2 * n_parts + n_mats + 2 * textures)],
            ),
            count("GlobalSettings", 1),
            count("Model", n_parts),
            count("Geometry", n_parts),
            count("Material", n_mats),
            count("Texture", textures),
            count("Video", textures),
        ],
    );
    let header = nc(
        "FBXHeaderExtension",
        vec![],
        vec![
            n("FBXHeaderVersion", vec![P::I32(1003)]),
            n("FBXVersion", vec![P::I32(7400)]),
            n("EncryptionType", vec![P::I32(0)]),
            nc(
                "CreationTimeStamp",
                vec![],
                vec![
                    n("Version", vec![P::I32(1000)]),
                    n("Year", vec![P::I32(1970)]),
                    n("Month", vec![P::I32(1)]),
                    n("Day", vec![P::I32(1)]),
                    n("Hour", vec![P::I32(10)]),
                    n("Minute", vec![P::I32(0)]),
                    n("Second", vec![P::I32(0)]),
                    n("Millisecond", vec![P::I32(0)]),
                ],
            ),
            n("Creator", vec![s("meshtard")]),
        ],
    );
    let doc_id = id();
    let documents = nc(
        "Documents",
        vec![],
        vec![
            n("Count", vec![P::I32(1)]),
            nc(
                "Document",
                vec![P::I64(doc_id), s(""), s("Scene")],
                vec![
                    nc(
                        "Properties70",
                        vec![],
                        vec![
                            pp("SourceObject", "object", "", "", vec![]),
                            pp("ActiveAnimStackName", "KString", "", "", vec![s("")]),
                        ],
                    ),
                    n("RootNode", vec![P::I64(0)]),
                ],
            ),
        ],
    );
    let top = vec![
        header,
        // The FBX SDK checks these against each other; Blender writes fixed values.
        n(
            "FileId",
            vec![P::Raw(
                b"\x28\xb3\x2a\xeb\xb6\x24\xcc\xc2\xbf\xc8\xb0\x2a\xa9\x2b\xfc\xf1".to_vec(),
            )],
        ),
        n("CreationTime", vec![s("1970-01-01 10:00:00:000")]),
        n("Creator", vec![s("meshtard")]),
        global,
        documents,
        n("References", vec![]),
        definitions,
        nc("Objects", vec![], objects),
        nc("Connections", vec![], links),
        nc("Takes", vec![], vec![n("Current", vec![s("")])]),
    ];

    let mut out = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    out.extend_from_slice(&7400u32.to_le_bytes());
    write_children(&mut out, &top, true);
    // Footer, as Blender writes it.
    out.extend_from_slice(b"\xfa\xbc\xab\x09\xd0\xc8\xd4\x66\xb1\x76\xfb\x83\x1c\xf7\x26\x7e");
    out.extend_from_slice(&[0; 4]);
    let pad = match (16 - out.len() % 16) % 16 {
        0 => 16,
        p => p,
    };
    out.extend(std::iter::repeat_n(0u8, pad));
    out.extend_from_slice(&7400u32.to_le_bytes());
    out.extend_from_slice(&[0; 120]);
    out.extend_from_slice(b"\xf8\x5a\x8c\x6a\xde\xf5\xd9\x7e\xec\xe9\x0c\xe3\x75\x8f\x29\x0b");
    Files {
        main: out,
        others: texture_files(&mats),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::fbx;
    use crate::formats::tests::{painted_cube, summary};
    use crate::mesh_export::Units;

    #[test]
    fn separate_disconnected_faces() {
        let mut m = painted_cube();
        // A loose square away from the cube.
        let p = |x: f64, y: f64| DVec3::new(x, y, 0.0);
        let sq = [p(50., 0.), p(60., 0.), p(60., 10.), p(50., 10.)];
        for i in 0..4 {
            crate::add_line(&mut m.root, sq[i], sq[(i + 1) % 4]);
        }
        let count = |bytes: &[u8], name: &str| {
            let (_, nodes) = fbx::parse_tree(bytes).unwrap();
            let objects = nodes.iter().find(|n| n.name == "Objects").unwrap();
            objects.children_named(name).count()
        };
        let none = |_: &str| None;
        for separate in [false, true] {
            let opts = ExportOptions {
                separate,
                ..Default::default()
            };
            let files = to_fbx(&m, &opts);
            let parts = if separate { 2 } else { 1 };
            assert_eq!(count(&files.main, "Model"), parts);
            assert_eq!(count(&files.main, "Geometry"), parts);
            let (mesh, _) = fbx::parse(&files.main, &none).unwrap();
            assert_eq!(mesh.polygons.len(), 14, "separate {separate}");
            let red = mesh.materials.iter().position(|x| x.name == "Red").unwrap();
            let on_red = mesh
                .polygons
                .iter()
                .filter(|p| p.material == Some(red))
                .count();
            assert_eq!(on_red, 2, "separate {separate}");
            // Deterministic output.
            assert_eq!(files.main, to_fbx(&m, &opts).main);
        }
    }

    #[test]
    fn binary_round_trip() {
        let m = painted_cube();
        for y_up in [false, true] {
            let files = to_fbx(
                &m,
                &ExportOptions {
                    units: Units::Millimeters,
                    y_up,
                    ..Default::default()
                },
            );
            let (v, nodes) = fbx::parse_tree(&files.main).unwrap();
            assert_eq!(v, 7400);
            assert!(nodes.iter().any(|n| n.name == "Connections"));
            let none = |_: &str| None;
            let (mesh, hints) = fbx::parse(&files.main, &none).unwrap();
            assert_eq!(hints.units, Some(Units::Centimeters));
            assert_eq!(mesh.polygons.len(), 12);
            assert_eq!(mesh.materials.len(), 2);
            let red = mesh.materials.iter().position(|x| x.name == "Red").unwrap();
            assert_eq!(mesh.materials[red].color, [255, 0, 0]);
            // Per-polygon materials: the top's two triangles are red.
            let on_red = mesh
                .polygons
                .iter()
                .filter(|p| p.material == Some(red))
                .count();
            assert_eq!(on_red, 2);
            // Millimeters in the file, read back as centimeters.
            let (faces, verts, max) = summary(&mesh, 1.0 / 2.54, false);
            assert_eq!((faces, verts), (6, 8), "y_up {y_up}");
            assert!((max - DVec3::splat(10.0)).length() < 1e-6, "{max}");
        }
    }
}
