//! 3DS (3D Studio) import, from the public chunk descriptions of the format (Jeff
//! Lewis's "3D Studio File Format" notes and the `lib3ds` chunk list).
//!
//! - Chunks: little-endian id (u16) and length (u32, header included). Meshes are
//!   `0x4000` named objects holding a `0x4100` triangle mesh: `0x4110` vertices,
//!   `0x4120` faces (with `0x4130` per-material face lists) and `0x4140` texture
//!   coordinates. Vertices are stored in world coordinates, so the keyframer's
//!   hierarchy and pivots (`0xB000`) are not needed and are ignored.
//! - Materials (`0xAFFF`): name, diffuse color (byte or float, gamma-corrected or not)
//!   and transparency percentage; the first texture map's file (`0xA200`/`0xA300`),
//!   found through [`super::ReadFile`] (8.3 names are tried as written and in other
//!   cases).
//! - The master scale (`0x0100`) multiplies coordinates; 3DS records no unit, so the
//!   dialog defaults to inches. 3DS is Z up.
//! - Cameras, lights and animation are ignored.

use glam::{DVec2, DVec3};

use super::{Hints, ReadFile, find_texture};
use crate::mesh_import::{Mesh, MeshMaterial, MeshTexture, Polygon};

struct Chunks<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for Chunks<'a> {
    /// Chunk id and contents (without the 6-byte header).
    type Item = (u16, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let d = self.data.get(self.pos..self.pos + 6)?;
        let id = u16::from_le_bytes([d[0], d[1]]);
        let len = u32::from_le_bytes([d[2], d[3], d[4], d[5]]) as usize;
        if len < 6 {
            return None;
        }
        let end = (self.pos + len).min(self.data.len());
        let body = &self.data[self.pos + 6..end];
        self.pos = end;
        Some((id, body))
    }
}

fn chunks(data: &[u8]) -> Chunks<'_> {
    Chunks { data, pos: 0 }
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn f32_at(d: &[u8], o: usize) -> Option<f64> {
    Some(f64::from(f32::from_le_bytes(
        d.get(o..o + 4)?.try_into().ok()?,
    )))
}

/// A zero-terminated string and the bytes after it.
fn cstr(d: &[u8]) -> (String, &[u8]) {
    let n = d.iter().position(|&b| b == 0).unwrap_or(d.len());
    (
        String::from_utf8_lossy(&d[..n]).into_owned(),
        d.get(n + 1..).unwrap_or(&[]),
    )
}

/// A color chunk's value (0..1), preferring the gamma-corrected byte color.
fn color(d: &[u8]) -> Option<[f64; 3]> {
    let mut out = None;
    for (id, c) in chunks(d) {
        match id {
            0x0011 | 0x0012 if c.len() >= 3 => {
                return Some([0, 1, 2].map(|k| f64::from(c[k]) / 255.0));
            }
            0x0010 | 0x0013 => {
                out = Some([f32_at(c, 0)?, f32_at(c, 4)?, f32_at(c, 8)?]);
            }
            _ => {}
        }
    }
    out
}

/// A percentage chunk's value (0..1).
fn percent(d: &[u8]) -> Option<f64> {
    chunks(d).find_map(|(id, c)| match id {
        0x0030 => u16_at(c, 0).map(|x| f64::from(x) / 100.0),
        0x0031 => f32_at(c, 0).map(|x| x / 100.0),
        _ => None,
    })
}

struct Material {
    name: String,
    color: [f64; 3],
    transparency: f64,
    map: Option<String>,
}

fn material(d: &[u8]) -> Material {
    let mut m = Material {
        name: String::new(),
        color: [1.0; 3],
        transparency: 0.0,
        map: None,
    };
    for (id, c) in chunks(d) {
        match id {
            0xA000 => m.name = cstr(c).0,
            0xA020 => m.color = color(c).unwrap_or(m.color),
            0xA050 => m.transparency = percent(c).unwrap_or(0.0),
            0xA200 if m.map.is_none() => {
                m.map = chunks(c)
                    .find(|(id, _)| *id == 0xA300)
                    .map(|(_, f)| cstr(f).0);
            }
            _ => {}
        }
    }
    m
}

/// One object's triangle mesh.
#[derive(Default)]
struct TriMesh {
    vertices: Vec<DVec3>,
    uvs: Vec<DVec2>,
    faces: Vec<[usize; 3]>,
    /// Material name per face list.
    face_materials: Vec<(String, Vec<usize>)>,
}

fn tri_mesh(d: &[u8]) -> Result<TriMesh, String> {
    let mut m = TriMesh::default();
    let short = || "3DS: truncated mesh".to_string();
    for (id, c) in chunks(d) {
        match id {
            0x4110 => {
                let n = u16_at(c, 0).ok_or_else(short)? as usize;
                for i in 0..n {
                    let o = 2 + i * 12;
                    m.vertices.push(DVec3::new(
                        f32_at(c, o).ok_or_else(short)?,
                        f32_at(c, o + 4).ok_or_else(short)?,
                        f32_at(c, o + 8).ok_or_else(short)?,
                    ));
                }
            }
            0x4140 => {
                let n = u16_at(c, 0).ok_or_else(short)? as usize;
                for i in 0..n {
                    let o = 2 + i * 8;
                    m.uvs.push(DVec2::new(
                        f32_at(c, o).ok_or_else(short)?,
                        f32_at(c, o + 4).ok_or_else(short)?,
                    ));
                }
            }
            0x4120 => {
                let n = u16_at(c, 0).ok_or_else(short)? as usize;
                for i in 0..n {
                    let o = 2 + i * 8;
                    m.faces
                        .push([0, 2, 4].map(|k| u16_at(c, o + k).unwrap_or(0) as usize));
                }
                if c.len() < 2 + n * 8 {
                    return Err(short());
                }
                for (sid, s) in chunks(&c[2 + n * 8..]) {
                    if sid == 0x4130 {
                        let (name, rest) = cstr(s);
                        let k = u16_at(rest, 0).unwrap_or(0) as usize;
                        let list = (0..k)
                            .filter_map(|i| u16_at(rest, 2 + i * 2))
                            .map(usize::from)
                            .collect();
                        m.face_materials.push((name, list));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(m)
}

/// Reads a 3DS file. `read` finds texture files.
pub fn parse(bytes: &[u8], read: ReadFile) -> Result<(Mesh, Hints), String> {
    let (id, main) = chunks(bytes).next().ok_or("not a 3DS file")?;
    if id != 0x4D4D {
        return Err("not a 3DS file".into());
    }
    let mut scale = 1.0;
    let mut materials: Vec<Material> = Vec::new();
    let mut meshes: Vec<TriMesh> = Vec::new();
    for (id, c) in chunks(main) {
        if id != 0x3D3D {
            continue;
        }
        for (id, e) in chunks(c) {
            match id {
                0x0100 => scale = f32_at(e, 0).filter(|s| *s > 0.0).unwrap_or(1.0),
                0xAFFF => materials.push(material(e)),
                0x4000 => {
                    let (_, rest) = cstr(e);
                    for (oid, o) in chunks(rest) {
                        if oid == 0x4100 {
                            meshes.push(tri_mesh(o)?);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut mesh = Mesh::default();
    let mut used: Vec<Option<usize>> = vec![None; materials.len()];
    let mut textures: Vec<Option<Option<MeshTexture>>> = vec![None; materials.len()];
    for tm in meshes {
        let base = mesh.positions.len();
        mesh.positions
            .extend(tm.vertices.iter().map(|&p| p * scale));
        let mut face_mat: Vec<Option<usize>> = vec![None; tm.faces.len()];
        for (name, list) in &tm.face_materials {
            let Some(mi) = materials.iter().position(|m| &m.name == name) else {
                continue;
            };
            for &f in list {
                if let Some(slot) = face_mat.get_mut(f) {
                    *slot = Some(mi);
                }
            }
        }
        for (f, v) in tm.faces.iter().enumerate() {
            if v.iter().any(|&i| i >= tm.vertices.len()) {
                return Err("3DS: vertex index out of range".into());
            }
            let material = face_mat[f].map(|mi| {
                *used[mi].get_or_insert_with(|| {
                    let m = &materials[mi];
                    let texture = textures[mi]
                        .get_or_insert_with(|| m.map.as_deref().and_then(|f| texture(f, read)))
                        .clone();
                    mesh.materials.push(MeshMaterial {
                        name: if m.name.is_empty() {
                            "Material".into()
                        } else {
                            m.name.clone()
                        },
                        color: m.color.map(super::unit_byte),
                        opacity: 1.0 - m.transparency.clamp(0.0, 1.0),
                        texture,
                    });
                    mesh.materials.len() - 1
                })
            });
            let uvs = if tm.uvs.len() == tm.vertices.len() {
                v.iter().map(|&i| tm.uvs[i]).collect()
            } else {
                Vec::new()
            };
            mesh.polygons.push(Polygon {
                vertices: v.iter().map(|&i| base + i).collect(),
                material,
                uvs,
            });
        }
    }
    Ok((
        mesh,
        Hints {
            units: None,
            y_up: Some(false),
        },
    ))
}

/// A texture file, also tried in lower and upper case (3DS stores DOS 8.3 names).
fn texture(name: &str, read: ReadFile) -> Option<MeshTexture> {
    [name.to_owned(), name.to_lowercase(), name.to_uppercase()]
        .iter()
        .find_map(|n| find_texture(n, read))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u16, body: &[u8]) -> Vec<u8> {
        let mut out = id.to_le_bytes().to_vec();
        out.extend_from_slice(&((body.len() + 6) as u32).to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    /// A one-triangle 3DS file with a red material, written from the chunk layout.
    pub(crate) fn triangle_3ds() -> Vec<u8> {
        let mut verts = 3u16.to_le_bytes().to_vec();
        for p in [[0.0f32, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]] {
            for x in p {
                verts.extend_from_slice(&x.to_le_bytes());
            }
        }
        let mut faces = 1u16.to_le_bytes().to_vec();
        for i in [0u16, 1, 2, 0] {
            faces.extend_from_slice(&i.to_le_bytes());
        }
        let mut msh = b"Red\0".to_vec();
        msh.extend_from_slice(&1u16.to_le_bytes());
        msh.extend_from_slice(&0u16.to_le_bytes());
        faces.extend(chunk(0x4130, &msh));
        let mut trimesh = chunk(0x4110, &verts);
        trimesh.extend(chunk(0x4120, &faces));
        let mut obj = b"Tri\0".to_vec();
        obj.extend(chunk(0x4100, &trimesh));
        let mut mat = chunk(0xA000, b"Red\0");
        mat.extend(chunk(0xA020, &chunk(0x0011, &[255, 0, 0])));
        mat.extend(chunk(0xA050, &chunk(0x0030, &25u16.to_le_bytes())));
        let mut edit = chunk(0x0100, &1.5f32.to_le_bytes());
        edit.extend(chunk(0xAFFF, &mat));
        edit.extend(chunk(0x4000, &obj));
        chunk(0x4D4D, &chunk(0x3D3D, &edit))
    }

    #[test]
    fn reads_mesh_materials_and_scale() {
        let none = |_: &str| None;
        let (mesh, _) = parse(&triangle_3ds(), &none).unwrap();
        assert_eq!(mesh.polygons.len(), 1);
        assert_eq!(mesh.positions[1], DVec3::new(3.0, 0.0, 0.0));
        assert_eq!(mesh.materials.len(), 1);
        assert_eq!(mesh.materials[0].color, [255, 0, 0]);
        assert!((mesh.materials[0].opacity - 0.75).abs() < 1e-9);
        assert_eq!(mesh.polygons[0].material, Some(0));
        assert!(parse(b"garbage", &none).is_err());
    }
}
