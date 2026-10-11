//! SketchUp's Intersect Faces.

use glam::DVec3;

use crate::TOL;
use crate::entities::{EdgeId, Entities, FaceId};
use crate::geom::point_segment;
use crate::offset::EdgeSnapshot;
use crate::ops::add_line;

/// SketchUp's Intersect Faces: adds edges where each face in `faces` crosses each face in
/// `with` (non-coplanar pairs; the intersection segment clipped to both faces, holes
/// respected), splitting faces as add_line does. Returns the new edges.
///
/// All segments are computed from the faces as they are before anything is added, then
/// added with `add_line`, so faces split by one segment are still cut by the others.
pub fn intersect_faces(ents: &mut Entities, faces: &[FaceId], with: &[FaceId]) -> Vec<EdgeId> {
    let mut segments: Vec<(DVec3, DVec3)> = Vec::new();
    for &f in faces {
        for &g in with {
            if f == g || !ents.faces.contains_key(f) || !ents.faces.contains_key(g) {
                continue;
            }
            for seg in face_face(ents, f, g) {
                let dup = segments.iter().any(|&(a, b)| {
                    (a.distance(seg.0) <= TOL && b.distance(seg.1) <= TOL)
                        || (a.distance(seg.1) <= TOL && b.distance(seg.0) <= TOL)
                });
                if !dup {
                    segments.push(seg);
                }
            }
        }
    }
    let before = EdgeSnapshot::near(ents, &segments);
    for &(a, b) in &segments {
        add_line(ents, a, b);
    }
    before.added_on(ents, &segments)
}

/// Segments where faces `f` and `g` cross: their planes' line clipped to both regions.
fn face_face(ents: &Entities, f: FaceId, g: FaceId) -> Vec<(DVec3, DVec3)> {
    let (pf, pg) = (ents.face_plane(f), ents.face_plane(g));
    let dir = pf.normal.cross(pg.normal);
    let len2 = dir.length_squared();
    if len2 < 1e-12 {
        return Vec::new(); // parallel or coplanar
    }
    let origin = (pf.normal.cross(dir) * -pg.d + pg.normal.cross(dir) * pf.d) / len2;
    let dir = dir / len2.sqrt();
    let a = clip(ents, f, origin, dir);
    if a.is_empty() {
        return Vec::new();
    }
    let b = clip(ents, g, origin, dir);
    let mut out = Vec::new();
    for &(a0, a1) in &a {
        for &(b0, b1) in &b {
            let (s0, s1) = (a0.max(b0), a1.min(b1));
            if s1 - s0 > TOL {
                out.push((origin + dir * s0, origin + dir * s1));
            }
        }
    }
    out
}

/// Parameter intervals of the line `origin + s * dir` (lying in the face's plane) that
/// are inside the face or on its boundary.
fn clip(ents: &Entities, f: FaceId, origin: DVec3, dir: DVec3) -> Vec<(f64, f64)> {
    let face = &ents.faces[f];
    let side = face.normal.cross(dir);
    let loops: Vec<Vec<DVec3>> = face.loops().map(|l| ents.loop_points(l)).collect();
    let param = |p: DVec3| (p - origin).dot(dir);

    let mut params: Vec<f64> = Vec::new();
    for pts in &loops {
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            let (sa, sb) = ((a - origin).dot(side), (b - origin).dot(side));
            if sa.abs() <= TOL {
                params.push(param(a));
            }
            if (sa > TOL && sb < -TOL) || (sa < -TOL && sb > TOL) {
                let t = sa / (sa - sb);
                params.push(param(a + (b - a) * t));
            }
        }
    }
    params.sort_by(f64::total_cmp);
    params.dedup_by(|x, y| (*x - *y).abs() <= TOL);

    let on_boundary = |p: DVec3| {
        loops.iter().any(|pts| {
            (0..pts.len()).any(|k| point_segment(p, pts[k], pts[(k + 1) % pts.len()]).0 <= TOL)
        })
    };
    let mut out: Vec<(f64, f64)> = Vec::new();
    for w in params.windows(2) {
        let mid = origin + dir * ((w[0] + w[1]) / 2.0);
        if !(ents.face_contains(f, mid) || on_boundary(mid)) {
            continue;
        }
        match out.last_mut() {
            Some(last) if (last.1 - w[0]).abs() <= TOL => last.1 = w[1],
            _ => out.push((w[0], w[1])),
        }
    }
    out
}
