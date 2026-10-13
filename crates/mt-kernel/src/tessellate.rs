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
    triangulate_with(ents, f, true)
}

/// [`triangulate`]; `fast` takes the shortcut for simple faces (off in a test that
/// checks the shortcut gives what the general case gives).
fn triangulate_with(ents: &Entities, f: FaceId, fast: bool) -> Vec<[DVec3; 3]> {
    let face = &ents.faces[f];
    let plane = ents.face_plane(f);
    let proj =
        |c: &[VertexId]| -> Vec<DVec2> { c.iter().map(|&v| plane.project(ents.pos(v))).collect() };
    // Earcut's triangles of `ids`, counter-clockwise around the face normal.
    let emit = |ids: &[VertexId], idx: &[usize], out: &mut Vec<[DVec3; 3]>| {
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
    };

    // The common face: one loop that never touches itself. The general case below
    // then makes the loop its only region with no holes and earcuts exactly this.
    if fast && face.holes.is_empty() {
        let outer = ents.loop_vertices(&face.outer);
        if outer.len() >= 3 && all_distinct(&outer) {
            let poly = proj(&outer);
            // A NaN area makes the general case drop the loop.
            if !signed_area2(&poly).is_nan() {
                let flat: Vec<f64> = poly.iter().flat_map(|q| [q.x, q.y]).collect();
                let mut out = Vec::new();
                if let Ok(idx) = crate::geom::earcut(&flat, &[]) {
                    emit(&outer, &idx, &mut out);
                }
                return out;
            }
        }
    }

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
        let Ok(idx) = crate::geom::earcut(&flat, &starts) else {
            continue;
        };
        emit(&ids, &idx, &mut out);
    }
    out
}

/// No vertex appears twice.
fn all_distinct(vs: &[VertexId]) -> bool {
    if vs.len() <= 16 {
        return vs.iter().enumerate().all(|(i, v)| !vs[..i].contains(v));
    }
    let mut sorted = vs.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).all(|w| w[0] != w[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    /// The shortcut for simple faces gives exactly the general case's triangles, for
    /// every face of the corpus models.
    #[test]
    fn simple_face_shortcut_matches_the_general_case() {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
        let mut faces = 0;
        for entry in std::fs::read_dir(corpus.join("openskp")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|x| x != "skp") {
                continue;
            }
            let Ok(src) = mt_skp::read(&path) else {
                continue;
            };
            let model = crate::import::from_skp(&src);
            let lists =
                std::iter::once(&model.root).chain(model.definitions.values().map(|d| &d.entities));
            for ents in lists {
                for f in ents.faces.keys() {
                    let fast = triangulate_with(ents, f, true);
                    let general = triangulate_with(ents, f, false);
                    assert_eq!(fast, general, "{}", path.display());
                    faces += 1;
                }
            }
        }
        assert!(faces > 1000, "{faces} faces compared");
    }

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
