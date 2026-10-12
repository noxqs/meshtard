//! PLY (Stanford polygon file) import and export, written from Paul Bourke's format
//! description (<https://paulbourke.net/dataformats/ply/>).
//!
//! Import:
//! - ASCII, binary little endian and binary big endian; every scalar type name of the
//!   format (`char`..`double` and `int8`..`float64`). Elements other than `vertex` and
//!   `face` are skipped.
//! - `x y z` positions; faces from the `vertex_indices` (or `vertex_index`) list.
//! - Colors (optional): a face's own `red green blue [alpha]` or else the average of
//!   its vertices' colors becomes a material `Color_RRGGBB`; more than 256 colors are
//!   rounded to 4 bits per channel first. Float colors (0..1) are read too.
//! - Texture coordinates `s t` / `u v` / `texture_u texture_v` per vertex, or a face
//!   `texcoord` list, with the texture named by a `comment TextureFile <name>` line
//!   (MeshLab's convention; with several, a face `texnumber` picks one): faces get
//!   the texture.
//! - PLY records no units or axes.
//!
//! Export: binary little endian or ASCII; per-material vertices (so colors are exact)
//! with `red green blue` bytes, triangles as `vertex_indices` lists. With textures,
//! MeshLab's convention: `comment TextureFile` lines naming the images written next
//! to the file, a face `texcoord` list (empty for untextured faces) and, with several
//! textures, a face `texnumber`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glam::{DVec2, DVec3};

use super::{ExportOptions, Files, Hints, ReadFile, find_texture, groups};
use crate::mesh_import::{Mesh, MeshMaterial, MeshTexture, Polygon};
use crate::model::Model;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "char" | "int8" => Self::I8,
            "uchar" | "uint8" => Self::U8,
            "short" | "int16" => Self::I16,
            "ushort" | "uint16" => Self::U16,
            "int" | "int32" => Self::I32,
            "uint" | "uint32" => Self::U32,
            "float" | "float32" => Self::F32,
            "double" | "float64" => Self::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }

    fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
}

#[derive(Debug, Clone)]
struct Property {
    name: String,
    ty: Scalar,
    /// The count type of a list property.
    list: Option<Scalar>,
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    count: usize,
    props: Vec<Property>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Encoding {
    Ascii,
    Little,
    Big,
}

/// Reads values one at a time from the body.
struct Body<'a> {
    enc: Encoding,
    bytes: &'a [u8],
    pos: usize,
    words: std::str::SplitAsciiWhitespace<'a>,
}

impl Body<'_> {
    fn read(&mut self, ty: Scalar) -> Result<f64, String> {
        if self.enc == Encoding::Ascii {
            let w = self.words.next().ok_or("PLY data ends early")?;
            return w.parse().map_err(|_| format!("bad PLY number {w:?}"));
        }
        let n = ty.size();
        let b = self
            .bytes
            .get(self.pos..self.pos + n)
            .ok_or("PLY data ends early")?;
        self.pos += n;
        let mut a = [0u8; 8];
        a[..n].copy_from_slice(b);
        if self.enc == Encoding::Big {
            a[..n].reverse();
        }
        Ok(match ty {
            Scalar::I8 => f64::from(a[0] as i8),
            Scalar::U8 => f64::from(a[0]),
            Scalar::I16 => f64::from(i16::from_le_bytes([a[0], a[1]])),
            Scalar::U16 => f64::from(u16::from_le_bytes([a[0], a[1]])),
            Scalar::I32 => f64::from(i32::from_le_bytes([a[0], a[1], a[2], a[3]])),
            Scalar::U32 => f64::from(u32::from_le_bytes([a[0], a[1], a[2], a[3]])),
            Scalar::F32 => f64::from(f32::from_le_bytes([a[0], a[1], a[2], a[3]])),
            Scalar::F64 => f64::from_le_bytes(a),
        })
    }
}

/// One element instance: scalar values by property, lists by property.
type Record = (Vec<f64>, Vec<Vec<f64>>);

fn read_record(body: &mut Body, el: &Element) -> Result<Record, String> {
    let mut scalars = vec![0.0; el.props.len()];
    let mut lists = vec![Vec::new(); el.props.len()];
    for (k, p) in el.props.iter().enumerate() {
        match p.list {
            Some(ct) => {
                let n = body.read(ct)?;
                if !(0.0..=1e6).contains(&n) {
                    return Err("bad PLY list length".into());
                }
                lists[k] = (0..n as usize)
                    .map(|_| body.read(p.ty))
                    .collect::<Result<_, _>>()?;
            }
            None => scalars[k] = body.read(p.ty)?,
        }
    }
    Ok((scalars, lists))
}

/// Reads a PLY file. With `colors`, vertex or face colors become materials. `read`
/// finds a texture named by a `TextureFile` comment.
pub fn parse(bytes: &[u8], colors: bool, read: ReadFile) -> Result<(Mesh, Hints), String> {
    let end = find(bytes, b"end_header").ok_or("not a PLY file")?;
    let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "bad PLY header")?;
    if !header.starts_with("ply") {
        return Err("not a PLY file".into());
    }
    let mut body_start = end + b"end_header".len();
    // The header ends with one line break (LF or CR LF).
    if bytes.get(body_start) == Some(&b'\r') {
        body_start += 1;
    }
    if bytes.get(body_start) == Some(&b'\n') {
        body_start += 1;
    }
    let mut enc = None;
    let mut elements: Vec<Element> = Vec::new();
    let mut texture_files: Vec<String> = Vec::new();
    for line in header.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["format", f, ..] => {
                enc = Some(match *f {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::Little,
                    "binary_big_endian" => Encoding::Big,
                    _ => return Err(format!("unknown PLY format {f}")),
                })
            }
            ["comment", key, rest @ ..] if key.eq_ignore_ascii_case("texturefile") => {
                texture_files.push(rest.join(" "));
            }
            ["element", name, count] => elements.push(Element {
                name: (*name).to_owned(),
                count: count.parse().map_err(|_| "bad PLY element count")?,
                props: Vec::new(),
            }),
            ["property", "list", ct, ty, name] => {
                let el = elements.last_mut().ok_or("PLY property before element")?;
                el.props.push(Property {
                    name: (*name).to_owned(),
                    ty: Scalar::parse(ty).ok_or("bad PLY type")?,
                    list: Some(Scalar::parse(ct).ok_or("bad PLY type")?),
                });
            }
            ["property", ty, name] => {
                let el = elements.last_mut().ok_or("PLY property before element")?;
                el.props.push(Property {
                    name: (*name).to_owned(),
                    ty: Scalar::parse(ty).ok_or("bad PLY type")?,
                    list: None,
                });
            }
            _ => {}
        }
    }
    let enc = enc.ok_or("PLY format line missing")?;
    let text_body = if enc == Encoding::Ascii {
        std::str::from_utf8(&bytes[body_start..]).map_err(|_| "bad PLY text")?
    } else {
        ""
    };
    let mut body = Body {
        enc,
        bytes: &bytes[body_start..],
        pos: 0,
        words: text_body.split_ascii_whitespace(),
    };

    let mut mesh = Mesh::default();
    let mut vertex_colors: Vec<Option<[f64; 4]>> = Vec::new();
    let mut vertex_uvs: Vec<Option<DVec2>> = Vec::new();
    // Corners, own color, per-corner texture coordinates.
    type Face = (Vec<usize>, Option<[f64; 4]>, Vec<DVec2>);
    let mut faces: Vec<Face> = Vec::new();
    // Each face's texture number (`texnumber`).
    let mut face_tex: Vec<Option<usize>> = Vec::new();
    for el in &elements {
        let idx = |names: &[&str]| {
            el.props
                .iter()
                .position(|p| names.contains(&p.name.as_str()))
        };
        let color_idx = |names: [&[&str]; 4]| {
            let c: Vec<Option<usize>> = names.iter().map(|n| idx(n)).collect();
            (c[0].is_some() && c[1].is_some() && c[2].is_some()).then(|| {
                let scale = if el.props[c[0].unwrap()].ty.is_float() {
                    1.0
                } else {
                    255.0
                };
                (c, scale)
            })
        };
        let rgba: [&[&str]; 4] = [
            &["red", "diffuse_red", "r"],
            &["green", "diffuse_green", "g"],
            &["blue", "diffuse_blue", "b"],
            &["alpha", "diffuse_alpha", "a"],
        ];
        let color_of = |rec: &[f64], c: &(Vec<Option<usize>>, f64)| {
            let (ix, scale) = c;
            let get = |k: usize, d: f64| ix[k].map_or(d, |i| rec[i] / scale);
            [get(0, 1.0), get(1, 1.0), get(2, 1.0), get(3, 1.0)]
        };
        match el.name.as_str() {
            "vertex" => {
                let (x, y, z) = (idx(&["x"]), idx(&["y"]), idx(&["z"]));
                let (Some(x), Some(y), Some(z)) = (x, y, z) else {
                    return Err("PLY vertices without x y z".into());
                };
                let colors = color_idx(rgba);
                let u = idx(&["s", "u", "texture_u", "texture_s"]);
                let v = idx(&["t", "v", "texture_v", "texture_t"]);
                for _ in 0..el.count {
                    let (rec, _) = read_record(&mut body, el)?;
                    mesh.positions.push(DVec3::new(rec[x], rec[y], rec[z]));
                    vertex_colors.push(colors.as_ref().map(|c| color_of(&rec, c)));
                    vertex_uvs.push(u.zip(v).map(|(u, v)| DVec2::new(rec[u], rec[v])));
                }
            }
            "face" => {
                let list = idx(&["vertex_indices", "vertex_index"])
                    .filter(|&i| el.props[i].list.is_some())
                    .ok_or("PLY faces without vertex_indices")?;
                let tc = idx(&["texcoord"]).filter(|&i| el.props[i].list.is_some());
                let tn = idx(&["texnumber"]).filter(|&i| el.props[i].list.is_none());
                let colors = color_idx(rgba);
                for _ in 0..el.count {
                    let (rec, lists) = read_record(&mut body, el)?;
                    let vs: Vec<usize> = lists[list]
                        .iter()
                        .map(|&i| {
                            let ok = i >= 0.0 && (i as usize) < mesh.positions.len();
                            ok.then_some(i as usize)
                                .ok_or_else(|| "PLY vertex index out of range".to_string())
                        })
                        .collect::<Result<_, _>>()?;
                    let uvs = tc
                        .map(|i| {
                            lists[i]
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|&c| DVec2::from_array(c))
                                .collect()
                        })
                        .unwrap_or_default();
                    faces.push((vs, colors.as_ref().map(|c| color_of(&rec, c)), uvs));
                    face_tex.push(tn.map(|i| rec[i]).filter(|&n| n >= 0.0).map(|n| n as usize));
                }
            }
            _ => {
                for _ in 0..el.count {
                    read_record(&mut body, el)?;
                }
            }
        }
    }

    // Colors of faces: their own, else their vertices' average.
    let face_color = |vs: &[usize], own: Option<[f64; 4]>| {
        own.or_else(|| {
            let cs: Vec<[f64; 4]> = vs.iter().filter_map(|&v| vertex_colors[v]).collect();
            (!cs.is_empty()).then(|| {
                let mut s = [0.0; 4];
                for c in &cs {
                    for k in 0..4 {
                        s[k] += c[k] / cs.len() as f64;
                    }
                }
                s
            })
        })
    };
    let bytes_of = |c: [f64; 4]| [0, 1, 2, 3].map(|k| super::unit_byte(c[k]));
    let mut face_bytes: Vec<Option<[u8; 4]>> = faces
        .iter()
        .map(|(vs, own, _)| {
            if colors {
                face_color(vs, *own).map(bytes_of)
            } else {
                None
            }
        })
        .collect();
    let distinct: std::collections::BTreeSet<[u8; 4]> =
        face_bytes.iter().flatten().copied().collect();
    if distinct.len() > 256 {
        for c in face_bytes.iter_mut().flatten() {
            *c = c.map(|x| ((u16::from(x) + 8) / 17 * 17).min(255) as u8);
        }
    }
    let textures: Vec<Option<MeshTexture>> = texture_files
        .iter()
        .map(|f| find_texture(f, read))
        .collect();
    let mut by_color: BTreeMap<[u8; 4], usize> = BTreeMap::new();
    // The material of each texture number.
    let mut textured_slot: BTreeMap<usize, usize> = BTreeMap::new();
    face_tex.resize(faces.len(), None);
    for (((vs, _, face_uvs), color), tex) in faces.into_iter().zip(face_bytes).zip(face_tex) {
        let uvs: Vec<DVec2> = if face_uvs.len() == vs.len() {
            face_uvs
        } else if vs.iter().all(|&v| vertex_uvs[v].is_some()) {
            vs.iter().filter_map(|&v| vertex_uvs[v]).collect()
        } else {
            Vec::new()
        };
        let number = tex.unwrap_or(0);
        let texture = textures.get(number).and_then(Option::as_ref);
        let material = if let (Some(t), false) = (texture, uvs.is_empty()) {
            Some(*textured_slot.entry(number).or_insert_with(|| {
                mesh.materials.push(MeshMaterial {
                    name: t
                        .filename
                        .rsplit_once('.')
                        .map_or(t.filename.as_str(), |x| x.0)
                        .to_owned(),
                    color: [255, 255, 255],
                    opacity: 1.0,
                    texture: Some(t.clone()),
                });
                mesh.materials.len() - 1
            }))
        } else {
            color.map(|c| {
                *by_color.entry(c).or_insert_with(|| {
                    mesh.materials.push(super::color_material(
                        [c[0], c[1], c[2]],
                        f64::from(c[3]) / 255.0,
                    ));
                    mesh.materials.len() - 1
                })
            })
        };
        mesh.polygons.push(Polygon {
            vertices: vs,
            material,
            uvs,
        });
    }
    Ok((mesh, Hints::default()))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// PLY export choices.
#[derive(Debug, Clone, PartialEq)]
pub struct PlyOptions {
    /// Units, axes, two-sided faces and `textures` (texture coordinates and images).
    pub common: ExportOptions,
    pub ascii: bool,
    /// Write `red green blue` per vertex.
    pub colors: bool,
}

/// Encodes the model as PLY. With textures, the texture images are written next to
/// it (MeshLab's `comment TextureFile` lines, in order) and each face gets a
/// `texcoord` list (empty for untextured faces) and, with several textures, a
/// `texnumber`.
pub fn to_ply(model: &Model, opts: &PlyOptions) -> Files {
    let (mats, groups) = groups(model, &opts.common);
    let textures = super::texture_files(&mats);
    // Each slot's texture number.
    let tex_of: Vec<Option<usize>> = mats
        .iter()
        .map(|m| {
            let (name, _) = m.texture.as_ref()?;
            textures.iter().position(|(n, _)| n == name)
        })
        .collect();
    let textured = !textures.is_empty();
    let numbered = textures.len() > 1;
    let nv: usize = groups.iter().map(|g| g.positions.len()).sum();
    let nf: usize = groups.iter().map(|g| g.triangles.len()).sum();
    let mut head = String::from("ply\n");
    let _ = writeln!(
        head,
        "format {} 1.0",
        if opts.ascii {
            "ascii"
        } else {
            "binary_little_endian"
        }
    );
    let _ = writeln!(
        head,
        "comment Exported by meshtard, units: {}",
        opts.common.units.name()
    );
    for (name, _) in &textures {
        let _ = writeln!(head, "comment TextureFile {name}");
    }
    let _ = writeln!(head, "element vertex {nv}");
    head.push_str("property float x\nproperty float y\nproperty float z\n");
    if opts.colors {
        head.push_str("property uchar red\nproperty uchar green\nproperty uchar blue\n");
    }
    let _ = writeln!(head, "element face {nf}");
    head.push_str("property list uchar int vertex_indices\n");
    if textured {
        head.push_str("property list uchar float texcoord\n");
    }
    if numbered {
        head.push_str("property int texnumber\n");
    }
    head.push_str("end_header\n");
    let mut out = head.into_bytes();
    let mut text = String::new();
    for (g, m) in groups.iter().zip(&mats) {
        for p in &g.positions {
            if opts.ascii {
                let _ = write!(
                    text,
                    "{} {} {}",
                    super::num(p.x),
                    super::num(p.y),
                    super::num(p.z)
                );
                if opts.colors {
                    let _ = write!(text, " {} {} {}", m.color[0], m.color[1], m.color[2]);
                }
                text.push('\n');
            } else {
                for x in p.to_array() {
                    out.extend_from_slice(&(x as f32).to_le_bytes());
                }
                if opts.colors {
                    out.extend_from_slice(&m.color);
                }
            }
        }
    }
    let mut base = 0;
    for (slot, g) in groups.iter().enumerate() {
        let tex = tex_of[slot];
        for t in &g.triangles {
            let ix = t.map(|(p, _)| (base + p) as i32);
            // Texture coordinates of a textured slot's corners.
            let uv: Vec<f64> = match tex {
                Some(_) => t.iter().flat_map(|&(_, k)| g.uvs[k].to_array()).collect(),
                None => Vec::new(),
            };
            let number = tex.map_or(-1, |n| n as i32);
            if opts.ascii {
                let _ = write!(text, "3 {} {} {}", ix[0], ix[1], ix[2]);
                if textured {
                    let _ = write!(text, " {}", uv.len());
                    for x in &uv {
                        let _ = write!(text, " {}", super::num(*x));
                    }
                }
                if numbered {
                    let _ = write!(text, " {number}");
                }
                text.push('\n');
            } else {
                out.push(3);
                for i in ix {
                    out.extend_from_slice(&i.to_le_bytes());
                }
                if textured {
                    out.push(uv.len() as u8);
                    for x in &uv {
                        out.extend_from_slice(&(*x as f32).to_le_bytes());
                    }
                }
                if numbered {
                    out.extend_from_slice(&number.to_le_bytes());
                }
            }
        }
        base += g.positions.len();
    }
    out.extend_from_slice(text.as_bytes());
    Files {
        main: out,
        others: textures,
    }
}

/// Distinct colors of a mesh's materials (tests).
#[cfg(test)]
fn colors(mesh: &Mesh) -> std::collections::HashMap<[u8; 3], usize> {
    let mut out = std::collections::HashMap::new();
    for p in &mesh.polygons {
        if let Some(m) = p.material {
            *out.entry(mesh.materials[m].color).or_insert(0) += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::tests::{painted_cube, summary};

    fn none(_: &str) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn ascii_with_face_colors() {
        let text = "ply\nformat ascii 1.0\ncomment TextureFile t.png\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 2\nproperty list uchar int vertex_indices\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n3 0 1 2 255 0 0\n3 0 2 3 0 0 255\n";
        let (mesh, _) = parse(text.as_bytes(), true, &none).unwrap();
        assert_eq!(mesh.polygons.len(), 2);
        assert_eq!(mesh.materials.len(), 2);
        assert_eq!(mesh.materials[0].name, "Color_FF0000");
        let (plain, _) = parse(text.as_bytes(), false, &none).unwrap();
        assert!(plain.materials.is_empty());
        assert!(
            parse(
                b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nend_header\n",
                false,
                &none
            )
            .is_err()
        );
    }

    #[test]
    fn binary_round_trip_keeps_colors_and_shape() {
        let m = painted_cube();
        for ascii in [false, true] {
            let ply = to_ply(
                &m,
                &PlyOptions {
                    common: ExportOptions {
                        units: crate::mesh_export::Units::Millimeters,
                        ..Default::default()
                    },
                    ascii,
                    colors: true,
                },
            )
            .main;
            let (mesh, _) = parse(&ply, true, &none).unwrap();
            assert_eq!(mesh.polygons.len(), 12);
            let c = colors(&mesh);
            assert_eq!(c.get(&[255, 0, 0]), Some(&2));
            let (faces, verts, max) = summary(&mesh, 1.0 / 25.4, false);
            assert_eq!((faces, verts), (6, 8), "ascii {ascii}");
            assert!((max - DVec3::splat(10.0)).length() < 1e-3);
        }
    }

    #[test]
    fn big_endian_and_vertex_colors() {
        let mut b = b"ply\r\nformat binary_big_endian 1.0\r\nelement vertex 3\r\nproperty double x\r\nproperty double y\r\nproperty double z\r\nproperty float red\r\nproperty float green\r\nproperty float blue\r\nelement face 1\r\nproperty list uchar uint vertex_indices\r\nend_header\r\n".to_vec();
        for (p, c) in [
            ([0.0, 0.0, 0.0], 1.0f32),
            ([2.0, 0.0, 0.0], 1.0),
            ([0.0, 2.0, 0.0], 0.0),
        ] {
            for x in p {
                b.extend_from_slice(&f64::to_be_bytes(x));
            }
            for _ in 0..3 {
                b.extend_from_slice(&c.to_be_bytes());
            }
        }
        b.push(3);
        for i in 0u32..3 {
            b.extend_from_slice(&i.to_be_bytes());
        }
        let (mesh, _) = parse(&b, true, &none).unwrap();
        assert_eq!(mesh.positions[1], DVec3::new(2.0, 0.0, 0.0));
        // Average of white, white, black.
        assert_eq!(mesh.materials[0].color, [170, 170, 170]);
    }

    #[test]
    fn texture_comment_gives_textured_faces() {
        let text = "ply\nformat ascii 1.0\ncomment TextureFile tex.png\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float s\nproperty float t\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 0 0\n1 0 0 1 0\n0 1 0 0 1\n3 0 1 2\n";
        let read = |p: &str| (p == "tex.png").then(|| b"\x89PNG".to_vec());
        let (mesh, _) = parse(text.as_bytes(), true, &read).unwrap();
        assert_eq!(mesh.materials.len(), 1);
        assert!(mesh.materials[0].texture.is_some());
        assert_eq!(mesh.polygons[0].uvs.len(), 3);
    }

    #[test]
    fn texture_coordinates_round_trip() {
        let mut m = painted_cube();
        let tex = |name: &str, byte: u8| crate::Texture {
            filename: name.into(),
            data: std::sync::Arc::from(vec![0x89, b'P', b'N', b'G', byte]),
            width: 10.0,
            height: 10.0,
            colorize: false,
        };
        m.materials[0].texture = Some(tex("a.png", 1));
        m.materials.push(crate::Material {
            name: "B".into(),
            color: [0, 0, 255],
            opacity: 1.0,
            texture: Some(tex("b.png", 2)),
        });
        let bottom = m
            .root
            .faces
            .iter()
            .find(|(_, f)| f.normal.z.abs() > 0.9 && m.root.loop_points(&f.outer)[0].z < 5.0)
            .map(|(id, _)| id)
            .unwrap();
        m.root.faces[bottom].attrs.material = Some(1);
        for ascii in [false, true] {
            let files = to_ply(
                &m,
                &PlyOptions {
                    common: ExportOptions::default(),
                    ascii,
                    colors: true,
                },
            );
            assert_eq!(files.others.len(), 2);
            let text = String::from_utf8_lossy(&files.main);
            assert!(
                text.contains("comment TextureFile model_Red.png"),
                "{text:.400}"
            );
            assert!(text.contains("property int texnumber"));
            let read = |name: &str| {
                files
                    .others
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, d)| d.to_vec())
            };
            let (mesh, _) = parse(&files.main, true, &read).unwrap();
            assert_eq!(mesh.polygons.len(), 12);
            let textured: Vec<_> = mesh
                .materials
                .iter()
                .filter_map(|x| x.texture.as_ref().map(|t| t.data[4]))
                .collect();
            assert_eq!(textured.len(), 2, "ascii {ascii}");
            // Two textured triangles per texture, with coordinates.
            for (i, mat) in mesh.materials.iter().enumerate() {
                if mat.texture.is_some() {
                    let tris: Vec<_> = mesh
                        .polygons
                        .iter()
                        .filter(|p| p.material == Some(i))
                        .collect();
                    assert_eq!(tris.len(), 2);
                    assert!(tris.iter().all(|p| p.uvs.len() == 3));
                }
            }
            // The other faces keep their color.
            assert!(mesh.polygons.iter().filter(|p| p.uvs.is_empty()).count() == 8);
        }
    }
}
