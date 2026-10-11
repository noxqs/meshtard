//! Sandbox terrain: SketchUp's From Scratch grid and From Contours TIN.
//!
//! Both build loose triangles meant to go into a new group. Edges between two
//! triangles that SketchUp hides are soft and smooth: the grid's diagonals (the grid
//! lines stay visible) and every interior edge of a contour TIN (its border stays
//! hard). Both from memory of SketchUp's Sandbox (unverified).

use std::collections::BTreeMap;

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{CoEdge, EdgeAttrs, EdgeId, Entities, Loop, VertexId};

/// Adds triangle `a b c` (counter-clockwise around its normal) between existing
/// vertices, reusing edges from `edges`. Degenerate triangles are skipped.
fn add_triangle(
    ents: &mut Entities,
    edges: &mut BTreeMap<(VertexId, VertexId), EdgeId>,
    [a, b, c]: [VertexId; 3],
) -> bool {
    let (pa, pb, pc) = (ents.pos(a), ents.pos(b), ents.pos(c));
    let n = (pb - pa).cross(pc - pa);
    if n.length() <= TOL * TOL {
        return false;
    }
    let coedges = [(a, b), (b, c), (c, a)]
        .into_iter()
        .map(|(s, t)| {
            let key = if s < t { (s, t) } else { (t, s) };
            let e = *edges
                .entry(key)
                .or_insert_with(|| ents.add_raw_edge(s, t, EdgeAttrs::default()));
            CoEdge {
                edge: e,
                reversed: ents.edges[e].start != s,
            }
        })
        .collect();
    ents.add_raw_face(
        Loop { coedges },
        Vec::new(),
        n.normalize(),
        Default::default(),
    );
    true
}

fn soften(ents: &mut Entities, e: EdgeId) {
    let a = &mut ents.edges[e].attrs;
    a.soft = true;
    a.smooth = true;
}

/// Sandbox From Scratch: a grid of `nu` x `nv` squares from `origin` with steps `u`
/// and `v` (one grid spacing long each), every square split into two triangles along
/// the diagonal from its first corner. Faces look towards `u x v` flipped up when it
/// points down; diagonals are soft and smooth.
pub fn grid(origin: DVec3, u: DVec3, v: DVec3, nu: usize, nv: usize) -> Entities {
    let mut ents = Entities::new();
    if nu == 0 || nv == 0 {
        return ents;
    }
    let flip = u.cross(v).z < 0.0;
    let mut verts = Vec::with_capacity((nu + 1) * (nv + 1));
    for j in 0..=nv {
        for i in 0..=nu {
            verts.push(ents.add_vertex(origin + u * i as f64 + v * j as f64));
        }
    }
    let at = |i: usize, j: usize| verts[j * (nu + 1) + i];
    let mut edges = BTreeMap::new();
    for j in 0..nv {
        for i in 0..nu {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            for t in [[a, b, c], [a, c, d]] {
                let t = if flip { [t[0], t[2], t[1]] } else { t };
                add_triangle(&mut ents, &mut edges, t);
            }
            let key = if a < c { (a, c) } else { (c, a) };
            if let Some(&e) = edges.get(&key) {
                soften(&mut ents, e);
            }
        }
    }
    ents
}

/// Sandbox From Contours: a triangulated irregular network through `points` (the
/// contour lines' vertices), Delaunay in plan (x, y). Points closer than the tolerance
/// in plan to an earlier one are dropped. Faces look up; edges between two faces are
/// soft and smooth, the border stays hard.
pub fn tin(points: &[DVec3]) -> Entities {
    let plan: Vec<DVec2> = points.iter().map(|p| p.truncate()).collect();
    let tris = crate::delaunay::triangulate(&plan, TOL);
    let mut ents = Entities::new();
    let mut vmap: BTreeMap<usize, VertexId> = BTreeMap::new();
    let mut edges = BTreeMap::new();
    for t in tris {
        let vs = t.map(|i| *vmap.entry(i).or_insert_with(|| ents.add_vertex(points[i])));
        add_triangle(&mut ents, &mut edges, vs);
    }
    let interior: Vec<EdgeId> = ents
        .edges
        .iter()
        .filter(|(_, e)| e.faces.len() == 2)
        .map(|(id, _)| id)
        .collect();
    for e in interior {
        soften(&mut ents, e);
    }
    ents
}

/// The points of contour edges: every vertex of `edges` once.
pub fn contour_points(ents: &Entities, edges: &[EdgeId]) -> Vec<DVec3> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for &e in edges {
        let Some(edge) = ents.edges.get(e) else {
            continue;
        };
        for v in [edge.start, edge.end] {
            if seen.insert(v) {
                out.push(ents.pos(v));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Checks, validate};

    #[test]
    fn grid_counts_and_soft_diagonals() {
        let g = grid(DVec3::ZERO, DVec3::X * 120.0, DVec3::Y * 120.0, 4, 3);
        assert_eq!(g.faces.len(), 2 * 4 * 3);
        assert_eq!(g.vertices.len(), 5 * 4);
        // Grid lines plus one diagonal per square.
        assert_eq!(g.edges.len(), 4 * 4 + 3 * 5 + 12);
        let soft = g.edges.values().filter(|e| e.attrs.soft && e.attrs.smooth);
        assert_eq!(soft.count(), 12);
        assert!(g.faces.values().all(|f| f.normal.z > 0.99));
        assert_eq!(validate(&g, Checks::ALL), Vec::<String>::new());
        // Drawn the other way round the faces still look up.
        let g = grid(DVec3::ZERO, DVec3::Y * 10.0, DVec3::X * 10.0, 2, 2);
        assert!(g.faces.values().all(|f| f.normal.z > 0.99));
        assert_eq!(validate(&g, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn contour_tin_spans_levels() {
        // Three nested square contours at heights 0, 10, 20 (a stepped hill).
        let mut pts = Vec::new();
        for (k, half) in [(0.0, 50.0), (1.0, 30.0), (2.0, 10.0)] {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                pts.push(DVec3::new(x * half, y * half, k * 10.0));
            }
        }
        // The same corner twice (contours touching) is used once.
        pts.push(pts[0]);
        let t = tin(&pts);
        // 12 distinct points with a hull of 4: 2n - 2 - h triangles.
        assert_eq!(t.faces.len(), 2 * 12 - 2 - 4);
        assert_eq!(t.vertices.len(), 12);
        assert!(t.faces.values().all(|f| f.normal.z > 0.0));
        let hard: Vec<_> = t.edges.values().filter(|e| !e.attrs.soft).collect();
        assert_eq!(hard.len(), 4, "only the hull stays hard");
        assert!(hard.iter().all(|e| e.faces.len() == 1));
        let top = t
            .vertices
            .values()
            .map(|v| v.pos.z)
            .fold(f64::MIN, f64::max);
        assert_eq!(top, 20.0);
        assert_eq!(validate(&t, Checks::ALL), Vec::<String>::new());
    }
}
