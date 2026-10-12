//! Consistency checks, run after every operation in tests.

use std::collections::HashMap;

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{EdgeId, Entities, FaceId, Loop};
use crate::geom::{Plane, segment_segment, signed_area2};

/// Which checks to run. Files written by SketchUp satisfy the topology checks, but not
/// necessarily our geometric tolerances, so imports check topology only.
#[derive(Debug, Clone, Copy)]
pub struct Checks {
    pub geometry: bool,
}

impl Checks {
    pub const ALL: Self = Self { geometry: true };
    pub const TOPOLOGY: Self = Self { geometry: false };
}

/// Every violated invariant, as text. Empty means valid.
pub fn validate(ents: &Entities, checks: Checks) -> Vec<String> {
    let mut errs = Vec::new();
    let mut err = |s: String| errs.push(s);

    for (vid, v) in &ents.vertices {
        for &e in &v.edges {
            match ents.edges.get(e) {
                None => err(format!("vertex {vid:?} lists missing edge {e:?}")),
                Some(edge) if edge.start != vid && edge.end != vid => err(format!(
                    "vertex {vid:?} lists edge {e:?} that does not use it"
                )),
                _ => {}
            }
        }
    }

    for (eid, e) in &ents.edges {
        for v in [e.start, e.end] {
            match ents.vertices.get(v) {
                None => err(format!("edge {eid:?} uses missing vertex {v:?}")),
                Some(vert) if !vert.edges.contains(&eid) => {
                    err(format!("vertex {v:?} does not list its edge {eid:?}"))
                }
                _ => {}
            }
        }
        if e.start == e.end {
            err(format!("edge {eid:?} is a loop on one vertex"));
        }
        for &f in &e.faces {
            let used = ents.faces.get(f).is_some_and(|face| {
                face.loops()
                    .any(|l| l.coedges.iter().any(|c| c.edge == eid))
            });
            if !used {
                err(format!(
                    "edge {eid:?} lists face {f:?} that does not use it"
                ));
            }
        }
        if ents.edge_between(e.start, e.end) != Some(eid) {
            err(format!(
                "edge {eid:?} duplicates another edge between the same vertices"
            ));
        }
    }

    for (fid, face) in &ents.faces {
        if (face.normal.length() - 1.0).abs() > 1e-6 {
            err(format!("face {fid:?} normal is not unit length"));
        }
        for (i, l) in face.loops().enumerate() {
            check_loop(ents, fid, i, l, &mut err);
        }
        for l in face.loops() {
            for c in &l.coedges {
                if ents
                    .edges
                    .get(c.edge)
                    .is_some_and(|e| !e.faces.contains(&fid))
                {
                    err(format!("edge {:?} does not list face {fid:?}", c.edge));
                }
            }
        }
        if checks.geometry
            && face
                .loops()
                .all(|l| l.coedges.iter().all(|c| ents.edges.contains_key(c.edge)))
        {
            check_face_geometry(ents, fid, &mut err);
        }
    }

    // Two faces bounded by the same edges are one face twice. (Not a topology check:
    // some files written by SketchUp have such pairs.)
    let mut by_edges: std::collections::BTreeMap<Vec<EdgeId>, FaceId> =
        std::collections::BTreeMap::new();
    for (fid, face) in ents.faces.iter().filter(|_| checks.geometry) {
        let mut edges: Vec<EdgeId> = face
            .loops()
            .flat_map(|l| l.coedges.iter().map(|c| c.edge))
            .collect();
        edges.sort();
        edges.dedup();
        if let Some(g) = by_edges.insert(edges, fid) {
            err(format!("faces {g:?} and {fid:?} have the same edges"));
        }
    }

    for e in ents.index_errors() {
        err(e);
    }

    if checks.geometry {
        check_vertex_spacing(ents, &mut err);
        check_edge_crossings(ents, &mut err);
        check_face_overlap(ents, &mut err);
    }
    errs
}

fn check_loop(ents: &Entities, fid: FaceId, i: usize, l: &Loop, err: &mut impl FnMut(String)) {
    if l.coedges.len() < 3 {
        err(format!("face {fid:?} loop {i} has fewer than 3 edges"));
        return;
    }
    if l.coedges.iter().any(|c| !ents.edges.contains_key(c.edge)) {
        err(format!("face {fid:?} loop {i} uses a missing edge"));
        return;
    }
    for (k, c) in l.coedges.iter().enumerate() {
        let next = l.coedges[(k + 1) % l.coedges.len()];
        if ents.coedge_ends(*c).1 != ents.coedge_ends(next).0 {
            err(format!("face {fid:?} loop {i} is not closed at coedge {k}"));
        }
    }
}

fn check_face_geometry(ents: &Entities, fid: FaceId, err: &mut impl FnMut(String)) {
    let face = &ents.faces[fid];
    let plane = ents.face_plane(fid);
    for l in face.loops() {
        for p in ents.loop_points(l) {
            if !plane.contains(p) {
                err(format!("face {fid:?} is not planar"));
                return;
            }
        }
    }
    let proj = |l: &Loop| -> Vec<DVec2> {
        ents.loop_points(l)
            .into_iter()
            .map(|p| plane.project(p))
            .collect()
    };
    if signed_area2(&proj(&face.outer)) <= 0.0 {
        err(format!(
            "face {fid:?} outer loop does not run counter-clockwise around its normal"
        ));
    }
    for h in &face.holes {
        let poly = proj(h);
        if signed_area2(&poly) >= 0.0 {
            err(format!("face {fid:?} hole does not run clockwise"));
        }
        let outer = proj(&face.outer);
        if !crate::geom::point_in_polygon(crate::arrangement::interior_point(&outer, &[]), &outer) {
            err(format!("face {fid:?} outer loop has no interior"));
        }
    }
    let fitted = Plane::from_polygon(&ents.loop_points(&face.outer));
    if fitted.is_some_and(|p| p.normal.dot(face.normal) < 0.0) {
        err(format!(
            "face {fid:?} normal disagrees with its outer loop winding"
        ));
    }
}

/// Bounding boxes bucketed in a uniform grid, so the geometric checks only compare
/// things that are near each other. Boxes covering too many cells are kept apart
/// and returned by every query. Queries return indices in ascending order, so the
/// checks report problems in the same order as comparing every pair would.
struct Grid {
    cell: f64,
    cells: HashMap<[i64; 3], Vec<usize>>,
    big: Vec<usize>,
}

impl Grid {
    const MAX_CELLS: i64 = 64;

    fn new(boxes: &[(DVec3, DVec3)]) -> Self {
        let mut sizes: Vec<f64> = boxes.iter().map(|(a, b)| (*b - *a).max_element()).collect();
        sizes.sort_by(f64::total_cmp);
        let median = sizes.get(sizes.len() / 2).copied().unwrap_or(1.0);
        let mut grid = Self {
            cell: median.max(TOL * 8.0),
            cells: HashMap::new(),
            big: Vec::new(),
        };
        for (i, (a, b)) in boxes.iter().enumerate() {
            let (lo, hi) = (grid.key(*a), grid.key(*b));
            let n = (0..3).map(|k| hi[k] - lo[k] + 1).product::<i64>();
            if n > Self::MAX_CELLS {
                grid.big.push(i);
                continue;
            }
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        grid.cells.entry([x, y, z]).or_default().push(i);
                    }
                }
            }
        }
        grid
    }

    fn key(&self, p: DVec3) -> [i64; 3] {
        let k = (p / self.cell).floor();
        [k.x as i64, k.y as i64, k.z as i64]
    }

    /// Indices of boxes that may overlap `a..b`, ascending, without duplicates.
    fn query(&self, a: DVec3, b: DVec3) -> Vec<usize> {
        let (lo, hi) = (self.key(a), self.key(b));
        let mut out = self.big.clone();
        let n = (0..3).map(|k| hi[k] - lo[k] + 1).product::<i64>();
        if n > Self::MAX_CELLS * 64 {
            // A query box this large: walk the filled cells instead.
            for (k, v) in &self.cells {
                if (0..3).all(|d| lo[d] <= k[d] && k[d] <= hi[d]) {
                    out.extend(v);
                }
            }
        } else {
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        if let Some(v) = self.cells.get(&[x, y, z]) {
                            out.extend(v);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn overlaps(a: &(DVec3, DVec3), b: &(DVec3, DVec3)) -> bool {
    a.0.cmple(b.1).all() && b.0.cmple(a.1).all()
}

fn check_vertex_spacing(ents: &Entities, err: &mut impl FnMut(String)) {
    let vs: Vec<_> = ents.vertices.iter().collect();
    let pad = DVec3::splat(TOL);
    let boxes: Vec<_> = vs.iter().map(|(_, v)| (v.pos, v.pos)).collect();
    let grid = Grid::new(&boxes);
    for (i, (a, va)) in vs.iter().enumerate() {
        for j in grid.query(va.pos - pad, va.pos + pad) {
            let (b, vb) = vs[j];
            if j > i && va.pos.distance(vb.pos) <= TOL {
                err(format!("vertices {a:?} and {b:?} coincide"));
            }
        }
    }
}

fn check_edge_crossings(ents: &Entities, err: &mut impl FnMut(String)) {
    let es: Vec<_> = ents.edges.iter().collect();
    let vs: Vec<_> = ents.vertices.iter().collect();
    let pad = DVec3::splat(TOL);
    let boxes: Vec<(DVec3, DVec3)> = es
        .iter()
        .map(|(_, e)| {
            let (p, q) = (ents.pos(e.start), ents.pos(e.end));
            (p.min(q) - pad, p.max(q) + pad)
        })
        .collect();
    let edge_grid = Grid::new(&boxes);
    let vertex_grid = Grid::new(&vs.iter().map(|(_, v)| (v.pos, v.pos)).collect::<Vec<_>>());
    for (i, (a, ea)) in es.iter().enumerate() {
        let (p0, p1) = (ents.pos(ea.start), ents.pos(ea.end));
        for j in edge_grid.query(boxes[i].0, boxes[i].1) {
            if j <= i || !overlaps(&boxes[i], &boxes[j]) {
                continue;
            }
            let (b, eb) = es[j];
            let shared = [eb.start, eb.end]
                .iter()
                .any(|v| *v == ea.start || *v == ea.end);
            if shared {
                continue;
            }
            let (q0, q1) = (ents.pos(eb.start), ents.pos(eb.end));
            if let Some((d, _, _)) = segment_segment(p0, p1, q0, q1)
                && d <= TOL / 2.0
            {
                err(format!(
                    "edges {a:?} and {b:?} cross without a shared vertex"
                ));
            }
        }
        for k in vertex_grid.query(boxes[i].0, boxes[i].1) {
            let (vid, v) = vs[k];
            if vid == ea.start || vid == ea.end {
                continue;
            }
            let (d, _) = crate::geom::point_segment(v.pos, p0, p1);
            if d <= TOL / 2.0 {
                err(format!(
                    "vertex {vid:?} lies on edge {a:?} without splitting it"
                ));
            }
        }
    }
}

/// Coplanar faces never overlap: a point inside one is outside every other.
fn check_face_overlap(ents: &Entities, err: &mut impl FnMut(String)) {
    let samples: Vec<(FaceId, glam::DVec3)> = ents
        .faces
        .keys()
        .map(|f| {
            let plane = ents.face_plane(f);
            let (outer, holes) = ents.face_polygons(f, &plane);
            let holes: Vec<&Vec<DVec2>> = holes.iter().collect();
            let q = crate::arrangement::interior_point(&outer, &holes);
            let (u, v) = plane.basis();
            (f, u * q.x + v * q.y + plane.normal * plane.d)
        })
        .collect();
    let pad = DVec3::splat(TOL);
    let faces: Vec<FaceId> = ents.faces.keys().collect();
    let boxes: Vec<(DVec3, DVec3)> = faces
        .iter()
        .map(|&g| {
            let pts: Vec<DVec3> = ents.faces[g]
                .loops()
                .flat_map(|l| ents.loop_points(l))
                .collect();
            let lo = pts.iter().fold(DVec3::INFINITY, |m, p| m.min(*p));
            let hi = pts.iter().fold(DVec3::NEG_INFINITY, |m, p| m.max(*p));
            (lo - pad, hi + pad)
        })
        .collect();
    let grid = Grid::new(&boxes);
    for (f, p) in &samples {
        for k in grid.query(*p, *p) {
            let g = faces[k];
            // Coplanar only: faces may pass through each other (SketchUp does not
            // intersect them unless asked), and a sample on the line where two planes
            // cross is not an overlap.
            if g != *f
                && overlaps(&boxes[k], &(*p, *p))
                && ents.face_plane(g).coincides(&ents.face_plane(*f))
                && ents.face_contains(g, *p)
            {
                err(format!("faces {f:?} and {g:?} overlap"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid finds every box that overlaps a query, small, large or far apart.
    #[test]
    fn grid_queries_find_every_overlapping_box() {
        let mut seed = 7u64;
        let mut rnd = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut boxes = Vec::new();
        for i in 0..400 {
            let p = DVec3::new(rnd(), rnd(), rnd()) * 100.0;
            let size = if i % 50 == 0 { 80.0 } else { rnd() * 3.0 };
            boxes.push((p, p + DVec3::new(rnd(), rnd(), rnd()) * size));
        }
        let grid = Grid::new(&boxes);
        for q in boxes
            .iter()
            .chain(&[(DVec3::splat(-10.0), DVec3::splat(200.0))])
        {
            let found = grid.query(q.0, q.1);
            assert!(found.windows(2).all(|w| w[0] < w[1]));
            for (j, b) in boxes.iter().enumerate() {
                if overlaps(q, b) {
                    assert!(found.contains(&j), "missed box {j}");
                }
            }
        }
    }
}
