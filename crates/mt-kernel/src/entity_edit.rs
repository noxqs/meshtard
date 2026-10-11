//! Geometry edits typed into Entity Info: an edge's length, an arc's or circle's
//! radius and segment count.

use glam::{DMat4, DVec3};

use crate::TOL;
use crate::entities::{CurveId, EdgeAttrs, EdgeId, Entities, FaceAttrs};
use crate::transform::transform_vertices;

/// Sets the length of edge `e` by moving its end vertex along the edge (the start
/// stays; which end SketchUp keeps is inferred, unverified). Connected geometry
/// stretches as with Move. Returns false when nothing changed (no such edge, a
/// length below the tolerance, or the same length).
pub fn set_edge_length(ents: &mut Entities, e: EdgeId, length: f64) -> bool {
    let Some(edge) = ents.edges.get(e) else {
        return false;
    };
    let (a, b) = (ents.pos(edge.start), ents.pos(edge.end));
    let old = a.distance(b);
    if length < TOL || old < TOL || (length - old).abs() < 1e-9 {
        return false;
    }
    let dir = (b - a) / old;
    let end = edge.end;
    transform_vertices(ents, &[end], DMat4::from_translation(dir * (length - old)));
    true
}

/// Sets the radius of the arc or circle `c` by scaling it about its center in its
/// plane; edges connected to an arc's ends stretch. Returns false when `c` is not an
/// arc curve or nothing changed.
pub fn set_arc_radius(ents: &mut Entities, c: CurveId, radius: f64) -> bool {
    let Some(arc) = ents.curves.get(c).and_then(|x| x.arc) else {
        return false;
    };
    if radius < TOL || (radius - arc.radius).abs() < 1e-9 {
        return false;
    }
    let edges = ents.curve_edges(c);
    let vertices = crate::transform::vertices_of(ents, &edges, &[]);
    let k = radius / arc.radius;
    let xf = DMat4::from_translation(arc.center)
        * DMat4::from_scale(DVec3::splat(k))
        * DMat4::from_translation(-arc.center);
    transform_vertices(ents, &vertices, xf);
    true
}

/// Redraws the arc or circle `c` with `segments` segments (at least 2 for an arc, 3
/// for a circle; at most 999). Faces it bounded are recreated and keep their
/// materials and other attributes; its edges keep theirs (soft, smooth, tag).
/// Returns the new curve's edges, or `None` when nothing changed.
pub fn set_arc_segments(ents: &mut Entities, c: CurveId, segments: usize) -> Option<Vec<EdgeId>> {
    let mut arc = ents.curves.get(c).and_then(|x| x.arc)?;
    let min = if arc.is_circle() { 3 } else { 2 };
    let segments = segments.clamp(min, 999);
    if segments == arc.segments {
        return None;
    }
    let old_edges = ents.curve_edges(c);
    let edge_attrs: EdgeAttrs = old_edges
        .first()
        .and_then(|&e| ents.edges.get(e))
        .map(|e| EdgeAttrs {
            curve: None,
            ..e.attrs.clone()
        })
        .unwrap_or_default();
    // The faces around the curve, to give the new faces their attributes back.
    let mut faces: Vec<_> = old_edges
        .iter()
        .flat_map(|&e| ents.edges[e].faces.iter().copied())
        .collect();
    faces.sort();
    faces.dedup();
    let before = ents.clone();
    let old_faces: Vec<(crate::entities::FaceId, DVec3, FaceAttrs)> = faces
        .iter()
        .map(|&f| (f, before.faces[f].normal, before.faces[f].attrs.clone()))
        .collect();

    crate::ops::erase_curve(ents, c);
    arc.segments = segments;
    let edges = crate::ops::add_curve(ents, &arc.points(), Some(arc));
    for &e in &edges {
        if let Some(x) = ents.edges.get_mut(e) {
            let curve = x.attrs.curve;
            x.attrs = EdgeAttrs {
                curve,
                ..edge_attrs.clone()
            };
        }
    }
    // New faces along the curve take the attributes of the old face they lie in.
    let mut new_faces: Vec<_> = edges
        .iter()
        .filter_map(|&e| ents.edges.get(e))
        .flat_map(|e| e.faces.iter().copied())
        .collect();
    new_faces.sort();
    new_faces.dedup();
    for f in new_faces {
        let sample = crate::transform::face_sample(ents, f);
        let normal = ents.faces[f].normal;
        if let Some((_, old_normal, attrs)) = old_faces.iter().find(|(old, n, _)| {
            n.dot(normal).abs() > 1.0 - 1e-6 && before.face_contains(*old, sample)
        }) {
            let face = &mut ents.faces[f];
            face.attrs = attrs.clone();
            if old_normal.dot(normal) < 0.0 {
                // Recreated facing the other way: swap the sides' materials.
                let a = &mut face.attrs;
                std::mem::swap(&mut a.material, &mut a.back_material);
                a.uv = a.uv.take().map(|uv| {
                    Box::new(crate::texture::FaceUv {
                        front: uv.back,
                        back: uv.front,
                    })
                });
            }
        }
    }
    Some(edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Arc;

    fn circle(r: f64, segments: usize) -> Arc {
        Arc {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            x_axis: DVec3::X,
            radius: r,
            start_angle: 0.0,
            end_angle: std::f64::consts::TAU,
            segments,
        }
    }

    fn area(ents: &Entities) -> f64 {
        ents.faces
            .keys()
            .map(|f| {
                crate::tessellate::triangulate(ents, f)
                    .iter()
                    .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
                    .sum::<f64>()
            })
            .sum()
    }

    #[test]
    fn edge_length_moves_the_end() {
        let mut e = Entities::new();
        let edges = crate::add_line(&mut e, DVec3::ZERO, DVec3::new(3.0, 4.0, 0.0));
        let edge = edges[0];
        assert!(set_edge_length(&mut e, edge, 10.0));
        let x = &e.edges[edge];
        let (a, b) = (e.pos(x.start), e.pos(x.end));
        assert!((a.distance(b) - 10.0).abs() < 1e-9);
        assert!(a.abs_diff_eq(DVec3::ZERO, 1e-12) || b.abs_diff_eq(DVec3::ZERO, 1e-12));
        assert!(!set_edge_length(&mut e, edge, 10.0), "same length");
        assert!(!set_edge_length(&mut e, edge, 0.0), "zero length");
    }

    #[test]
    fn edge_length_stretches_a_rectangle_face() {
        let mut e = Entities::new();
        let p = [
            DVec3::ZERO,
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 5.0, 0.0),
            DVec3::new(0.0, 5.0, 0.0),
        ];
        for i in 0..4 {
            crate::add_line(&mut e, p[i], p[(i + 1) % 4]);
        }
        assert_eq!(e.faces.len(), 1);
        let bottom = e
            .edges
            .iter()
            .find(|(_, x)| {
                let (a, b) = (e.pos(x.start), e.pos(x.end));
                a.y.abs() < 1e-9 && b.y.abs() < 1e-9
            })
            .map(|(k, _)| k)
            .unwrap();
        assert!(set_edge_length(&mut e, bottom, 20.0));
        // A trapezoid now: (20 + 10) / 2 * 5.
        assert_eq!(e.faces.len(), 1);
        assert!((area(&e) - 75.0).abs() < 1e-6, "{}", area(&e));
    }

    #[test]
    fn radius_scales_a_circle_with_its_face() {
        let mut e = Entities::new();
        let c = circle(10.0, 24);
        crate::add_curve(&mut e, &c.points(), Some(c));
        let curve = e.curves.keys().next().unwrap();
        let before = area(&e);
        assert!(set_arc_radius(&mut e, curve, 20.0));
        assert_eq!(e.faces.len(), 1);
        assert!((area(&e) / before - 4.0).abs() < 1e-6);
        assert!((e.curves[curve].arc.unwrap().radius - 20.0).abs() < 1e-9);
        assert!(!set_arc_radius(&mut e, curve, 20.0));
    }

    #[test]
    fn segments_rebuild_keeps_the_face_material() {
        let mut e = Entities::new();
        let c = circle(10.0, 24);
        crate::add_curve(&mut e, &c.points(), Some(c));
        let f = e.faces.keys().next().unwrap();
        e.faces[f].attrs.material = Some(3);
        let curve = e.curves.keys().next().unwrap();
        for k in e.curve_edges(curve) {
            e.edges[k].attrs.soft = true;
        }
        let edges = set_arc_segments(&mut e, curve, 12).expect("changed");
        assert_eq!(edges.len(), 12);
        assert_eq!(e.curves.len(), 1);
        let new_curve = e.curves.keys().next().unwrap();
        assert_eq!(e.curves[new_curve].arc.unwrap().segments, 12);
        assert_eq!(e.faces.len(), 1);
        let f = e.faces.keys().next().unwrap();
        assert_eq!(e.faces[f].attrs.material, Some(3));
        assert!(edges.iter().all(|&k| e.edges[k].attrs.soft));
        assert!(set_arc_segments(&mut e, new_curve, 12).is_none());
        // Never below 3 for a circle.
        let edges = set_arc_segments(&mut e, new_curve, 1).expect("changed");
        assert_eq!(edges.len(), 3);
    }

    #[test]
    fn segments_of_a_circle_in_a_face_keep_both_faces() {
        let mut e = Entities::new();
        let p = [
            DVec3::new(-30.0, -30.0, 0.0),
            DVec3::new(30.0, -30.0, 0.0),
            DVec3::new(30.0, 30.0, 0.0),
            DVec3::new(-30.0, 30.0, 0.0),
        ];
        for i in 0..4 {
            crate::add_line(&mut e, p[i], p[(i + 1) % 4]);
        }
        let c = circle(10.0, 24);
        crate::add_curve(&mut e, &c.points(), Some(c));
        assert_eq!(e.faces.len(), 2);
        let disc = e
            .faces
            .iter()
            .find(|(_, x)| x.holes.is_empty() && x.outer.coedges.len() == 24)
            .map(|(k, _)| k)
            .unwrap();
        e.faces[disc].attrs.material = Some(1);
        let curve = e.curves.keys().next().unwrap();
        set_arc_segments(&mut e, curve, 8).expect("changed");
        assert_eq!(e.faces.len(), 2);
        let mats: Vec<_> = e
            .faces
            .values()
            .map(|x| (x.outer.coedges.len(), x.attrs.material))
            .collect();
        assert!(mats.contains(&(8, Some(1))), "{mats:?}");
        assert!(mats.contains(&(4, None)), "{mats:?}");
    }
}
