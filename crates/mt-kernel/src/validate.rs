//! Consistency checks, run after every operation in tests.

use glam::DVec2;

use crate::TOL;
use crate::entities::{Entities, FaceId, Loop};
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

fn check_vertex_spacing(ents: &Entities, err: &mut impl FnMut(String)) {
    let vs: Vec<_> = ents.vertices.iter().collect();
    for (i, (a, va)) in vs.iter().enumerate() {
        for (b, vb) in &vs[i + 1..] {
            if va.pos.distance(vb.pos) <= TOL {
                err(format!("vertices {a:?} and {b:?} coincide"));
            }
        }
    }
}

fn check_edge_crossings(ents: &Entities, err: &mut impl FnMut(String)) {
    let es: Vec<_> = ents.edges.iter().collect();
    for (i, (a, ea)) in es.iter().enumerate() {
        let (p0, p1) = (ents.pos(ea.start), ents.pos(ea.end));
        for (b, eb) in &es[i + 1..] {
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
        for (vid, v) in &ents.vertices {
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
    for (f, p) in &samples {
        for g in ents.faces.keys() {
            // Coplanar only: faces may pass through each other (SketchUp does not
            // intersect them unless asked), and a sample on the line where two planes
            // cross is not an overlap.
            if g != *f
                && ents.face_plane(g).coincides(&ents.face_plane(*f))
                && ents.face_contains(g, *p)
            {
                err(format!("faces {f:?} and {g:?} overlap"));
            }
        }
    }
}
