//! SketchUp's Follow Me tool.

use std::collections::HashSet;

use glam::DVec3;

use crate::TOL;
use crate::arrangement::interior_point;
use crate::entities::{EdgeId, Entities, FaceId, VertexId};
use crate::geom::{Plane, point_in_polygon, point_segment};
use crate::ops::{add_line, erase_edge};

/// Joints sharper than this keep hard edges; gentler ones are softened and smoothed.
const SMOOTH_ANGLE: f64 = 30.0;

/// SketchUp's Follow Me: sweeps the profile face along the path (a connected chain of
/// edges, in order, possibly closed). The profile is mitered at every path joint; side
/// faces are created, the profile face is kept at the start and a copy closes the end
/// (a closed path yields a closed ring with no caps). The profile need not touch the
/// path's start; the sweep follows the path's shape from where the profile is (SketchUp
/// moves along the path relative to the profile). Edges between faces generated from
/// consecutive path segments are softened and smoothed when the joint angle is small
/// (< 30 deg), like SketchUp does for curved paths; so are the edges swept from
/// vertices inside a curve of the profile (a circle profile gives a smooth tube).
/// Returns the new faces.
///
/// Each profile point moves along a path segment's direction until it meets the miter
/// plane of the next joint (the plane bisecting the two segments), so side faces are
/// planar. An open path ends with a cap perpendicular to its last segment. Returns
/// nothing and changes nothing when the path is not a connected chain, doubles back
/// on itself, or runs parallel to the profile's plane at its start.
///
/// Differences from SketchUp: miter lines stay on flat faces that continue across a
/// joint (SketchUp keeps those too on most models, unverified); the end cap is not a
/// curve when the profile was one; on a closed non-planar path the ring may not close
/// exactly, so the last segment's side faces may be missing.
pub fn follow_me(ents: &mut Entities, profile: FaceId, path: &[EdgeId]) -> Vec<FaceId> {
    let Some(face) = ents.faces.get(profile) else {
        return Vec::new();
    };
    let Some((points, closed)) = path_points(ents, path) else {
        return Vec::new();
    };
    let n_seg = points.len() - 1;
    let dirs: Vec<DVec3> = points
        .windows(2)
        .map(|w| (w[1] - w[0]).normalize())
        .collect();
    let profile_plane = ents.face_plane(profile);
    if profile_plane.normal.dot(dirs[0]).abs() < 1e-3 {
        return Vec::new(); // profile plane along the path: nothing to sweep
    }

    // Miter plane at each joint: through the joint, normal along the bisector.
    let miter = |i: usize| -> Option<Plane> {
        let n = dirs[(i + n_seg - 1) % n_seg] + dirs[i % n_seg];
        (n.length() > 1e-6).then(|| Plane::new(n, points[i]))
    };
    let mut planes: Vec<Plane> = Vec::with_capacity(n_seg + 1);
    for i in 0..=n_seg {
        let plane = if closed {
            miter(i % n_seg)
        } else if i == 0 {
            Some(profile_plane)
        } else if i == n_seg {
            Some(Plane::new(dirs[n_seg - 1], points[n_seg]))
        } else {
            miter(i)
        };
        match plane {
            Some(p) => planes.push(p),
            None => return Vec::new(), // the path doubles back
        }
    }

    // Profile loops and which of their vertices sit inside a curve.
    let loops: Vec<Vec<DVec3>> = face.loops().map(|l| ents.loop_points(l)).collect();
    let smooth_vertex: Vec<Vec<bool>> = face
        .loops()
        .map(|l| {
            let m = l.coedges.len();
            (0..m)
                .map(|k| {
                    let curve = |c: usize| ents.edges[l.coedges[c].edge].attrs.curve;
                    let (a, b) = (curve((k + m - 1) % m), curve(k));
                    a.is_some() && a == b
                })
                .collect()
        })
        .collect();
    let profile_edges: Vec<EdgeId> = face
        .loops()
        .flat_map(|l| l.coedges.iter().map(|c| c.edge))
        .collect();

    // Sections: the profile carried from plane to plane along the segments.
    let project = |pts: &[Vec<DVec3>], d: DVec3, plane: &Plane| -> Vec<Vec<DVec3>> {
        pts.iter()
            .map(|l| {
                l.iter()
                    .map(|&p| p - d * (plane.distance(p) / plane.normal.dot(d)))
                    .collect()
            })
            .collect()
    };
    let mut sections: Vec<Vec<Vec<DVec3>>> = Vec::with_capacity(n_seg + 1);
    sections.push(if closed {
        project(&loops, dirs[0], &planes[0])
    } else {
        loops.clone()
    });
    for i in 1..=n_seg {
        if closed && i == n_seg {
            sections.push(sections[0].clone());
        } else {
            let next = project(&sections[i - 1], dirs[i - 1], &planes[i]);
            sections.push(next);
        }
    }
    // Each section point must lie ahead of the previous one along the segment.
    for i in 0..n_seg {
        for (l0, l1) in sections[i].iter().zip(&sections[i + 1]) {
            if l0
                .iter()
                .zip(l1)
                .any(|(a, b)| (*b - *a).dot(dirs[i]) <= TOL)
            {
                return Vec::new();
            }
        }
    }

    let faces_before: HashSet<FaceId> = ents.faces.keys().collect();
    let mut soft: Vec<(DVec3, DVec3)> = Vec::new();
    let add_section = |ents: &mut Entities, s: &[Vec<DVec3>]| {
        for l in s {
            for k in 0..l.len() {
                add_line(ents, l[k], l[(k + 1) % l.len()]);
            }
        }
    };
    add_section(ents, &sections[0]);
    for i in 0..n_seg {
        for (li, (l0, l1)) in sections[i].iter().zip(&sections[i + 1]).enumerate() {
            for k in 0..l0.len() {
                add_line(ents, l0[k], l1[k]);
                if smooth_vertex[li][k] {
                    soft.push((l0[k], l1[k]));
                }
            }
        }
        if !(closed && i + 1 == n_seg) {
            add_section(ents, &sections[i + 1]);
        }
    }
    // Section edges at gentle joints.
    for i in 0..=n_seg {
        let internal = if closed {
            i < n_seg
        } else {
            i > 0 && i < n_seg
        };
        if !internal {
            continue;
        }
        let (a, b) = (dirs[(i + n_seg - 1) % n_seg], dirs[i % n_seg]);
        if a.angle_between(b).to_degrees() < SMOOTH_ANGLE {
            for l in &sections[i] {
                for k in 0..l.len() {
                    soft.push((l[k], l[(k + 1) % l.len()]));
                }
            }
        }
    }

    // Faces auto-face made inside the solid: across each internal joint, and inside the
    // holes of the end cap.
    let new_faces: Vec<FaceId> = ents
        .faces
        .keys()
        .filter(|f| !faces_before.contains(f))
        .collect();
    for f in new_faces {
        let plane = ents.face_plane(f);
        let internal = (0..=n_seg).any(|i| {
            let is_joint = if closed {
                i < n_seg
            } else {
                i > 0 && i < n_seg
            };
            is_joint && plane.coincides(&planes[i])
        });
        let in_cap_hole = !closed
            && plane.coincides(&planes[n_seg])
            && !inside_section(&planes[n_seg], &sections[n_seg], face_sample(ents, f));
        if internal || in_cap_hole {
            ents.remove_face(f);
        }
    }
    if closed && ents.faces.contains_key(profile) {
        ents.remove_face(profile);
        for e in profile_edges {
            if ents.edges.get(e).is_some_and(|x| x.faces.is_empty()) {
                let curve = ents.edges[e].attrs.curve;
                erase_edge(ents, e);
                if let Some(c) = curve
                    && ents.curve_edges(c).is_empty()
                {
                    ents.curves.remove(c);
                }
            }
        }
    }

    for (a, b) in soft {
        let ids: Vec<EdgeId> = ents
            .edges
            .iter()
            .filter(|(_, e)| {
                point_segment(ents.pos(e.start), a, b).0 <= TOL
                    && point_segment(ents.pos(e.end), a, b).0 <= TOL
            })
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            let attrs = &mut ents.edges[id].attrs;
            attrs.soft = true;
            attrs.smooth = true;
        }
    }

    ents.faces
        .keys()
        .filter(|f| !faces_before.contains(f))
        .collect()
}

/// Ordered points of a path given as connected edges, and whether it is closed.
/// Collinear joints and repeated points are dropped. `None` if the edges are not a chain.
fn path_points(ents: &Entities, path: &[EdgeId]) -> Option<(Vec<DVec3>, bool)> {
    let edge = |e: EdgeId| ents.edges.get(e);
    let first = edge(*path.first()?)?;
    let mut verts: Vec<VertexId> = match path.get(1) {
        None => vec![first.start, first.end],
        Some(&e2) => {
            let second = edge(e2)?;
            if first.end == second.start || first.end == second.end {
                vec![first.start, first.end]
            } else if first.start == second.start || first.start == second.end {
                vec![first.end, first.start]
            } else {
                return None;
            }
        }
    };
    for &e in &path[1..] {
        let x = edge(e)?;
        let last = *verts.last()?;
        if x.start == last {
            verts.push(x.end);
        } else if x.end == last {
            verts.push(x.start);
        } else {
            return None;
        }
    }
    let closed = path.len() >= 3 && verts.first() == verts.last();
    let mut pts: Vec<DVec3> = Vec::new();
    for v in verts {
        let p = ents.pos(v);
        if pts.last().is_none_or(|q: &DVec3| q.distance(p) > TOL) {
            pts.push(p);
        }
    }
    // Drop joints where the path goes straight on.
    let straight =
        |a: DVec3, b: DVec3, c: DVec3| (b - a).normalize().dot((c - b).normalize()) > 1.0 - 1e-9;
    let mut out: Vec<DVec3> = Vec::new();
    for (i, &p) in pts.iter().enumerate() {
        if i > 0 && i + 1 < pts.len() && straight(*out.last()?, p, pts[i + 1]) {
            continue;
        }
        out.push(p);
    }
    if closed {
        // The start joint of a closed path may be straight too: start at the next one.
        out.pop();
        let m = out.len();
        if m >= 3 && straight(out[m - 1], out[0], out[1]) {
            out.remove(0);
        }
        if out.len() < 3 {
            return None;
        }
        out.push(out[0]);
    }
    (out.len() >= 2).then_some((out, closed))
}

fn face_sample(ents: &Entities, f: FaceId) -> DVec3 {
    let plane = ents.face_plane(f);
    let (outer, holes) = ents.face_polygons(f, &plane);
    let holes: Vec<&Vec<_>> = holes.iter().collect();
    let q = interior_point(&outer, &holes);
    let (u, v) = plane.basis();
    u * q.x + v * q.y + plane.normal * plane.d
}

/// Whether `p` is inside a section's outer loop and outside its holes.
fn inside_section(plane: &Plane, section: &[Vec<DVec3>], p: DVec3) -> bool {
    let proj = |l: &Vec<DVec3>| l.iter().map(|q| plane.project(*q)).collect::<Vec<_>>();
    let q = plane.project(p);
    point_in_polygon(q, &proj(&section[0]))
        && !section[1..].iter().any(|h| point_in_polygon(q, &proj(h)))
}
