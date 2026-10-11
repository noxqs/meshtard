//! 3MF (3D Manufacturing Format) import and export, after the 3MF Core
//! Specification 1.3 and the Materials and Properties Extension 1.2
//! (<https://github.com/3MFConsortium/spec_core>, BSD-2-Clause). A 3MF file is a ZIP
//! (OPC) package with an XML model part.
//!
//! Import:
//! - The root model part named by `_rels/.rels` (else `3D/3dmodel.model`); build
//!   items with their transforms, components (nested, also in other parts through the
//!   Production Extension's `p:path`), meshes.
//! - Properties: base materials (name and display color), color groups (the
//!   triangle's first color) and 2D texture groups (PNG/JPEG textures with texture
//!   coordinates); a triangle's own `pid`/`p1` or else its object's.
//! - Units from the model's `unit` attribute (millimeter by default); 3MF is Z up.
//! - Beam lattices, slices and other extensions are ignored.
//!
//! Export: one mesh object (welded vertices) in the chosen units, a base material per
//! model material (display color with alpha for translucent ones), textured
//! materials as 2D texture groups with their images in `3D/Textures/`.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{Cursor, Read, Write};

use glam::{DMat4, DVec2, DVec3, DVec4};

use super::xml::{Event, events, local};
use super::{ExportOptions, Hints, groups, num, texture_from_bytes, xml_escape};
use crate::mesh_export::Units;
use crate::mesh_import::{Mesh, MeshMaterial, Polygon};
use crate::model::Model;

/// A property resource of a model part.
#[derive(Debug, Clone)]
enum Resource {
    Base(Vec<(String, [u8; 4])>),
    Colors(Vec<[u8; 4]>),
    /// Texture id, coordinates.
    TexCoords(String, Vec<DVec2>),
    /// Path of an image in the package.
    Texture(String),
}

/// A triangle's corners and properties (`pid`, `p1`, `p2`, `p3`).
type Triangle = ([usize; 3], Option<String>, [Option<usize>; 3]);

#[derive(Debug, Clone, Default)]
struct Object {
    vertices: Vec<DVec3>,
    triangles: Vec<Triangle>,
    pid: Option<String>,
    pindex: Option<usize>,
    /// Referenced object id, part path, transform.
    components: Vec<(String, Option<String>, DMat4)>,
}

#[derive(Debug, Clone, Default)]
struct Part {
    unit: Option<String>,
    objects: HashMap<String, Object>,
    resources: HashMap<String, Resource>,
    items: Vec<(String, Option<String>, DMat4)>,
}

fn color(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// A 3MF transform: 12 numbers, row vectors (`p' = p M`).
fn transform(s: Option<&str>) -> DMat4 {
    let Some(s) = s else {
        return DMat4::IDENTITY;
    };
    let m: Vec<f64> = s
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    if m.len() != 12 {
        return DMat4::IDENTITY;
    }
    DMat4::from_cols(
        DVec4::new(m[0], m[1], m[2], 0.0),
        DVec4::new(m[3], m[4], m[5], 0.0),
        DVec4::new(m[6], m[7], m[8], 0.0),
        DVec4::new(m[9], m[10], m[11], 1.0),
    )
}

fn parse_part(text: &str) -> Result<Part, String> {
    let mut part = Part::default();
    let mut object: Option<(String, Object)> = None;
    let mut resource: Option<(String, Resource)> = None;
    let num = |e: &Event, k: &str| -> Result<f64, String> {
        e.attr(k)
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| format!("3MF: bad or missing {k}"))
    };
    let index = |e: &Event, k: &str| e.attr(k).and_then(|v| v.trim().parse::<usize>().ok());
    for e in events(text)? {
        match &e {
            Event::Start { name, .. } => match local(name) {
                "model" => part.unit = e.attr("unit").map(str::to_owned),
                "object" => {
                    object = Some((
                        e.attr("id").unwrap_or_default().to_owned(),
                        Object {
                            pid: e.attr("pid").map(str::to_owned),
                            pindex: index(&e, "pindex"),
                            ..Default::default()
                        },
                    ));
                }
                "vertex" => {
                    if let Some((_, o)) = &mut object {
                        o.vertices
                            .push(DVec3::new(num(&e, "x")?, num(&e, "y")?, num(&e, "z")?));
                    }
                }
                "triangle" => {
                    if let Some((_, o)) = &mut object {
                        let v = [index(&e, "v1"), index(&e, "v2"), index(&e, "v3")];
                        let [Some(a), Some(b), Some(c)] = v else {
                            return Err("3MF: bad triangle".into());
                        };
                        o.triangles.push((
                            [a, b, c],
                            e.attr("pid").map(str::to_owned),
                            [index(&e, "p1"), index(&e, "p2"), index(&e, "p3")],
                        ));
                    }
                }
                "component" => {
                    if let Some((_, o)) = &mut object {
                        o.components.push((
                            e.attr("objectid").unwrap_or_default().to_owned(),
                            e.attr("path").map(str::to_owned),
                            transform(e.attr("transform")),
                        ));
                    }
                }
                "item" => part.items.push((
                    e.attr("objectid").unwrap_or_default().to_owned(),
                    e.attr("path").map(str::to_owned),
                    transform(e.attr("transform")),
                )),
                "basematerials" => {
                    resource = Some((
                        e.attr("id").unwrap_or_default().to_owned(),
                        Resource::Base(Vec::new()),
                    ))
                }
                "base" => {
                    if let Some((_, Resource::Base(list))) = &mut resource {
                        list.push((
                            e.attr("name").unwrap_or_default().to_owned(),
                            e.attr("displaycolor").and_then(color).unwrap_or([255; 4]),
                        ));
                    }
                }
                "colorgroup" => {
                    resource = Some((
                        e.attr("id").unwrap_or_default().to_owned(),
                        Resource::Colors(Vec::new()),
                    ))
                }
                "color" => {
                    if let Some((_, Resource::Colors(list))) = &mut resource {
                        list.push(e.attr("color").and_then(color).unwrap_or([255; 4]));
                    }
                }
                "texture2dgroup" => {
                    resource = Some((
                        e.attr("id").unwrap_or_default().to_owned(),
                        Resource::TexCoords(
                            e.attr("texid").unwrap_or_default().to_owned(),
                            Vec::new(),
                        ),
                    ))
                }
                "tex2coord" => {
                    if let Some((_, Resource::TexCoords(_, list))) = &mut resource {
                        list.push(DVec2::new(num(&e, "u")?, num(&e, "v")?));
                    }
                }
                "texture2d" => {
                    part.resources.insert(
                        e.attr("id").unwrap_or_default().to_owned(),
                        Resource::Texture(e.attr("path").unwrap_or_default().to_owned()),
                    );
                }
                _ => {}
            },
            Event::End(name) => match local(name) {
                "object" => {
                    if let Some((id, o)) = object.take() {
                        part.objects.insert(id, o);
                    }
                }
                "basematerials" | "colorgroup" | "texture2dgroup" => {
                    if let Some((id, r)) = resource.take() {
                        part.resources.insert(id, r);
                    }
                }
                _ => {}
            },
        }
    }
    Ok(part)
}

/// What a triangle's properties resolve to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum MatKey {
    Base(String, String, usize),
    Color([u8; 4]),
    Texture(String, String),
}

struct Reader<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    parts: HashMap<String, Part>,
    mesh: Mesh,
    materials: BTreeMap<MatKey, usize>,
}

fn norm_path(p: &str) -> String {
    p.trim_start_matches('/').to_owned()
}

impl Reader<'_> {
    fn file(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut f = self.zip.by_name(&norm_path(path)).ok()?;
        let mut out = Vec::new();
        f.read_to_end(&mut out).ok()?;
        Some(out)
    }

    fn part(&mut self, path: &str) -> Result<(), String> {
        if self.parts.contains_key(path) {
            return Ok(());
        }
        let bytes = self
            .file(path)
            .ok_or_else(|| format!("3MF: missing part {path}"))?;
        let part = parse_part(&String::from_utf8_lossy(&bytes))?;
        self.parts.insert(path.to_owned(), part);
        Ok(())
    }

    fn material(&mut self, path: &str, key: MatKey) -> usize {
        if let Some(&m) = self.materials.get(&key) {
            return m;
        }
        let mat = match &key {
            MatKey::Base(_, id, i) => {
                let base = match self.parts[path].resources.get(id) {
                    Some(Resource::Base(list)) => list.get(*i).cloned(),
                    _ => None,
                };
                let (name, c) = base.unwrap_or_else(|| ("Material".into(), [255; 4]));
                MeshMaterial {
                    name: if name.is_empty() {
                        "Material".into()
                    } else {
                        name
                    },
                    color: [c[0], c[1], c[2]],
                    opacity: f64::from(c[3]) / 255.0,
                    texture: None,
                }
            }
            MatKey::Color(c) => super::color_material([c[0], c[1], c[2]], f64::from(c[3]) / 255.0),
            MatKey::Texture(_, tex) => {
                let file = match self.parts[path].resources.get(tex) {
                    Some(Resource::Texture(f)) => f.clone(),
                    _ => String::new(),
                };
                let texture = self.file(&file).and_then(|d| texture_from_bytes(&file, d));
                let name = super::base_name(&file);
                MeshMaterial {
                    name: name.rsplit_once('.').map_or(name, |x| x.0).to_owned(),
                    color: [255; 3],
                    opacity: 1.0,
                    texture,
                }
            }
        };
        self.mesh.materials.push(mat);
        self.materials.insert(key, self.mesh.materials.len() - 1);
        self.mesh.materials.len() - 1
    }

    /// Adds object `id` of part `path` placed by `xf`.
    fn object(&mut self, path: &str, id: &str, xf: DMat4, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err("3MF: components nest too deep".into());
        }
        self.part(path)?;
        let Some(obj) = self.parts[path].objects.get(id).cloned() else {
            return Err(format!("3MF: missing object {id}"));
        };
        let base = self.mesh.positions.len();
        self.mesh
            .positions
            .extend(obj.vertices.iter().map(|&p| xf.transform_point3(p)));
        for (v, pid, p) in &obj.triangles {
            if v.iter().any(|&i| i >= obj.vertices.len()) {
                return Err("3MF: vertex index out of range".into());
            }
            let pid = pid.clone().or_else(|| obj.pid.clone());
            let p1 = p[0].or(obj.pindex);
            let mut uvs = Vec::new();
            let key = match (pid.as_ref(), p1) {
                (Some(pid), Some(p1)) => match self.parts[path].resources.get(pid) {
                    Some(Resource::Base(_)) => Some(MatKey::Base(path.to_owned(), pid.clone(), p1)),
                    Some(Resource::Colors(list)) => list.get(p1).map(|&c| MatKey::Color(c)),
                    Some(Resource::TexCoords(tex, list)) => {
                        let at = |k: Option<usize>| list.get(k.unwrap_or(p1)).copied();
                        if let (Some(a), Some(b), Some(c)) = (at(p[0]), at(p[1]), at(p[2])) {
                            uvs = vec![a, b, c];
                        }
                        Some(MatKey::Texture(path.to_owned(), tex.clone()))
                    }
                    _ => None,
                },
                _ => None,
            };
            let material = key.map(|k| self.material(path, k));
            self.mesh.polygons.push(Polygon {
                vertices: v.map(|i| base + i).to_vec(),
                material,
                uvs,
            });
        }
        for (child, child_path, t) in &obj.components {
            let p = child_path.as_deref().map_or(path.to_owned(), norm_path);
            self.object(&p, child, xf * *t, depth + 1)?;
        }
        Ok(())
    }
}

/// The unit named by a 3MF `unit` attribute (microns are not a [`Units`]; they read
/// as millimeters scaled by the caller).
fn units(unit: Option<&str>) -> (Units, f64) {
    match unit.unwrap_or("millimeter") {
        "micron" => (Units::Millimeters, 0.001),
        "centimeter" => (Units::Centimeters, 1.0),
        "inch" => (Units::Inches, 1.0),
        "foot" => (Units::Feet, 1.0),
        "meter" => (Units::Meters, 1.0),
        _ => (Units::Millimeters, 1.0),
    }
}

/// Reads a 3MF package.
pub fn parse(bytes: &[u8]) -> Result<(Mesh, Hints), String> {
    let zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a 3MF file: {e}"))?;
    let mut r = Reader {
        zip,
        parts: HashMap::new(),
        mesh: Mesh::default(),
        materials: BTreeMap::new(),
    };
    let root = r
        .file("_rels/.rels")
        .and_then(|rels| {
            let text = String::from_utf8_lossy(&rels).into_owned();
            events(&text).ok()?.iter().find_map(|e| {
                (e.attr("Type")?.ends_with("/3dmodel")).then(|| e.attr("Target").map(norm_path))?
            })
        })
        .unwrap_or_else(|| "3D/3dmodel.model".into());
    r.part(&root)?;
    let items = r.parts[&root].items.clone();
    for (id, path, xf) in items {
        let p = path.as_deref().map_or(root.clone(), norm_path);
        r.object(&p, &id, xf, 0)?;
    }
    let (u, scale) = units(r.parts[&root].unit.as_deref());
    if scale != 1.0 {
        for p in &mut r.mesh.positions {
            *p *= scale;
        }
    }
    Ok((
        r.mesh,
        Hints {
            units: Some(u),
            y_up: Some(false),
        },
    ))
}

fn unit_name(u: Units) -> &'static str {
    match u {
        Units::Inches => "inch",
        Units::Feet => "foot",
        Units::Millimeters => "millimeter",
        Units::Centimeters => "centimeter",
        Units::Meters => "meter",
    }
}

const CORE_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const MATERIAL_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/material/2015/02";

/// Encodes the model as a 3MF package (Z up, in `opts.units`).
pub fn to_3mf(model: &Model, opts: &ExportOptions) -> Result<Vec<u8>, String> {
    let opts = ExportOptions {
        y_up: false,
        ..opts.clone()
    };
    let (mats, groups) = groups(model, &opts);
    // One welded vertex list for the whole object.
    let mut index: HashMap<[u64; 3], usize> = HashMap::new();
    let mut vertices = String::new();
    let mut tris = String::new();
    let mut count = 0;
    let mut texture_parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut resources = String::new();
    let _ = writeln!(resources, "    <basematerials id=\"1\">");
    for m in &mats {
        let alpha = super::unit_byte(m.opacity);
        let _ = writeln!(
            resources,
            "      <base name=\"{}\" displaycolor=\"#{:02X}{:02X}{:02X}{}\"/>",
            xml_escape(&m.name),
            m.color[0],
            m.color[1],
            m.color[2],
            if alpha < 255 {
                format!("{alpha:02X}")
            } else {
                String::new()
            }
        );
    }
    let _ = writeln!(resources, "    </basematerials>");
    let mut next_id = 2;
    for (slot, (g, m)) in groups.iter().zip(&mats).enumerate() {
        let tex_group = m.texture.as_ref().map(|(file, data)| {
            let path = format!("/3D/Textures/{file}");
            texture_parts.push((path.clone(), data.to_vec()));
            let (tex, grp) = (next_id, next_id + 1);
            next_id += 2;
            let ct = if super::super::mesh_export::image_extension(data) == "jpg" {
                "image/jpeg"
            } else {
                "image/png"
            };
            let _ = writeln!(
                resources,
                "    <m:texture2d id=\"{tex}\" path=\"{}\" contenttype=\"{ct}\"/>",
                xml_escape(&path)
            );
            let _ = writeln!(
                resources,
                "    <m:texture2dgroup id=\"{grp}\" texid=\"{tex}\">"
            );
            for t in &g.uvs {
                let _ = writeln!(
                    resources,
                    "      <m:tex2coord u=\"{}\" v=\"{}\"/>",
                    num(t.x),
                    num(t.y)
                );
            }
            let _ = writeln!(resources, "    </m:texture2dgroup>");
            grp
        });
        for t in &g.triangles {
            let v = t.map(|(p, _)| {
                let pos = g.positions[p];
                *index
                    .entry(pos.to_array().map(f64::to_bits))
                    .or_insert_with(|| {
                        let _ = writeln!(
                            vertices,
                            "          <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
                            num(pos.x),
                            num(pos.y),
                            num(pos.z)
                        );
                        count += 1;
                        count - 1
                    })
            });
            match tex_group {
                Some(grp) => {
                    let _ = writeln!(
                        tris,
                        "          <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"{grp}\" p1=\"{}\" p2=\"{}\" p3=\"{}\"/>",
                        v[0], v[1], v[2], t[0].1, t[1].1, t[2].1
                    );
                }
                None => {
                    let _ = writeln!(
                        tris,
                        "          <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"1\" p1=\"{slot}\"/>",
                        v[0], v[1], v[2]
                    );
                }
            }
        }
    }
    let object_id = next_id;
    let mut doc = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        doc,
        "<model unit=\"{}\" xml:lang=\"en-US\" xmlns=\"{CORE_NS}\" xmlns:m=\"{MATERIAL_NS}\">",
        unit_name(opts.units)
    );
    doc.push_str("  <metadata name=\"Application\">meshtard</metadata>\n  <resources>\n");
    doc.push_str(&resources);
    let _ = writeln!(
        doc,
        "    <object id=\"{object_id}\" type=\"model\" name=\"{}\">\n      <mesh>\n        <vertices>",
        xml_escape(&opts.base_name)
    );
    doc.push_str(&vertices);
    doc.push_str("        </vertices>\n        <triangles>\n");
    doc.push_str(&tris);
    doc.push_str("        </triangles>\n      </mesh>\n    </object>\n  </resources>\n");
    let _ = writeln!(
        doc,
        "  <build>\n    <item objectid=\"{object_id}\"/>\n  </build>\n</model>"
    );

    let types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n  <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n  <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n  <Default Extension=\"png\" ContentType=\"image/png\"/>\n  <Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\n</Types>\n";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n  <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n</Relationships>\n";
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), types.as_bytes().to_vec()),
        ("_rels/.rels".into(), rels.as_bytes().to_vec()),
        ("3D/3dmodel.model".into(), doc.into_bytes()),
    ];
    if !texture_parts.is_empty() {
        let mut r = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
        );
        for (i, (path, _)) in texture_parts.iter().enumerate() {
            let _ = writeln!(
                r,
                "  <Relationship Target=\"{}\" Id=\"tex{i}\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dtexture\"/>",
                xml_escape(path)
            );
        }
        r.push_str("</Relationships>\n");
        files.push(("3D/_rels/3dmodel.model.rels".into(), r.into_bytes()));
        for (path, data) in texture_parts {
            files.push((norm_path(&path), data));
        }
    }
    write_zip(&files)
}

/// A ZIP archive of `files` (deflated), in order.
pub(crate) fn write_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut seen = std::collections::HashSet::new();
    for (name, data) in files {
        if !seen.insert(name.clone()) {
            continue;
        }
        zip.start_file(name.as_str(), options)
            .map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::tests::{painted_cube, summary};

    #[test]
    fn round_trip_keeps_shape_and_materials() {
        let m = painted_cube();
        let bytes = to_3mf(
            &m,
            &ExportOptions {
                units: Units::Millimeters,
                ..Default::default()
            },
        )
        .unwrap();
        let (mesh, hints) = parse(&bytes).unwrap();
        assert_eq!(hints.units, Some(Units::Millimeters));
        assert_eq!(mesh.positions.len(), 8);
        assert_eq!(mesh.polygons.len(), 12);
        assert_eq!(mesh.materials.len(), 2);
        assert!(
            mesh.materials
                .iter()
                .any(|x| x.name == "Red" && x.color == [255, 0, 0])
        );
        let (faces, verts, max) = summary(&mesh, 1.0 / 25.4, false);
        assert_eq!((faces, verts), (6, 8));
        assert!((max - DVec3::splat(10.0)).length() < 1e-3);
    }

    #[test]
    fn components_transforms_and_colors() {
        let model = r##"<?xml version="1.0"?>
<model unit="inch" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:m="http://schemas.microsoft.com/3dmanufacturing/material/2015/02">
<resources>
  <m:colorgroup id="5"><m:color color="#00FF00"/><m:color color="#0000FF80"/></m:colorgroup>
  <object id="1" type="model" pid="5" pindex="0"><mesh><vertices>
    <vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/>
  </vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="2" v3="1" p1="1"/></triangles></mesh></object>
  <object id="2" type="model"><components>
    <component objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 0"/>
    <component objectid="1"/>
  </components></object>
</resources>
<build><item objectid="2" transform="2 0 0 0 2 0 0 0 2 0 0 5"/></build>
</model>"##;
        let bytes = write_zip(&[("3D/3dmodel.model".into(), model.as_bytes().to_vec())]).unwrap();
        let (mesh, hints) = parse(&bytes).unwrap();
        assert_eq!(hints.units, Some(Units::Inches));
        assert_eq!(mesh.polygons.len(), 4);
        // (1,0,0) moved by 10 then scaled 2 and lifted 5.
        assert!(mesh.positions.contains(&DVec3::new(22.0, 0.0, 5.0)));
        assert_eq!(mesh.materials.len(), 2);
        assert_eq!(mesh.materials[1].color, [0, 0, 255]);
        assert!((mesh.materials[1].opacity - 128.0 / 255.0).abs() < 1e-9);
        assert!(parse(b"not a zip").is_err());
    }
}
