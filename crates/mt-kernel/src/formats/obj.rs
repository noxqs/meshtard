//! OBJ texture support, as a pass after [`crate::mesh_import::parse_obj`] (which reads
//! positions, faces and MTL colors): texture coordinates (`vt`, the second index of a
//! face corner) and MTL diffuse texture maps (`map_Kd`, the last word of the
//! statement, options before it ignored), PNG or JPEG.

use glam::DVec2;

use super::{ReadFile, find_texture};
use crate::mesh_import::Mesh;

/// Adds texture coordinates and textures to `mesh`, parsed from `text` by
/// `parse_obj`. `read` reads the MTL libraries and images next to the OBJ.
pub fn add_textures(text: &str, mesh: &mut Mesh, read: ReadFile) {
    let mut vts: Vec<DVec2> = Vec::new();
    let mut face = 0;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let mut it = line.split_whitespace();
        match it.next() {
            Some("vt") => {
                let c: Vec<f64> = it.take(2).filter_map(|x| x.parse().ok()).collect();
                vts.push(DVec2::new(
                    c.first().copied().unwrap_or(0.0),
                    c.get(1).copied().unwrap_or(0.0),
                ));
            }
            Some("f") => {
                let uvs: Option<Vec<DVec2>> = it
                    .map(|c| {
                        let i: i64 = c.split('/').nth(1)?.parse().ok()?;
                        let i = if i < 0 { vts.len() as i64 + i } else { i - 1 };
                        vts.get(usize::try_from(i).ok()?).copied()
                    })
                    .collect();
                if let (Some(uvs), Some(p)) = (uvs, mesh.polygons.get_mut(face))
                    && uvs.len() == p.vertices.len()
                {
                    p.uvs = uvs;
                }
                face += 1;
            }
            Some("mtllib") => {
                let name = it.collect::<Vec<_>>().join(" ");
                if let Some(mtl) = read(&name) {
                    add_maps(&String::from_utf8_lossy(&mtl), mesh, read);
                }
            }
            _ => {}
        }
    }
}

/// Sets the textures of `mesh`'s materials from an MTL library's `map_Kd` lines.
fn add_maps(mtl: &str, mesh: &mut Mesh, read: ReadFile) {
    let mut current: Option<usize> = None;
    for line in mtl.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("newmtl") => {
                let name = it.collect::<Vec<_>>().join(" ");
                current = mesh.materials.iter().position(|m| m.name == name);
            }
            Some(key) if key.eq_ignore_ascii_case("map_Kd") => {
                let (Some(m), Some(file)) = (current, it.last()) else {
                    continue;
                };
                if mesh.materials[m].texture.is_none() {
                    mesh.materials[m].texture = find_texture(file, read);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_import::parse_obj;

    #[test]
    fn uvs_and_maps() {
        let obj = "mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl Wood\nf 1/1 2/2 3/3\nf 1 2 3\nf 1/-3/1 2/-2/1 3/-1/1\n";
        let mtl = "newmtl Wood\nKd 1 1 1\nmap_Kd -s 1 1 1 wood.jpg\n";
        let read = |p: &str| match p {
            "m.mtl" => Some(mtl.as_bytes().to_vec()),
            "wood.jpg" => Some(vec![0xff, 0xd8, 0xff]),
            _ => None,
        };
        let mut mesh = parse_obj(obj, |n| read(n).map(|b| String::from_utf8(b).unwrap())).unwrap();
        add_textures(obj, &mut mesh, &read);
        assert_eq!(mesh.polygons[0].uvs[1], DVec2::new(1.0, 0.0));
        assert!(mesh.polygons[1].uvs.is_empty());
        assert_eq!(mesh.polygons[2].uvs[2], DVec2::new(0.0, 1.0));
        let t = mesh.materials[0].texture.as_ref().unwrap();
        assert_eq!(t.filename, "wood.jpg");
    }

    #[test]
    fn bridged_hole_polygon_becomes_face_with_hole() {
        // Outline 1-2-3-4, bridge 1-5, hole 5-8-7-6 (clockwise), back over the bridge.
        let obj = "v 0 0 0\nv 10 0 0\nv 10 10 0\nv 0 10 0\nv 3 3 0\nv 7 3 0\nv 7 7 0\nv 3 7 0\nf 1 2 3 4 1 5 8 7 6 5\n";
        let mesh = parse_obj(obj, |_| None).unwrap();
        for merge in [false, true] {
            let ents = crate::mesh_import::to_entities(
                &mesh,
                &crate::mesh_import::MeshOptions {
                    inches_per_unit: 1.0,
                    y_up: false,
                    merge_coplanar: merge,
                },
                &[],
            );
            let area: f64 = ents
                .faces
                .keys()
                .flat_map(|f| crate::tessellate::triangulate(&ents, f))
                .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0)
                .sum();
            assert!((area - 84.0).abs() < 1e-9, "merge {merge}: {area}");
            if merge {
                assert_eq!(ents.faces.len(), 1);
                assert_eq!(ents.faces.values().next().unwrap().holes.len(), 1);
            }
        }
    }

    #[test]
    fn texture_coordinates_survive_import_and_export() {
        // A 2 x 1 quad showing the texture once, rotated a quarter turn.
        let obj = "mtllib m.mtl\nv 0 0 0\nv 2 0 0\nv 2 1 0\nv 0 1 0\nvt 0 1\nvt 0 0\nvt 1 0\nvt 1 1\nusemtl Wood\nf 1/1 2/2 3/3 4/4\n";
        let mtl = "newmtl Wood\nmap_Kd wood.jpg\n";
        let read = |p: &str| match p {
            "m.mtl" => Some(mtl.as_bytes().to_vec()),
            "wood.jpg" => Some(vec![0xff, 0xd8, 0xff]),
            _ => None,
        };
        let (mesh, _) =
            crate::formats::read_mesh(crate::formats::Kind::Obj, obj.as_bytes(), &read, true)
                .unwrap();
        let ents = crate::mesh_import::to_entities(
            &mesh,
            &crate::mesh_import::MeshOptions {
                inches_per_unit: 1.0,
                y_up: false,
                merge_coplanar: true,
            },
            &[0],
        );
        let mut model = crate::Model::default();
        model
            .materials
            .push(crate::formats::model_material(&mesh.materials[0]));
        model.root = ents;
        let tris = crate::mesh_export::triangles(&model);
        assert_eq!(tris.len(), 2);
        let expect = |p: glam::DVec3| DVec2::new(p.y, 1.0 - p.x / 2.0);
        for t in &tris {
            let uv = t.uv.expect("textured");
            for k in 0..3 {
                let d = uv[k] - expect(t.points[k]);
                // Up to a whole number of repeats.
                assert!((d - d.round()).length() < 1e-9, "{uv:?} at {:?}", t.points);
                assert!(
                    (d - uv[0] + expect(t.points[0])).length() < 1e-9,
                    "{uv:?} at {:?}",
                    t.points
                );
            }
        }
    }
}
