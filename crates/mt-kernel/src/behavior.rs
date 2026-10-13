//! Component behaviors applied when the model is drawn, never to the model itself:
//! "Cut opening" (a glued component shows a hole in the face it sits on), "Always face
//! camera" (the instance turns about its blue axis toward the viewer) and "Shadows face
//! sun" (such an instance casts its shadow as if it faced the sun).
//!
//! SketchUp cuts the opening along the component's edges lying in its red-green plane
//! (the gluing plane); the outermost loops they form are the holes. Which way a
//! face-camera component faces is from SketchUp's Ruby docs ("the Y axis faces the
//! camera"); that it is the component's -Y side (its front in Front view) that turns
//! toward the viewer is unverified.

use glam::{DMat4, DVec2, DVec3};

use crate::TOL;
use crate::arrangement::{cells, edges_in_plane};
use crate::entities::{Entities, FaceId};
use crate::geom::{Plane, point_in_polygon, signed_area2};

/// Where an instance turns to: a viewer's eye (perspective) or a direction (parallel
/// projection, the sun).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Toward {
    Point(DVec3),
    Direction(DVec3),
}

impl Toward {
    /// The direction to turn to from `origin`.
    pub fn from(self, origin: DVec3) -> DVec3 {
        match self {
            Self::Point(p) => p - origin,
            Self::Direction(d) => d,
        }
    }
}

/// The rotation (in the space `world` maps to) about the instance's blue axis through
/// its origin that turns its front (-Y) toward `toward`. `world` maps definition space
/// to that space. Identity when the blue axis points at the target or `world` is
/// degenerate.
pub fn face_toward(world: DMat4, toward: Toward) -> DMat4 {
    let origin = world.transform_point3(DVec3::ZERO);
    let axis = world.transform_vector3(DVec3::Z).normalize_or_zero();
    let flat = |v: DVec3| (v - axis * v.dot(axis)).normalize_or_zero();
    let front = flat(-world.transform_vector3(DVec3::Y));
    let target = flat(toward.from(origin));
    if axis == DVec3::ZERO || front == DVec3::ZERO || target == DVec3::ZERO {
        return DMat4::IDENTITY;
    }
    let angle = front.cross(target).dot(axis).atan2(front.dot(target));
    if angle.abs() < 1e-12 {
        return DMat4::IDENTITY;
    }
    DMat4::from_translation(origin)
        * DMat4::from_axis_angle(axis, angle)
        * DMat4::from_translation(-origin)
}

/// The holes a "Cut opening" component cuts, in definition coordinates: the outermost
/// loops formed by its edges in the red-green plane (z = 0), counter-clockwise seen
/// from +Z. Loops inside another loop (a window frame's inner edge) are left out.
pub fn opening_loops(def: &Entities) -> Vec<Vec<DVec3>> {
    let plane = Plane::new(DVec3::Z, DVec3::ZERO);
    let edges = edges_in_plane(def, &plane);
    let regions = cells(def, &plane, &edges);
    let polys: Vec<Vec<DVec3>> = regions
        .iter()
        .map(|c| {
            c.outer
                .iter()
                .map(|&h| def.pos(def.coedge_ends(h).0))
                .collect()
        })
        .collect();
    let flat: Vec<Vec<DVec2>> = polys
        .iter()
        .map(|p| p.iter().map(|q| q.truncate()).collect())
        .collect();
    (0..regions.len())
        .filter(|&i| {
            let sample = regions[i].sample.truncate();
            !(0..regions.len()).any(|j| {
                j != i
                    && signed_area2(&flat[j]).abs() > signed_area2(&flat[i]).abs()
                    && point_in_polygon(sample, &flat[j])
            })
        })
        .map(|i| polys[i].clone())
        .collect()
}

/// Triangles of face `f` with `openings` (closed loops, in the face's coordinates) cut
/// out, counter-clockwise around the face normal, or `None` when no opening lies in
/// the face's plane over the face (draw it whole). Openings may cross or touch the
/// face's edges (a door at the foot of a wall): the face and the openings are
/// intersected in a scratch copy, the model is not changed.
pub fn cut_face(ents: &Entities, f: FaceId, openings: &[Vec<DVec3>]) -> Option<Vec<[DVec3; 3]>> {
    let face = &ents.faces[f];
    let plane = ents.face_plane(f);
    let loops: Vec<Vec<DVec3>> = face.loops().map(|l| ents.loop_points(l)).collect();
    let proj = |ps: &[DVec3]| -> Vec<DVec2> { ps.iter().map(|&p| plane.project(p)).collect() };
    let bounds = |ps: &[DVec2]| {
        ps.iter().fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        )
    };
    let face2: Vec<Vec<DVec2>> = loops.iter().map(|l| proj(l)).collect();
    let (lo, hi) = bounds(&face2[0]);
    let cuts: Vec<(&Vec<DVec3>, Vec<DVec2>)> = openings
        .iter()
        .filter(|o| o.len() >= 3 && o.iter().all(|&p| plane.distance(p).abs() <= TOL))
        .map(|o| (o, proj(o)))
        .filter(|(_, o2)| {
            let (a, b) = bounds(o2);
            a.x < hi.x - TOL && b.x > lo.x + TOL && a.y < hi.y - TOL && b.y > lo.y + TOL
        })
        .collect();
    if cuts.is_empty() {
        return None;
    }

    let mut scratch = Entities::default();
    for ring in loops.iter().chain(cuts.iter().map(|(o, _)| *o)) {
        for (k, &a) in ring.iter().enumerate() {
            let b = ring[(k + 1) % ring.len()];
            if a.distance(b) > TOL {
                crate::ops::add_line(&mut scratch, a, b);
            }
        }
    }
    let inside_face = |p: DVec2| {
        point_in_polygon(p, &face2[0]) && !face2[1..].iter().any(|h| point_in_polygon(p, h))
    };
    let mut out = Vec::new();
    for cell in cells(&scratch, &plane, &edges_in_plane(&scratch, &plane)) {
        let sample = plane.project(cell.sample);
        if !inside_face(sample) || cuts.iter().any(|(_, o2)| point_in_polygon(sample, o2)) {
            continue;
        }
        let rings: Vec<Vec<DVec3>> = std::iter::once(&cell.outer)
            .chain(&cell.holes)
            .map(|r| {
                r.iter()
                    .map(|&h| scratch.pos(scratch.coedge_ends(h).0))
                    .collect()
            })
            .collect();
        let mut flat = Vec::new();
        let mut starts = Vec::new();
        let mut points = Vec::new();
        for (i, r) in rings.iter().enumerate() {
            if i > 0 {
                starts.push(points.len());
            }
            for &p in r {
                let q = plane.project(p);
                flat.extend([q.x, q.y]);
                points.push(p);
            }
        }
        let Ok(idx) = crate::geom::earcut(&flat, &starts) else {
            continue;
        };
        for t in idx.as_chunks::<3>().0 {
            let (a, b, c) = (points[t[0]], points[t[1]], points[t[2]]);
            out.push(if (b - a).cross(c - a).dot(face.normal) < 0.0 {
                [a, c, b]
            } else {
                [a, b, c]
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(tris: &[[DVec3; 3]]) -> f64 {
        tris.iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
            .sum()
    }

    fn rect(ents: &mut Entities, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) {
        let p = [
            DVec3::new(x0, y0, z),
            DVec3::new(x1, y0, z),
            DVec3::new(x1, y1, z),
            DVec3::new(x0, y1, z),
        ];
        for k in 0..4 {
            crate::ops::add_line(ents, p[k], p[(k + 1) % 4]);
        }
    }

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<DVec3> {
        vec![
            DVec3::new(x0, y0, 0.0),
            DVec3::new(x1, y0, 0.0),
            DVec3::new(x1, y1, 0.0),
            DVec3::new(x0, y1, 0.0),
        ]
    }

    fn wall() -> (Entities, FaceId) {
        let mut ents = Entities::default();
        rect(&mut ents, 0.0, 0.0, 100.0, 50.0, 0.0);
        let f = ents.faces.keys().next().expect("auto-faced");
        (ents, f)
    }

    #[test]
    fn opening_inside_the_face_is_a_hole() {
        let (ents, f) = wall();
        let tris = cut_face(&ents, f, &[square(10.0, 10.0, 30.0, 40.0)]).unwrap();
        assert!((area(&tris) - (5000.0 - 600.0)).abs() < 1e-6);
        // The model is untouched.
        assert_eq!(ents.faces.len(), 1);
        assert!(ents.faces[f].holes.is_empty());
    }

    #[test]
    fn opening_at_the_edge_notches_the_face() {
        let (ents, f) = wall();
        // A door from the floor line up, and one hanging past the wall's side.
        let tris = cut_face(
            &ents,
            f,
            &[
                square(40.0, 0.0, 60.0, 30.0),
                square(90.0, 20.0, 120.0, 30.0),
            ],
        )
        .unwrap();
        assert!((area(&tris) - (5000.0 - 600.0 - 100.0)).abs() < 1e-6);
    }

    #[test]
    fn openings_elsewhere_leave_the_face_alone() {
        let (ents, f) = wall();
        let off_plane: Vec<DVec3> = square(10.0, 10.0, 30.0, 40.0)
            .into_iter()
            .map(|p| p + DVec3::Z)
            .collect();
        assert!(cut_face(&ents, f, &[off_plane]).is_none());
        assert!(cut_face(&ents, f, &[square(200.0, 0.0, 300.0, 10.0)]).is_none());
        assert!(cut_face(&ents, f, &[]).is_none());
    }

    #[test]
    fn opening_loops_take_the_outer_outline() {
        let mut def = Entities::default();
        // A frame: outer and inner outline on the gluing plane, plus a box above it.
        rect(&mut def, 0.0, 0.0, 40.0, 60.0, 0.0);
        rect(&mut def, 5.0, 5.0, 35.0, 55.0, 0.0);
        rect(&mut def, 0.0, 0.0, 40.0, 60.0, 3.0);
        let loops = opening_loops(&def);
        assert_eq!(loops.len(), 1);
        let flat: Vec<DVec2> = loops[0].iter().map(|p| p.truncate()).collect();
        assert!((signed_area2(&flat) / 2.0 - 2400.0).abs() < 1e-6);
        assert!(opening_loops(&Entities::default()).is_empty());
    }

    #[test]
    fn faces_camera_about_blue_axis() {
        // An instance at (10, 0, 0), turned 30 degrees.
        let world =
            DMat4::from_translation(DVec3::new(10.0, 0.0, 0.0)) * DMat4::from_rotation_z(0.5);
        let eye = DVec3::new(10.0, 0.0, 0.0) + DVec3::new(3.0, 4.0, 7.0);
        let r = face_toward(world, Toward::Point(eye));
        let turned = r * world;
        let front = -turned.transform_vector3(DVec3::Y);
        assert!((front - DVec3::new(0.6, 0.8, 0.0)).length() < 1e-9);
        // The origin and the blue axis stay put.
        assert!(
            turned
                .transform_point3(DVec3::ZERO)
                .distance(DVec3::new(10.0, 0.0, 0.0))
                < 1e-9
        );
        assert!(turned.transform_vector3(DVec3::Z).distance(DVec3::Z) < 1e-9);
        // Looking straight down the blue axis: nothing to turn.
        assert_eq!(
            face_toward(world, Toward::Direction(DVec3::Z)),
            DMat4::IDENTITY
        );
        // The sun as a direction.
        let s = face_toward(
            DMat4::IDENTITY,
            Toward::Direction(DVec3::new(1.0, 0.0, 1.0)),
        );
        let front = -(s.transform_vector3(DVec3::Y));
        assert!((front - DVec3::X).length() < 1e-9);
    }
}
