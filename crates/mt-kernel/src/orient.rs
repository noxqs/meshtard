//! Reverse Faces and Orient Faces (context menu of faces).
//!
//! Drafts from memory of SketchUp (`spec/parity/STATUS.md`):
//! - Reverse Faces turns each face over and swaps its front and back materials (and
//!   positioned textures), so the painted colors stay on the same physical side
//!   (unverified).
//! - Orient Faces walks from the picked face over edges shared by exactly two faces
//!   and turns over every reached face whose winding disagrees with its neighbor's
//!   (a shared edge must run opposite ways in the two faces). The picked face keeps
//!   its orientation. Edges with three or more faces stop the walk (unverified).

use std::collections::{BTreeSet, VecDeque};

use crate::entities::{CoEdge, EdgeId, Entities, FaceId};

/// Turns face `f` over: loops run the other way, the normal flips, and the front and
/// back materials and texture positions swap.
pub fn reverse_face(ents: &mut Entities, f: FaceId) -> bool {
    let Some(face) = ents.faces.get_mut(f) else {
        return false;
    };
    face.normal = -face.normal;
    for l in face.loops_mut() {
        l.coedges = l
            .coedges
            .iter()
            .rev()
            .map(|c| CoEdge {
                edge: c.edge,
                reversed: !c.reversed,
            })
            .collect();
    }
    let a = &mut face.attrs;
    std::mem::swap(&mut a.material, &mut a.back_material);
    if let Some(uv) = a.uv.as_deref_mut() {
        std::mem::swap(&mut uv.front, &mut uv.back);
    }
    true
}

/// Reverse Faces: turns every face of `faces` over. Returns whether any changed.
pub fn reverse_faces(ents: &mut Entities, faces: &[FaceId]) -> bool {
    let mut seen = BTreeSet::new();
    let mut any = false;
    for &f in faces {
        if seen.insert(f) {
            any |= reverse_face(ents, f);
        }
    }
    any
}

/// Whether face `f` uses edge `e` reversed (`None` when it does not use it).
fn use_of(ents: &Entities, f: FaceId, e: EdgeId) -> Option<bool> {
    ents.faces[f]
        .loops()
        .flat_map(|l| &l.coedges)
        .find(|c| c.edge == e)
        .map(|c| c.reversed)
}

/// Orient Faces: makes the faces connected to `seed` (through edges with two faces)
/// match its orientation. Returns the faces turned over.
pub fn orient_faces(ents: &mut Entities, seed: FaceId) -> Vec<FaceId> {
    let mut flipped = Vec::new();
    if !ents.faces.contains_key(seed) {
        return flipped;
    }
    let mut done: BTreeSet<FaceId> = BTreeSet::from([seed]);
    let mut queue = VecDeque::from([seed]);
    while let Some(f) = queue.pop_front() {
        let edges: Vec<EdgeId> = ents.faces[f]
            .loops()
            .flat_map(|l| l.coedges.iter().map(|c| c.edge))
            .collect();
        for e in edges {
            let [a, b] = ents.edges[e].faces[..] else {
                continue; // free or non-manifold edge
            };
            let g = if a == f { b } else { a };
            if g == f || !done.insert(g) {
                continue;
            }
            if use_of(ents, f, e) == use_of(ents, g, e) {
                reverse_face(ents, g);
                flipped.push(g);
            }
            queue.push_back(g);
        }
    }
    flipped
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;
    use crate::{Checks, add_line, validate};

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn check(ents: &Entities) {
        let errs = validate(ents, Checks::ALL);
        assert!(errs.is_empty(), "invalid: {errs:#?}");
    }

    /// A closed box of six faces, 10 x 10 x 10.
    fn cube() -> Entities {
        let mut e = Entities::new();
        let c = [
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 10.0, 0.0),
            p(0.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            add_line(&mut e, c[i], c[(i + 1) % 4]);
            let up = |q: DVec3| q + DVec3::Z * 10.0;
            add_line(&mut e, up(c[i]), up(c[(i + 1) % 4]));
            add_line(&mut e, c[i], up(c[i]));
        }
        assert_eq!(e.faces.len(), 6);
        e
    }

    /// Every edge with two faces runs opposite ways in them.
    fn consistent(e: &Entities) -> bool {
        e.edges.iter().all(|(id, edge)| match edge.faces[..] {
            [a, b] => use_of(e, a, id) != use_of(e, b, id),
            _ => true,
        })
    }

    #[test]
    fn reverse_flips_normal_and_swaps_materials() {
        let mut e = cube();
        let f = e.faces.keys().next().unwrap();
        e.faces[f].attrs.material = Some(3);
        let n = e.faces[f].normal;
        assert!(reverse_faces(&mut e, &[f, f]));
        assert_eq!(e.faces[f].normal, -n);
        assert_eq!(e.faces[f].attrs.material, None);
        assert_eq!(e.faces[f].attrs.back_material, Some(3));
        check(&e);
        // Twice is the original.
        reverse_faces(&mut e, &[f]);
        assert_eq!(e.faces[f].normal, n);
        assert_eq!(e.faces[f].attrs.material, Some(3));
        check(&e);
    }

    #[test]
    fn orient_makes_a_box_consistent_from_the_seed() {
        let mut e = cube();
        let faces: Vec<FaceId> = e.faces.keys().collect();
        // Turn three faces over at random-ish.
        reverse_faces(&mut e, &[faces[0], faces[2], faces[5]]);
        let seed = faces[1];
        let n = e.faces[seed].normal;
        let flipped = orient_faces(&mut e, seed);
        assert!(!flipped.contains(&seed));
        assert_eq!(e.faces[seed].normal, n, "the seed keeps its side");
        assert!(consistent(&e));
        check(&e);
        // Already consistent: nothing to do.
        assert!(orient_faces(&mut e, seed).is_empty());
        // From an inward face, everything turns inward.
        reverse_face(&mut e, seed);
        let flipped = orient_faces(&mut e, seed);
        assert_eq!(flipped.len(), 5);
        assert!(consistent(&e));
        check(&e);
    }
}
