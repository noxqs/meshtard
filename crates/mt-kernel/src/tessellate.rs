//! Triangles for drawing faces.

use glam::{DVec2, DVec3};

use crate::entities::{Entities, FaceId, VertexId};
use crate::geom::{point_in_polygon, signed_area2};

/// Splits a closed vertex sequence that visits a vertex more than once (a loop touching
/// itself) into simple cycles.
fn simple_cycles(vs: &[VertexId]) -> Vec<Vec<VertexId>> {
    let mut out = Vec::new();
    let mut stack: Vec<VertexId> = Vec::new();
    for &v in vs.iter().chain(vs.first()) {
        if let Some(i) = stack.iter().position(|&x| x == v) {
            let cycle: Vec<VertexId> = stack.drain(i..).collect();
            if cycle.len() >= 3 {
                out.push(cycle);
            }
        }
        stack.push(v);
    }
    out
}

/// Triangles of a face, counter-clockwise around its normal. Faces earcut cannot
/// handle give no triangles.
pub fn triangulate(ents: &Entities, f: FaceId) -> Vec<[DVec3; 3]> {
    let face = &ents.faces[f];
    let plane = ents.face_plane(f);
    let proj =
        |c: &[VertexId]| -> Vec<DVec2> { c.iter().map(|&v| plane.project(ents.pos(v))).collect() };

    // Pieces of the outer loop that run the same way as the whole loop are regions, the
    // others are holes; every piece of a hole loop is a hole. Files do not always wind
    // holes clockwise, so direction alone cannot tell them apart.
    let mut outers: Vec<(Vec<VertexId>, Vec<DVec2>)> = Vec::new();
    let mut holes: Vec<(Vec<VertexId>, Vec<DVec2>)> = Vec::new();
    let outer_sign = signed_area2(&proj(&ents.loop_vertices(&face.outer))).signum();
    for (i, l) in face.loops().enumerate() {
        for cycle in simple_cycles(&ents.loop_vertices(l)) {
            let poly = proj(&cycle);
            if i == 0 && signed_area2(&poly).signum() == outer_sign {
                outers.push((cycle, poly));
            } else {
                holes.push((cycle, poly));
            }
        }
    }
    outers.sort_by(|a, b| {
        signed_area2(&a.1)
            .abs()
            .total_cmp(&signed_area2(&b.1).abs())
    });

    let mut assigned: Vec<Vec<usize>> = vec![Vec::new(); outers.len()];
    for (h, (_, poly)) in holes.iter().enumerate() {
        // Smallest outer piece containing a point inside the hole (a centroid can fall
        // outside a non-convex hole).
        let c = crate::arrangement::interior_point(poly, &[]);
        if let Some(i) = outers.iter().position(|(_, o)| point_in_polygon(c, o)) {
            assigned[i].push(h);
        }
    }

    let mut out = Vec::new();
    for ((outer, _), hs) in outers.iter().zip(assigned) {
        let mut ids: Vec<VertexId> = outer.clone();
        let mut starts = Vec::new();
        for h in hs {
            starts.push(ids.len());
            ids.extend(&holes[h].0);
        }
        let flat: Vec<f64> = ids
            .iter()
            .flat_map(|&v| {
                let q = plane.project(ents.pos(v));
                [q.x, q.y]
            })
            .collect();
        let Ok(idx) = earcutr::earcut(&flat, &starts, 2) else {
            continue;
        };
        for t in idx.as_chunks::<3>().0 {
            let (a, b, c) = (
                ents.pos(ids[t[0]]),
                ents.pos(ids[t[1]]),
                ents.pos(ids[t[2]]),
            );
            out.push(if (b - a).cross(c - a).dot(face.normal) < 0.0 {
                [a, c, b]
            } else {
                [a, b, c]
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn pinched_loop_splits() {
        let mut sm: SlotMap<VertexId, ()> = SlotMap::with_key();
        let v: Vec<VertexId> = (0..5).map(|_| sm.insert(())).collect();
        // a b c a d e: two triangles sharing vertex a.
        let cycles = simple_cycles(&[v[0], v[1], v[2], v[0], v[3], v[4]]);
        assert_eq!(cycles.len(), 2);
        assert!(cycles.iter().all(|c| c.len() == 3));
    }
}
