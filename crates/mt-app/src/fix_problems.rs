//! Model Info > Statistics > Fix Problems (FEATURES.md 61): runs the kernel's
//! topology checks (`mt_kernel::validate`) on the model root and every definition,
//! repairs what can be repaired safely and reports what is left.
//!
//! Repairs (SketchUp's own list of fixes is unverified): face normals scaled back to
//! unit length; faces with broken loops (fewer than three edges, missing edges, open
//! loops) removed; edges on one vertex removed; edge, vertex and face lists cleaned
//! of entries that no longer exist or do not belong. Geometric problems (non-planar
//! faces, crossing edges) are reported, not changed.

use mt_kernel::{Checks, Entities, Model, validate};

/// What Fix Problems found and fixed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub found: usize,
    pub fixed: usize,
}

impl Report {
    pub(crate) fn message(&self) -> String {
        match (self.found, self.found - self.fixed.min(self.found)) {
            (0, _) => "No problems found.".to_owned(),
            (n, 0) => format!("{n} problems found and fixed."),
            (n, left) => format!(
                "{n} problems found, {} fixed; {left} could not be fixed.",
                n - left
            ),
        }
    }
}

/// How many topology problems the model has (nothing is changed).
pub(crate) fn count_problems(m: &Model) -> usize {
    std::iter::once(&m.root)
        .chain(m.definitions.values().map(|d| &d.entities))
        .map(|e| validate(e, Checks::TOPOLOGY).len())
        .sum()
}

/// Checks and repairs every entity collection of the model.
pub(crate) fn fix_model(m: &mut Model) -> Report {
    let mut r = fix_entities(&mut m.root);
    for d in m.definitions.values_mut() {
        let x = fix_entities(&mut d.entities);
        r.found += x.found;
        r.fixed += x.fixed;
    }
    r
}

/// Checks and repairs one entity collection.
pub(crate) fn fix_entities(ents: &mut Entities) -> Report {
    let found = validate(ents, Checks::TOPOLOGY).len();
    if found == 0 {
        return Report::default();
    }
    // Lists naming entities that are gone or do not use them.
    let vertex_ids: Vec<_> = ents.vertices.keys().collect();
    for v in vertex_ids {
        let edges = ents.vertices[v].edges.clone();
        let keep: Vec<_> = edges
            .into_iter()
            .filter(|e| {
                ents.edges
                    .get(*e)
                    .is_some_and(|x| x.start == v || x.end == v)
            })
            .collect();
        ents.vertices[v].edges = keep;
    }
    let edge_ids: Vec<_> = ents.edges.keys().collect();
    for e in &edge_ids {
        let faces = ents.edges[*e].faces.clone();
        let keep: Vec<_> = faces
            .into_iter()
            .filter(|f| {
                ents.faces.get(*f).is_some_and(|face| {
                    face.loops().any(|l| l.coedges.iter().any(|c| c.edge == *e))
                })
            })
            .collect();
        ents.edges[*e].faces = keep;
    }
    // Edges whose vertices are gone or that start and end on one vertex.
    for e in edge_ids {
        let Some(edge) = ents.edges.get(e) else {
            continue;
        };
        let broken = edge.start == edge.end
            || !ents.vertices.contains_key(edge.start)
            || !ents.vertices.contains_key(edge.end);
        if broken {
            ents.remove_edge(e);
        }
    }
    // Faces: unit normals, loops that close over existing edges.
    let face_ids: Vec<_> = ents.faces.keys().collect();
    for f in face_ids {
        let n = ents.faces[f].normal;
        if n.length() > 1e-12 && (n.length() - 1.0).abs() > 1e-9 {
            ents.faces[f].normal = n.normalize();
        }
        let face = &ents.faces[f];
        let bad_loop = face.normal.length() <= 1e-12
            || face.loops().any(|l| {
                l.coedges.len() < 3
                    || l.coedges.iter().any(|c| !ents.edges.contains_key(c.edge))
                    || (0..l.coedges.len()).any(|k| {
                        let next = l.coedges[(k + 1) % l.coedges.len()];
                        ents.coedge_ends(l.coedges[k]).1 != ents.coedge_ends(next).0
                    })
            });
        if bad_loop {
            ents.remove_face(f);
            continue;
        }
        // Edges of the face must list it.
        let edges: Vec<_> = face
            .loops()
            .flat_map(|l| l.coedges.iter().map(|c| c.edge))
            .collect();
        for e in edges {
            if let Some(x) = ents.edges.get_mut(e)
                && !x.faces.contains(&f)
            {
                x.faces.push(f);
            }
        }
    }
    let left = validate(ents, Checks::TOPOLOGY).len();
    Report {
        found,
        fixed: found.saturating_sub(left),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use mt_kernel::add_line;

    fn square(e: &mut Entities) {
        let p = |x: f64, y: f64| DVec3::new(x, y, 0.0);
        for (a, b) in [
            (p(0.0, 0.0), p(1.0, 0.0)),
            (p(1.0, 0.0), p(1.0, 1.0)),
            (p(1.0, 1.0), p(0.0, 1.0)),
            (p(0.0, 1.0), p(0.0, 0.0)),
        ] {
            add_line(e, a, b);
        }
    }

    #[test]
    fn a_valid_model_has_no_problems() {
        let mut m = Model::default();
        square(&mut m.root);
        assert_eq!(fix_model(&mut m), Report::default());
        assert_eq!(Report::default().message(), "No problems found.");
    }

    #[test]
    fn repairs_normals_and_stale_lists() {
        let mut e = Entities::new();
        square(&mut e);
        let f = e.faces.keys().next().unwrap();
        e.faces[f].normal *= 3.0;
        // An edge listing a face that is gone.
        let mut other = Entities::new();
        square(&mut other);
        let gone = other.faces.keys().next().unwrap();
        let e0 = e.edges.keys().next().unwrap();
        // A stale id from another collection: not a face of `e` (or not one using
        // the edge).
        if gone != f {
            e.edges[e0].faces.push(gone);
        } else {
            e.remove_face(f);
            e.edges[e0].faces.push(f);
        }
        let r = fix_entities(&mut e);
        assert!(r.found >= 1, "{r:?}");
        assert_eq!(r.fixed, r.found, "{r:?}");
        assert!(validate(&e, Checks::TOPOLOGY).is_empty());
        if let Some(face) = e.faces.values().next() {
            assert!((face.normal.length() - 1.0).abs() < 1e-12);
        }
        assert!(r.message().ends_with("found and fixed."));
    }
}
