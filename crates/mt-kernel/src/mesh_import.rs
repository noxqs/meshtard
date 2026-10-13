//! File > Import of triangle and polygon meshes: Wavefront OBJ (with MTL colors) and
//! STL (binary or ASCII), turned into kernel faces.
//!
//! - The mesh is healed into valid geometry ([`crate::mesh_heal`]): vertices closer
//!   than the kernel tolerance are welded, sides are split where a vertex lies on them
//!   or they cross, coplanar overlaps become their arrangement's cells. Edges shared by
//!   faces are shared. Degenerate polygons, slivers and repeated faces are skipped;
//!   non-planar polygons and polygons that pass a vertex twice (a hole joined to the
//!   outline by a bridge edge) are ear-clipped into triangles (merging joins the
//!   planar ones again).
//! - "Merge coplanar faces" (SketchUp's STL importer option, from memory) joins
//!   neighboring coplanar faces of the same material into one face, removing the edges
//!   between them; a region that would not give one simple outer loop is left as is.
//! - OBJ groups, objects, smoothing groups and normals are ignored; MTL diffuse colors
//!   and opacity become materials. Texture coordinates and `map_Kd` textures are added
//!   by [`crate::formats::obj`].
//! - Polygons with texture coordinates whose material has a texture get a face texture
//!   position fitted to them ([`uv_from_corners`]); merging keeps faces with different
//!   positions apart.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{
    CoEdge, EdgeAttrs, EdgeId, Entities, FaceAttrs, FaceId, Loop, MaterialId, VertexId,
};
use crate::geom::{Plane, signed_area2};
use crate::mesh_heal::{self, Welder};

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
    /// Texture coordinates per vertex (repeats, v up), empty when the file has none.
    /// Used when the material has a texture.
    pub uvs: Vec<DVec2>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshMaterial {
    pub name: String,
    /// sRGB.
    pub color: [u8; 3],
    pub opacity: f64,
    /// A texture image.
    pub texture: Option<MeshTexture>,
}

/// A mesh material's texture image.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshTexture {
    pub filename: String,
    /// Image file bytes (PNG or JPEG).
    pub data: Arc<[u8]>,
    /// Tile size in inches given to the model material (faces get their own
    /// mapping from the file's texture coordinates).
    pub size: [f64; 2],
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
                texture: None,
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
                    uvs: Vec::new(),
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
                            texture: None,
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
                uvs: Vec::new(),
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
                    uvs: Vec::new(),
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

/// Largest coordinate (inches) an imported vertex may have: about 25 000 km.
pub const MAX_COORD: f64 = 1e12;

/// Builds faces from `mesh` in a new collection. `materials` maps the mesh's materials
/// to model materials. The mesh is healed into valid geometry first
/// ([`crate::mesh_heal`]): welded, split where sides cross or touch, and with coplanar
/// overlaps resolved.
pub fn to_entities(mesh: &Mesh, opts: &MeshOptions, materials: &[MaterialId]) -> Entities {
    let place = |i: usize| -> DVec3 {
        let p = mesh.positions[i];
        let p = if opts.y_up {
            DVec3::new(p.x, -p.z, p.y)
        } else {
            p
        };
        p * opts.inches_per_unit
    };
    let mut welder = Welder::default();
    let mut welded: Vec<Option<usize>> = vec![None; mesh.positions.len()];
    let mut pieces = Vec::new();
    // Damaged files: polygons naming a missing vertex, or one at an infinite, NaN or
    // absurd position (which the healing's grids cannot hold), are left out.
    let usable = |i: usize| {
        i < mesh.positions.len() && {
            let p = place(i);
            p.is_finite() && p.abs().max_element() <= MAX_COORD
        }
    };
    for (src, poly) in mesh.polygons.iter().enumerate() {
        if !poly.vertices.iter().all(|&i| usable(i)) {
            continue;
        }
        let vs: Vec<usize> = poly
            .vertices
            .iter()
            .map(|&i| *welded[i].get_or_insert_with(|| welder.weld(place(i))))
            .collect();
        pieces.extend(mesh_heal::pieces(&welder.points, &vs, src));
    }
    let (pieces, _) = mesh_heal::split_sides(&mut welder, pieces);

    let mut ents = Entities::new();
    let mut ids: Vec<Option<VertexId>> = vec![None; welder.points.len()];
    let mut seen: BTreeSet<Vec<VertexId>> = BTreeSet::new();
    for piece in &pieces {
        let vs: Vec<VertexId> = piece
            .vs
            .iter()
            .map(|&i| *ids[i].get_or_insert_with(|| ents.add_vertex(welder.points[i])))
            .collect();
        let poly = &mesh.polygons[piece.src];
        let material = poly.material.and_then(|m| materials.get(m).copied());
        let face = add_face(&mut ents, &vs, piece.normal, material, &mut seen);
        // Texture coordinates, when the polygon has them and its material a texture.
        let tile = poly
            .material
            .and_then(|m| mesh.materials.get(m))
            .and_then(|m| m.texture.as_ref())
            .map(|t| t.size);
        if let (Some(f), Some(tile)) = (face, tile)
            && poly.uvs.len() == poly.vertices.len()
        {
            let corners: Vec<(DVec3, DVec2)> = poly
                .vertices
                .iter()
                .zip(&poly.uvs)
                .map(|(&i, &t)| (place(i), t))
                .collect();
            if let Some(m) = uv_from_corners(ents.faces[f].normal, &corners, tile) {
                ents.faces[f].attrs.uv = crate::FaceUv::boxed(Some(m), None);
            }
        }
    }
    mesh_heal::resolve_overlaps(&mut ents);
    if opts.merge_coplanar {
        let merged = merge_coplanar(&mut ents);
        // A merged face takes its first face's plane, which can make it coplanar with
        // a face it overlaps (the other side of a solid thinner than the tolerance):
        // resolve those overlaps too, and merge their cells.
        if mesh_heal::resolve_overlaps_touching(&mut ents, Some(&merged)) > 0 {
            merge_coplanar(&mut ents);
        }
        // The two sides of a sliver can merge into faces over the same edges.
        mesh_heal::drop_repeated_faces(&mut ents);
    }
    ents
}

/// Triangles (corner indices, counter-clockwise around `plane`'s normal) of a
/// polygon by ear clipping in `plane`, which handles concave polygons and outlines
/// that visit a vertex twice; a fan when ear clipping fails.
pub(crate) fn polygon_triangles(points: &[DVec3], plane: &Plane) -> Vec<[usize; 3]> {
    let flat: Vec<f64> = points
        .iter()
        .flat_map(|&p| plane.project(p).to_array())
        .collect();
    let fan = || (1..points.len() - 1).map(|k| [0, k, k + 1]).collect();
    let Ok(idx) = crate::geom::earcut(&flat, &[]) else {
        return fan();
    };
    if idx.is_empty() {
        return fan();
    }
    idx.as_chunks::<3>()
        .0
        .iter()
        .map(|&[a, b, c]| {
            let n = (points[b] - points[a]).cross(points[c] - points[a]);
            if n.dot(plane.normal) < 0.0 {
                [a, c, b]
            } else {
                [a, b, c]
            }
        })
        .collect()
}

/// Adds a face through `vs` (counter-clockwise around `normal`), reusing edges.
/// A face over the same vertices as one added before is skipped.
fn add_face(
    ents: &mut Entities,
    vs: &[VertexId],
    normal: DVec3,
    material: Option<MaterialId>,
    seen: &mut BTreeSet<Vec<VertexId>>,
) -> Option<FaceId> {
    // Area check in the plane (collinear points pass Newell with a tiny normal).
    let plane = Plane::new(normal, ents.pos(vs[0]));
    let poly: Vec<_> = vs.iter().map(|&v| plane.project(ents.pos(v))).collect();
    if signed_area2(&poly).abs() < TOL * TOL {
        return None;
    }
    let mut key = vs.to_vec();
    key.sort();
    if !seen.insert(key) {
        return None;
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
    Some(ents.add_raw_face(
        Loop { coedges },
        Vec::new(),
        normal,
        FaceAttrs {
            material,
            ..Default::default()
        },
    ))
}

/// The texture-to-plane matrix of a face with unit normal `normal` that puts the
/// texture points `t` (repeats) of `corners` on their positions, for a texture of tile
/// size `tile` (inches): an affine map through the first three corners that are not
/// collinear. `None` when there are none or the texture points are degenerate.
pub fn uv_from_corners(
    normal: DVec3,
    corners: &[(DVec3, DVec2)],
    tile: [f64; 2],
) -> Option<[f64; 9]> {
    let (xr, yr) = mt_skp::uv::face_uv_basis(normal.to_array());
    let (xr, yr) = (DVec3::from_array(xr), DVec3::from_array(yr));
    let size = |s: f64| if s > 1e-9 { s } else { 1.0 };
    let tile = DVec2::new(size(tile[0]), size(tile[1]));
    let plane = |p: DVec3| DVec2::new(p.dot(xr), p.dot(yr));
    let (p0, t0) = *corners.first()?;
    for i in 1..corners.len() {
        for j in i + 1..corners.len() {
            let (dp1, dp2) = (
                plane(corners[i].0) - plane(p0),
                plane(corners[j].0) - plane(p0),
            );
            if dp1.perp_dot(dp2).abs() < 1e-9 * dp1.length_squared().max(1e-12) {
                continue;
            }
            // Texture units (repeats times tile) to plane: [dP] = [dT] * A.
            let (dt1, dt2) = ((corners[i].1 - t0) * tile, (corners[j].1 - t0) * tile);
            let det = dt1.perp_dot(dt2);
            if det.abs() < 1e-12 * tile.x * tile.y {
                return None;
            }
            // inverse of [[dt1.x dt1.y] [dt2.x dt2.y]]
            let inv = [dt2.y / det, -dt1.y / det, -dt2.x / det, dt1.x / det];
            let r0 = DVec2::new(
                inv[0] * dp1.x + inv[1] * dp2.x,
                inv[0] * dp1.y + inv[1] * dp2.y,
            );
            let r1 = DVec2::new(
                inv[2] * dp1.x + inv[3] * dp2.x,
                inv[2] * dp1.y + inv[3] * dp2.y,
            );
            let t0u = t0 * tile;
            let off = plane(p0) - (r0 * t0u.x + r1 * t0u.y);
            let m = [r0.x, r0.y, 0.0, r1.x, r1.y, 0.0, off.x, off.y, 1.0];
            return m.iter().all(|x| x.is_finite()).then_some(m);
        }
    }
    None
}

/// Whether two faces' texture positions are the same (within rounding).
fn same_uv(a: &Option<Box<crate::FaceUv>>, b: &Option<Box<crate::FaceUv>>) -> bool {
    let close = |x: &Option<[f64; 9]>, y: &Option<[f64; 9]>| match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => x
            .iter()
            .zip(y)
            .all(|(a, b)| (a - b).abs() <= 1e-6 * (1.0 + a.abs().max(b.abs()))),
        _ => false,
    };
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => close(&a.front, &b.front) && close(&a.back, &b.back),
        _ => false,
    }
}

/// Joins each connected region of coplanar faces with the same material into one face.
/// Returns the faces made.
fn merge_coplanar(ents: &mut Entities) -> BTreeSet<FaceId> {
    let mut merged = BTreeSet::new();
    let faces: Vec<FaceId> = ents.faces.keys().collect();
    let mut done: BTreeSet<FaceId> = BTreeSet::new();
    for seed in faces {
        if done.contains(&seed) || !ents.faces.contains_key(seed) {
            continue;
        }
        let plane = ents.face_plane(seed);
        let material = ents.faces[seed].attrs.material;
        let uv = ents.faces[seed].attrs.uv.clone();
        // Flood across edges shared by exactly two faces of the region.
        let mut region = vec![seed];
        let mut members: BTreeSet<FaceId> = BTreeSet::from([seed]);
        let mut k = 0;
        while k < region.len() {
            let f = region[k];
            k += 1;
            for c in ents.faces[f].loops().flat_map(|l| &l.coedges) {
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
                    && same_uv(&gf.attrs.uv, &uv)
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
            merged.extend(merge_region(ents, &region, &members, plane));
        }
    }
    merged
}

/// Replaces the faces of `region` by one face bounded by the region's boundary.
fn merge_region(
    ents: &mut Entities,
    region: &[FaceId],
    members: &BTreeSet<FaceId>,
    plane: Plane,
) -> Option<FaceId> {
    let interior = |ents: &Entities, e| {
        let faces = &ents.edges[e].faces;
        faces.len() == 2 && faces.iter().all(|f| members.contains(f))
    };
    // Boundary coedges in loop direction, by start vertex.
    let mut next: BTreeMap<VertexId, Vec<CoEdge>> = BTreeMap::new();
    let mut count = 0;
    let mut boundary: BTreeSet<EdgeId> = BTreeSet::new();
    for &f in region {
        for c in ents.faces[f].loops().flat_map(|l| &l.coedges) {
            if !interior(ents, c.edge) {
                if !boundary.insert(c.edge) {
                    // Two faces of the region meet at an edge a third face uses too:
                    // the region would run along it both ways.
                    return None;
                }
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
                return None; // Open boundary: leave the region alone.
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
        return None;
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
        return None; // Pinched or several regions: keep the triangles.
    };
    // The merged face's plane goes through its first corner: every corner must be
    // in it, which faces each within the tolerance of the region's plane may not be.
    let normal = ents.faces[region[0]].normal;
    let first = Plane::new(
        normal,
        ents.pos(ents.coedge_ends(loops[outer].coedges[0]).0),
    );
    if !loops
        .iter()
        .all(|l| ents.loop_points(l).iter().all(|&p| first.contains(p)))
    {
        return None;
    }
    let attrs = ents.faces[region[0]].attrs.clone();
    let mut interior_edges: BTreeSet<_> = BTreeSet::new();
    for &f in region {
        for c in ents.faces[f].loops().flat_map(|l| &l.coedges) {
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
    Some(ents.add_raw_face(outer_loop, loops, normal, attrs))
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
