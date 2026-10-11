//! 3D Text geometry: flattened glyph outlines to faces with holes, optionally
//! extruded with Push/Pull.
//!
//! Outlines are closed polygons in the XY plane. Which ones are holes comes from how
//! deeply each is nested in the others (even depth: an outer boundary, odd: a hole of
//! the smallest boundary around it), so the result does not depend on a font's
//! contour direction convention.

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{CoEdge, EdgeAttrs, Entities, FaceAttrs, FaceId, Loop};
use crate::geom::{point_in_polygon, signed_area2};

/// Largest angle between extruded side faces whose shared edge is softened.
pub const SMOOTH_ANGLE: f64 = 30.0;

/// Drops repeated points (and a closing copy of the first) and polygons too small to
/// be a face.
fn clean(poly: &[DVec2]) -> Option<Vec<DVec2>> {
    let mut out: Vec<DVec2> = Vec::with_capacity(poly.len());
    for &p in poly {
        if out.last().is_none_or(|q| q.distance(p) > TOL) {
            out.push(p);
        }
    }
    while out.len() > 1 && out[0].distance(out[out.len() - 1]) <= TOL {
        out.pop();
    }
    (out.len() >= 3 && signed_area2(&out).abs() > TOL * TOL).then_some(out)
}

/// A closed edge loop through `pts` at height 0, running the way `pts` run.
fn add_loop(ents: &mut Entities, pts: &[DVec2]) -> Loop {
    let vs: Vec<_> = pts.iter().map(|p| ents.add_vertex(p.extend(0.0))).collect();
    let coedges = (0..vs.len())
        .map(|k| CoEdge {
            edge: ents.add_raw_edge(vs[k], vs[(k + 1) % vs.len()], EdgeAttrs::default()),
            reversed: false,
        })
        .collect();
    Loop { coedges }
}

/// Outer boundaries with their holes: `(outer, holes)` indices into `polys`.
fn nest(polys: &[Vec<DVec2>]) -> Vec<(usize, Vec<usize>)> {
    let area: Vec<f64> = polys.iter().map(|p| signed_area2(p).abs()).collect();
    // Polygons around polygon i (tested with its first point).
    let around: Vec<Vec<usize>> = (0..polys.len())
        .map(|i| {
            (0..polys.len())
                .filter(|&j| {
                    j != i && area[j] > area[i] && point_in_polygon(polys[i][0], &polys[j])
                })
                .collect()
        })
        .collect();
    let mut out: Vec<(usize, Vec<usize>)> = (0..polys.len())
        .filter(|&i| around[i].len().is_multiple_of(2))
        .map(|i| (i, Vec::new()))
        .collect();
    for i in (0..polys.len()).filter(|&i| !around[i].len().is_multiple_of(2)) {
        // The innermost polygon around a hole is its boundary.
        let parent = around[i]
            .iter()
            .copied()
            .min_by(|&a, &b| area[a].total_cmp(&area[b]));
        if let Some(slot) = parent.and_then(|p| out.iter_mut().find(|(o, _)| *o == p)) {
            slot.1.push(i);
        }
    }
    out
}

/// Faces (normal +Z) from closed outlines in the XY plane; with `filled` false only
/// the outline edges. A positive `depth` pushes every face up by it (new side faces;
/// the base faces down). Returns the entities and the faces at the top (or the only
/// faces when not extruded).
pub fn from_outlines(outlines: &[Vec<DVec2>], filled: bool, depth: f64) -> (Entities, Vec<FaceId>) {
    let polys: Vec<Vec<DVec2>> = outlines.iter().filter_map(|p| clean(p)).collect();
    let mut ents = Entities::new();
    if !filled {
        for p in &polys {
            add_loop(&mut ents, p);
        }
        return (ents, Vec::new());
    }
    let mut faces = Vec::new();
    for (outer, holes) in nest(&polys) {
        let oriented = |i: usize, ccw: bool| {
            let mut p = polys[i].clone();
            if (signed_area2(&p) > 0.0) != ccw {
                p.reverse();
            }
            p
        };
        let outer_loop = add_loop(&mut ents, &oriented(outer, true));
        let hole_loops = holes
            .iter()
            .map(|&h| add_loop(&mut ents, &oriented(h, false)))
            .collect();
        faces.push(ents.add_raw_face(outer_loop, hole_loops, DVec3::Z, FaceAttrs::default()));
    }
    if depth > TOL {
        faces = faces
            .into_iter()
            .filter_map(|f| crate::push_pull(&mut ents, f, depth, true))
            .collect();
        // Sides of curved letters look round: edges between side faces less than
        // SMOOTH_ANGLE apart are soft and smooth (unverified for SketchUp).
        let cos = SMOOTH_ANGLE.to_radians().cos();
        let smooth: Vec<_> = ents
            .edges
            .iter()
            .filter(|(_, e)| {
                let [a, b] = e.faces[..] else { return false };
                ents.faces[a].normal.dot(ents.faces[b].normal) > cos
            })
            .map(|(id, _)| id)
            .collect();
        for e in smooth {
            let a = &mut ents.edges[e].attrs;
            a.soft = true;
            a.smooth = true;
        }
    }
    (ents, faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Checks, validate};

    fn square(c: DVec2, half: f64, ccw: bool) -> Vec<DVec2> {
        let mut p: Vec<DVec2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .iter()
            .map(|&(x, y)| c + DVec2::new(x, y) * half)
            .collect();
        if !ccw {
            p.reverse();
        }
        p
    }

    #[test]
    fn nested_outlines_make_holes_and_islands() {
        // An "O" (ring) and, inside its hole, an island; written in either direction.
        for ccw in [true, false] {
            let outlines = vec![
                square(DVec2::ZERO, 10.0, ccw),
                square(DVec2::ZERO, 6.0, ccw),
                square(DVec2::ZERO, 2.0, !ccw),
                square(DVec2::new(30.0, 0.0), 5.0, ccw),
            ];
            let (ents, faces) = from_outlines(&outlines, true, 0.0);
            assert_eq!(faces.len(), 3);
            let holes: Vec<usize> = faces.iter().map(|&f| ents.faces[f].holes.len()).collect();
            assert_eq!(holes.iter().sum::<usize>(), 1);
            assert!(ents.faces.values().all(|f| f.normal == DVec3::Z));
            assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
        }
    }

    #[test]
    fn extruded_ring_is_a_closed_solid() {
        let outlines = vec![
            square(DVec2::ZERO, 10.0, true),
            square(DVec2::ZERO, 6.0, true),
        ];
        let (ents, top) = from_outlines(&outlines, true, 4.0);
        assert_eq!(top.len(), 1);
        // Top, base, four outer and four inner sides.
        assert_eq!(ents.faces.len(), 10);
        assert!(ents.edges.values().all(|e| e.faces.len() == 2));
        // Square corners stay hard.
        assert!(ents.edges.values().all(|e| !e.attrs.soft));
        let f = &ents.faces[top[0]];
        assert_eq!(f.normal, DVec3::Z);
        assert!(
            ents.loop_points(&f.outer)
                .iter()
                .all(|p| (p.z - 4.0).abs() < 1e-9)
        );
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn outline_only() {
        let (ents, faces) = from_outlines(&[square(DVec2::ZERO, 1.0, true)], false, 3.0);
        assert!(faces.is_empty() && ents.faces.is_empty());
        assert_eq!(ents.edges.len(), 4);
    }
}
