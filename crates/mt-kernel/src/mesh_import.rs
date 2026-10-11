//! File > Import of triangle and polygon meshes: Wavefront OBJ (with MTL colors) and
//! STL (binary or ASCII), turned into kernel faces.
//!
//! - Vertices closer than the kernel tolerance are welded; edges shared by faces are
//!   shared. Degenerate polygons and repeated faces are skipped; non-planar polygons
//!   are split into a triangle fan.
//! - "Merge coplanar faces" (SketchUp's STL importer option, from memory) joins
//!   neighboring coplanar faces of the same material into one face, removing the edges
//!   between them; a region that would not give one simple outer loop is left as is.
//! - OBJ groups, objects, smoothing groups, normals and texture coordinates are
//!   ignored; MTL diffuse colors and opacity become materials (texture maps are not
//!   read).

use std::collections::{HashMap, HashSet};

use glam::DVec3;

use crate::TOL;
use crate::entities::{CoEdge, EdgeAttrs, Entities, FaceAttrs, FaceId, Loop, MaterialId, VertexId};
use crate::geom::{Plane, signed_area2};

/// A polygon mesh as read from a file, in file units.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<DVec3>,
    pub polygons: Vec<Polygon>,
    pub materials: Vec<MeshMaterial>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Polygon {
    /// Indices into `positions`.
    pub vertices: Vec<usize>,
    /// Index into `materials`.
    pub material: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshMaterial {
    pub name: String,
    /// sRGB.
    pub color: [u8; 3],
    pub opacity: f64,
}

/// Reads MTL materials (`newmtl`, `Kd`, `d`, `Tr`).
pub fn parse_mtl(text: &str) -> Vec<MeshMaterial> {
    let mut out: Vec<MeshMaterial> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        let nums: Vec<f64> = it.clone().filter_map(|x| x.parse().ok()).collect();
        match key {
            "newmtl" => out.push(MeshMaterial {
                name: it.collect::<Vec<_>>().join(" "),
                color: [255, 255, 255],
                opacity: 1.0,
            }),
            "Kd" if nums.len() >= 3 => {
                if let Some(m) = out.last_mut() {
                    m.color = [0, 1, 2].map(|i| (nums[i].clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
            "d" if !nums.is_empty() => {
                if let Some(m) = out.last_mut() {
                    m.opacity = nums[0].clamp(0.0, 1.0);
                }
            }
            "Tr" if !nums.is_empty() => {
                if let Some(m) = out.last_mut() {
                    m.opacity = 1.0 - nums[0].clamp(0.0, 1.0);
                }
            }
            _ => {}
        }
    }
    out
}

/// Reads an OBJ file. `mtl` returns the text of a material library named by `mtllib`
/// (relative to the OBJ file), or `None` when it cannot be read.
pub fn parse_obj(text: &str, mtl: impl Fn(&str) -> Option<String>) -> Result<Mesh, String> {
    let mut mesh = Mesh::default();
    let mut current = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("");
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        match key {
            "v" => {
                let c: Vec<f64> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if c.len() < 3 {
                    return Err(format!("line {}: bad vertex", n + 1));
                }
                mesh.positions.push(DVec3::new(c[0], c[1], c[2]));
            }
            "f" => {
                let count = mesh.positions.len() as i64;
                let vertices = it
                    .map(|c| {
                        let i: i64 = c
                            .split('/')
                            .next()
                            .and_then(|x| x.parse().ok())
                            .ok_or_else(|| format!("line {}: bad face", n + 1))?;
                        let i = if i < 0 { count + i } else { i - 1 };
                        if (0..count).contains(&i) {
                            Ok(i as usize)
                        } else {
                            Err(format!("line {}: vertex index out of range", n + 1))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                mesh.polygons.push(Polygon {
                    vertices,
                    material: current,
                });
            }
            "mtllib" => {
                let name = it.collect::<Vec<_>>().join(" ");
                if let Some(text) = mtl(&name) {
                    for m in parse_mtl(&text) {
                        if !mesh.materials.iter().any(|x| x.name == m.name) {
                            mesh.materials.push(m);
                        }
                    }
                }
            }
            "usemtl" => {
                let name = it.collect::<Vec<_>>().join(" ");
                current = Some(match mesh.materials.iter().position(|m| m.name == name) {
                    Some(i) => i,
                    None => {
                        // A material the library did not define: plain white.
                        mesh.materials.push(MeshMaterial {
                            name,
                            color: [255, 255, 255],
                            opacity: 1.0,
                        });
                        mesh.materials.len() - 1
                    }
                });
            }
            _ => {}
        }
    }
    Ok(mesh)
}

/// Reads a binary or ASCII STL file.
pub fn parse_stl(bytes: &[u8]) -> Result<Mesh, String> {
    let mut mesh = Mesh::default();
    let binary_count = bytes
        .get(80..84)
        .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")) as usize);
    if let Some(n) = binary_count.filter(|&n| bytes.len() == 84 + n * 50) {
        for t in 0..n {
            let base = 84 + t * 50 + 12;
            let f = |k: usize| {
                let o = base + k * 4;
                f64::from(f32::from_le_bytes(bytes[o..o + 4].try_into().expect("4")))
            };
            let first = mesh.positions.len();
            for v in 0..3 {
                mesh.positions
                    .push(DVec3::new(f(v * 3), f(v * 3 + 1), f(v * 3 + 2)));
            }
            mesh.polygons.push(Polygon {
                vertices: vec![first, first + 1, first + 2],
                material: None,
            });
        }
        return Ok(mesh);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not an STL file".to_string())?;
    if !text.trim_start().starts_with("solid") {
        return Err("not an STL file".into());
    }
    let mut facet: Vec<usize> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("vertex") => {
                let c: Vec<f64> = it.filter_map(|x| x.parse().ok()).collect();
                if c.len() < 3 {
                    return Err("bad STL vertex".into());
                }
                facet.push(mesh.positions.len());
                mesh.positions.push(DVec3::new(c[0], c[1], c[2]));
            }
            Some("endloop") => {
                mesh.polygons.push(Polygon {
                    vertices: std::mem::take(&mut facet),
                    material: None,
                });
            }
            _ => {}
        }
    }
    Ok(mesh)
}

/// Mesh import choices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshOptions {
    /// Inches per file unit.
    pub inches_per_unit: f64,
    /// The file is Y up (OBJ's usual convention): its Y becomes Z.
    pub y_up: bool,
    pub merge_coplanar: bool,
}

/// Builds faces from `mesh` in a new collection. `materials` maps the mesh's materials
/// to model materials.
pub fn to_entities(mesh: &Mesh, opts: &MeshOptions, materials: &[MaterialId]) -> Entities {
    let mut ents = Entities::new();
    let mut welded: HashMap<[i64; 3], VertexId> = HashMap::new();
    let mut vertex = |ents: &mut Entities, i: usize| -> VertexId {
        let p = mesh.positions[i];
        let p = if opts.y_up {
            DVec3::new(p.x, -p.z, p.y)
        } else {
            p
        } * opts.inches_per_unit;
        let key = (p / TOL).round().as_i64vec3().to_array();
        *welded.entry(key).or_insert_with(|| ents.add_vertex(p))
    };
    let mut seen: HashSet<Vec<VertexId>> = HashSet::new();
    for poly in &mesh.polygons {
        let mut vs: Vec<VertexId> = Vec::with_capacity(poly.vertices.len());
        for &i in &poly.vertices {
            let v = vertex(&mut ents, i);
            if vs.last() != Some(&v) {
                vs.push(v);
            }
        }
        while vs.len() > 1 && vs.first() == vs.last() {
            vs.pop();
        }
        let material = poly.material.and_then(|m| materials.get(m).copied());
        let points: Vec<DVec3> = vs.iter().map(|&v| ents.pos(v)).collect();
        let Some(plane) = Plane::from_polygon(&points) else {
            continue;
        };
        let distinct: HashSet<VertexId> = vs.iter().copied().collect();
        if distinct.len() != vs.len() || vs.len() < 3 {
            continue;
        }
        if points.iter().all(|&p| plane.contains(p)) {
            add_face(&mut ents, &vs, plane.normal, material, &mut seen);
        } else {
            for k in 1..vs.len() - 1 {
                let tri = [vs[0], vs[k], vs[k + 1]];
                let pts = tri.map(|v| ents.pos(v));
                if let Some(p) = Plane::from_polygon(&pts) {
                    add_face(&mut ents, &tri, p.normal, material, &mut seen);
                }
            }
        }
    }
    if opts.merge_coplanar {
        merge_coplanar(&mut ents);
    }
    ents
}

/// Adds a face through `vs` (counter-clockwise around `normal`), reusing edges.
/// A face over the same vertices as one added before is skipped.
fn add_face(
    ents: &mut Entities,
    vs: &[VertexId],
    normal: DVec3,
    material: Option<MaterialId>,
    seen: &mut HashSet<Vec<VertexId>>,
) {
    // Area check in the plane (collinear points pass Newell with a tiny normal).
    let plane = Plane::new(normal, ents.pos(vs[0]));
    let poly: Vec<_> = vs.iter().map(|&v| plane.project(ents.pos(v))).collect();
    if signed_area2(&poly).abs() < TOL * TOL {
        return;
    }
    let mut key = vs.to_vec();
    key.sort();
    if !seen.insert(key) {
        return;
    }
    let coedges = (0..vs.len())
        .map(|k| {
            let (a, b) = (vs[k], vs[(k + 1) % vs.len()]);
            let e = ents
                .edge_between(a, b)
                .unwrap_or_else(|| ents.add_raw_edge(a, b, EdgeAttrs::default()));
            CoEdge {
                edge: e,
                reversed: ents.edges[e].start != a,
            }
        })
        .collect();
    ents.add_raw_face(
        Loop { coedges },
        Vec::new(),
        normal,
        FaceAttrs {
            material,
            ..Default::default()
        },
    );
}

/// Joins each connected region of coplanar faces with the same material into one face.
fn merge_coplanar(ents: &mut Entities) {
    let faces: Vec<FaceId> = ents.faces.keys().collect();
    let mut done: HashSet<FaceId> = HashSet::new();
    for seed in faces {
        if done.contains(&seed) || !ents.faces.contains_key(seed) {
            continue;
        }
        let plane = ents.face_plane(seed);
        let material = ents.faces[seed].attrs.material;
        // Flood across edges shared by exactly two faces of the region.
        let mut region = vec![seed];
        let mut members: HashSet<FaceId> = HashSet::from([seed]);
        let mut k = 0;
        while k < region.len() {
            let f = region[k];
            k += 1;
            for c in &ents.faces[f].outer.coedges {
                let e = &ents.edges[c.edge];
                if e.faces.len() != 2 {
                    continue;
                }
                let g = if e.faces[0] == f {
                    e.faces[1]
                } else {
                    e.faces[0]
                };
                if members.contains(&g) || done.contains(&g) {
                    continue;
                }
                let gf = &ents.faces[g];
                let same = gf.attrs.material == material
                    && gf.normal.dot(plane.normal) > 1.0 - 1e-6
                    && ents
                        .loop_points(&gf.outer)
                        .iter()
                        .all(|&p| plane.contains(p));
                if same {
                    members.insert(g);
                    region.push(g);
                }
            }
        }
        done.extend(region.iter().copied());
        if region.len() > 1 {
            merge_region(ents, &region, &members, plane);
        }
    }
}

/// Replaces the faces of `region` by one face bounded by the region's boundary.
fn merge_region(ents: &mut Entities, region: &[FaceId], members: &HashSet<FaceId>, plane: Plane) {
    let interior = |ents: &Entities, e| {
        let faces = &ents.edges[e].faces;
        faces.len() == 2 && faces.iter().all(|f| members.contains(f))
    };
    // Boundary coedges in loop direction, by start vertex.
    let mut next: HashMap<VertexId, Vec<CoEdge>> = HashMap::new();
    let mut count = 0;
    for &f in region {
        for c in &ents.faces[f].outer.coedges {
            if !interior(ents, c.edge) {
                next.entry(ents.coedge_ends(*c).0).or_default().push(*c);
                count += 1;
            }
        }
    }
    let mut loops: Vec<Loop> = Vec::new();
    let mut used = 0;
    while let Some((&start, _)) = next.iter().find(|(_, v)| !v.is_empty()) {
        let mut coedges = Vec::new();
        let mut v = start;
        loop {
            let Some(c) = next.get_mut(&v).and_then(Vec::pop) else {
                return; // Open boundary: leave the region alone.
            };
            coedges.push(c);
            used += 1;
            v = ents.coedge_ends(c).1;
            if v == start {
                break;
            }
        }
        loops.push(Loop { coedges });
    }
    if used != count {
        return;
    }
    let area = |ents: &Entities, l: &Loop| {
        let pts: Vec<_> = ents
            .loop_points(l)
            .into_iter()
            .map(|p| plane.project(p))
            .collect();
        signed_area2(&pts)
    };
    let outer: Vec<usize> = (0..loops.len())
        .filter(|&i| area(ents, &loops[i]) > 0.0)
        .collect();
    let [outer] = outer[..] else {
        return; // Pinched or several regions: keep the triangles.
    };
    let normal = ents.faces[region[0]].normal;
    let attrs = ents.faces[region[0]].attrs.clone();
    let mut interior_edges: HashSet<_> = HashSet::new();
    for &f in region {
        for c in &ents.faces[f].outer.coedges {
            if interior(ents, c.edge) {
                interior_edges.insert(c.edge);
            }
        }
    }
    for &f in region {
        ents.remove_face(f);
    }
    for e in interior_edges {
        let (a, b) = (ents.edges[e].start, ents.edges[e].end);
        ents.unlink_edge(e);
        ents.remove_vertex_if_unused(a);
        ents.remove_vertex_if_unused(b);
    }
    let outer_loop = loops.remove(outer);
    ents.add_raw_face(outer_loop, loops, normal, attrs);
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE_OBJ: &str = "\
mtllib box.mtl
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
v 0 0 1
v 1 0 1
v 1 1 1
v 0 1 1
usemtl Red
f 1 4 3 2
f 5 6 7 8
f 1 2 6 5
f 2 3 7 6
f 3 4 8 7
f 4 1 5 8
";

    fn opts(merge: bool) -> MeshOptions {
        MeshOptions {
            inches_per_unit: 1.0,
            y_up: false,
            merge_coplanar: merge,
        }
    }

    #[test]
    fn obj_cube_with_materials() {
        let mesh = parse_obj(CUBE_OBJ, |name| {
            assert_eq!(name, "box.mtl");
            Some("newmtl Red\nKd 1 0 0\nd 0.5\n".into())
        })
        .unwrap();
        assert_eq!(mesh.positions.len(), 8);
        assert_eq!(mesh.polygons.len(), 6);
        assert_eq!(mesh.materials[0].color, [255, 0, 0]);
        assert_eq!(mesh.materials[0].opacity, 0.5);
        let ents = to_entities(&mesh, &opts(false), &[7]);
        assert_eq!(ents.faces.len(), 6);
        assert_eq!(ents.edges.len(), 12);
        assert_eq!(ents.vertices.len(), 8);
        assert!(ents.faces.values().all(|f| f.attrs.material == Some(7)));
        assert!(crate::validate(&ents, crate::Checks::ALL).is_empty());
        // Outward normals: the bottom face looks down.
        assert!(ents.faces.values().any(|f| f.normal.z < -0.99));
    }

    #[test]
    fn stl_round_trip_merges_coplanar_triangles() {
        let mut m = crate::Model::default();
        crate::mesh_export::tests::cube(&mut m.root, DVec3::splat(10.0));
        let stl = crate::mesh_export::to_stl(&m, crate::mesh_export::Units::Millimeters);
        let mesh = parse_stl(&stl).unwrap();
        assert_eq!(mesh.polygons.len(), 12);
        let o = MeshOptions {
            inches_per_unit: 1.0 / 25.4,
            ..opts(false)
        };
        let tris = to_entities(&mesh, &o, &[]);
        assert_eq!(tris.faces.len(), 12);
        assert_eq!(tris.vertices.len(), 8);
        assert_eq!(tris.edges.len(), 18);
        let merged = to_entities(
            &mesh,
            &MeshOptions {
                merge_coplanar: true,
                ..o
            },
            &[],
        );
        assert_eq!(merged.faces.len(), 6);
        assert_eq!(merged.edges.len(), 12);
        assert!(crate::validate(&merged, crate::Checks::ALL).is_empty());
        let size = merged
            .vertices
            .values()
            .map(|v| v.pos.max_element())
            .fold(0.0, f64::max);
        assert!((size - 10.0).abs() < 1e-3, "{size}");
    }

    #[test]
    fn ascii_stl_and_y_up() {
        let text = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let mesh = parse_stl(text.as_bytes()).unwrap();
        assert_eq!(mesh.polygons.len(), 1);
        let ents = to_entities(
            &mesh,
            &MeshOptions {
                y_up: true,
                ..opts(false)
            },
            &[],
        );
        // File Y (up) became Z.
        assert!(ents.vertices.values().any(|v| (v.pos.z - 1.0).abs() < 1e-9));
        assert!(parse_stl(b"garbage").is_err());
    }

    #[test]
    fn degenerate_and_duplicate_faces_are_skipped() {
        let obj = "v 0 0 0\nv 1 0 0\nv 2 0 0\nv 0 1 0\nf 1 2 3\nf 1 2 4\nf 2 4 1\nf 1 1 2\n";
        let mesh = parse_obj(obj, |_| None).unwrap();
        let ents = to_entities(&mesh, &opts(false), &[]);
        assert_eq!(ents.faces.len(), 1);
        assert!(parse_obj("f 1 2 3\n", |_| None).is_err());
    }
}
