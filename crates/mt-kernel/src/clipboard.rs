//! Edit > Cut / Copy / Paste / Paste In Place: the clipboard is a detached [`Entities`]
//! holding a copy of the selection.
//!
//! - Copy takes the selected edges and faces (with the edges bounding the faces),
//!   component instances (their definitions are shared by id: the clipboard only makes
//!   sense within the model it was copied from), dimensions and leader texts, converted
//!   by a transform (the app uses the open context's transform, so the clipboard holds
//!   world coordinates).
//! - Paste inserts the clipboard into a context through a transform. Loose geometry
//!   merges with what is there the way drawing does ([`copy_geometry_from`]).
//!
//! Behaviors are drafts from memory of SketchUp (`spec/parity/STATUS.md`).

use glam::{DMat4, DVec3};
use slotmap::SlotMap;

use crate::annotation::{Dimension, Text};
use crate::entities::{DefinitionId, EdgeId, Entities, FaceId};
use crate::group::{Selection, bounds, copy_raw};
use crate::model::Definition;
use crate::transform::copy_geometry_from;

/// What a paste created in the target context.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pasted {
    pub edges: Vec<EdgeId>,
    pub faces: Vec<FaceId>,
    /// Indices into the context's `instances`, `dimensions` and `texts`.
    pub instances: Vec<usize>,
    pub dimensions: Vec<usize>,
    pub texts: Vec<usize>,
}

/// Copies the selection of `src` (plus the dimensions and texts at the given indices)
/// into a new collection, transformed by `xf`. Geometry is copied as is, without
/// merging; screen texts keep their screen position.
pub fn copy(
    src: &Entities,
    sel: &Selection,
    dimensions: &[usize],
    texts: &[usize],
    xf: DMat4,
) -> Entities {
    let mut out = Entities::new();
    let faces: Vec<FaceId> = sel
        .faces
        .iter()
        .copied()
        .filter(|f| src.faces.contains_key(*f))
        .collect();
    let edges: Vec<EdgeId> = sel
        .edges
        .iter()
        .copied()
        .filter(|e| src.edges.contains_key(*e))
        .collect();
    copy_raw(&mut out, src, &edges, &faces, xf);
    let mut instances = sel.instances.clone();
    instances.sort_unstable();
    instances.dedup();
    for i in instances {
        if let Some(inst) = src.instances.get(i) {
            let mut c = inst.clone();
            c.transform = xf * c.transform;
            out.instances.push(c);
        }
    }
    for &i in dimensions {
        if let Some(d) = src.dimensions.get(i) {
            out.dimensions.push(transform_dimension(d, xf));
        }
    }
    for &i in texts {
        if let Some(t) = src.texts.get(i) {
            out.texts.push(transform_text(t, xf));
        }
    }
    out
}

/// Whether the clipboard holds nothing to paste.
pub fn is_empty(clip: &Entities) -> bool {
    clip.edges.is_empty()
        && clip.faces.is_empty()
        && clip.instances.is_empty()
        && clip.dimensions.is_empty()
        && clip.texts.is_empty()
}

/// Bounding box of the clipboard's contents (instances by their definitions'
/// geometry, annotations by their points; screen texts are left out).
pub fn clip_bounds(
    clip: &Entities,
    defs: &SlotMap<DefinitionId, Definition>,
) -> Option<(DVec3, DVec3)> {
    let all = Selection {
        edges: clip.edges.keys().collect(),
        faces: Vec::new(),
        instances: (0..clip.instances.len()).collect(),
    };
    let mut points: Vec<DVec3> = Vec::new();
    if let Some((lo, hi)) = bounds(clip, &all, defs) {
        points.extend([lo, hi]);
    }
    for d in &clip.dimensions {
        points.extend([d.a, d.b, d.a + d.offset, d.b + d.offset]);
    }
    for t in clip.texts.iter().filter(|t| !t.screen) {
        points.extend([t.point, t.point + t.leader]);
    }
    let lo = points.iter().copied().reduce(DVec3::min)?;
    let hi = points.iter().copied().reduce(DVec3::max)?;
    Some((lo, hi))
}

/// Pastes the clipboard into `dst` transformed by `xf`. Loose edges and faces merge
/// with `dst`'s geometry; instances whose definition `keep` rejects are skipped (a
/// definition missing from the model, or one that would contain itself).
pub fn paste(
    dst: &mut Entities,
    clip: &Entities,
    xf: DMat4,
    keep: impl Fn(DefinitionId) -> bool,
) -> Pasted {
    let edges: Vec<EdgeId> = clip.edges.keys().collect();
    let faces: Vec<FaceId> = clip.faces.keys().collect();
    let (edges, faces) = if edges.is_empty() && faces.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        copy_geometry_from(dst, clip, &edges, &faces, xf)
    };
    let mut out = Pasted {
        edges,
        faces,
        ..Default::default()
    };
    for inst in clip.instances.iter().filter(|i| keep(i.definition)) {
        let mut c = inst.clone();
        c.transform = xf * c.transform;
        dst.instances.push(c);
        out.instances.push(dst.instances.len() - 1);
    }
    for d in &clip.dimensions {
        dst.dimensions.push(transform_dimension(d, xf));
        out.dimensions.push(dst.dimensions.len() - 1);
    }
    for t in &clip.texts {
        dst.texts.push(transform_text(t, xf));
        out.texts.push(dst.texts.len() - 1);
    }
    out
}

fn transform_dimension(d: &Dimension, xf: DMat4) -> Dimension {
    Dimension {
        a: xf.transform_point3(d.a),
        b: xf.transform_point3(d.b),
        offset: xf.transform_vector3(d.offset),
        ..d.clone()
    }
}

fn transform_text(t: &Text, xf: DMat4) -> Text {
    if t.screen {
        return t.clone();
    }
    Text {
        point: xf.transform_point3(t.point),
        leader: xf.transform_vector3(t.leader),
        ..t.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Checks, add_line, validate};

    fn square(ents: &mut Entities, o: DVec3, size: f64) {
        let p = [
            o,
            o + DVec3::X * size,
            o + DVec3::new(size, size, 0.0),
            o + DVec3::Y * size,
        ];
        for i in 0..4 {
            add_line(ents, p[i], p[(i + 1) % 4]);
        }
    }

    fn select_all(ents: &Entities) -> Selection {
        Selection {
            edges: ents.edges.keys().collect(),
            faces: ents.faces.keys().collect(),
            instances: (0..ents.instances.len()).collect(),
        }
    }

    #[test]
    fn copying_a_face_takes_its_edges() {
        let mut ents = Entities::new();
        square(&mut ents, DVec3::ZERO, 10.0);
        let sel = Selection {
            faces: ents.faces.keys().collect(),
            ..Default::default()
        };
        let clip = copy(&ents, &sel, &[], &[], DMat4::IDENTITY);
        assert_eq!((clip.edges.len(), clip.faces.len()), (4, 1));
        assert_eq!(validate(&clip, Checks::ALL), Vec::<String>::new());
        // The source is untouched.
        assert_eq!((ents.edges.len(), ents.faces.len()), (4, 1));
    }

    #[test]
    fn paste_with_offset_merges_with_neighbor() {
        let mut ents = Entities::new();
        square(&mut ents, DVec3::ZERO, 10.0);
        let clip = copy(&ents, &select_all(&ents), &[], &[], DMat4::IDENTITY);
        let defs = SlotMap::with_key();
        let (lo, hi) = clip_bounds(&clip, &defs).unwrap();
        assert_eq!((lo, hi), (DVec3::ZERO, DVec3::new(10.0, 10.0, 0.0)));
        // Paste next to the original: the shared edge welds.
        let pasted = paste(
            &mut ents,
            &clip,
            DMat4::from_translation(DVec3::X * 10.0),
            |_| true,
        );
        assert_eq!(ents.faces.len(), 2);
        assert_eq!(ents.edges.len(), 7, "shared edge merged");
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
        assert_eq!(pasted.faces.len(), 1);
        assert_eq!(pasted.edges.len(), 4);
        for &e in &pasted.edges {
            let edge = &ents.edges[e];
            assert!(ents.pos(edge.start).x >= 10.0 - 1e-9);
            assert!(ents.pos(edge.end).x >= 10.0 - 1e-9);
        }
    }

    #[test]
    fn paste_onto_a_larger_face_splits_it() {
        let mut ents = Entities::new();
        square(&mut ents, DVec3::ZERO, 30.0);
        let mut small = Entities::new();
        square(&mut small, DVec3::ZERO, 10.0);
        let clip = copy(&small, &select_all(&small), &[], &[], DMat4::IDENTITY);
        paste(
            &mut ents,
            &clip,
            DMat4::from_translation(DVec3::new(10.0, 10.0, 0.0)),
            |_| true,
        );
        // The pasted square lies inside the big face: a hole face pair, as drawing does.
        assert_eq!(ents.faces.len(), 2);
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn paste_in_place_of_cut_geometry_round_trips() {
        let mut ents = Entities::new();
        square(&mut ents, DVec3::new(5.0, 5.0, 0.0), 10.0);
        let clip = copy(&ents, &select_all(&ents), &[], &[], DMat4::IDENTITY);
        let mut empty = Entities::new();
        let pasted = paste(&mut empty, &clip, DMat4::IDENTITY, |_| true);
        assert_eq!((empty.edges.len(), empty.faces.len()), (4, 1));
        assert_eq!((pasted.edges.len(), pasted.faces.len()), (4, 1));
        assert_eq!(validate(&empty, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn instances_and_annotations_are_copied_through_the_transform() {
        let mut model = crate::Model::default();
        square(&mut model.root, DVec3::ZERO, 10.0);
        let sel = select_all(&model.root);
        let i = crate::group::make_group(
            &mut model.definitions,
            &mut model.root,
            &sel,
            "Group#1".into(),
            true,
        )
        .unwrap();
        model
            .root
            .dimensions
            .push(Dimension::linear(DVec3::ZERO, DVec3::X, DVec3::Y));
        let to_world = DMat4::from_translation(DVec3::Z * 3.0);
        let sel = Selection {
            instances: vec![i],
            ..Default::default()
        };
        let clip = copy(&model.root, &sel, &[0], &[], to_world);
        assert_eq!(clip.instances.len(), 1);
        assert_eq!(clip.dimensions[0].a, DVec3::Z * 3.0);
        let (lo, _) = clip_bounds(&clip, &model.definitions).unwrap();
        assert_eq!(lo, DVec3::Z * 3.0);

        let mut dst = Entities::new();
        let pasted = paste(&mut dst, &clip, to_world.inverse(), |_| true);
        assert_eq!(pasted.instances, vec![0]);
        assert_eq!(pasted.dimensions, vec![0]);
        assert_eq!(
            dst.instances[0].transform,
            model.root.instances[i].transform
        );
        // A rejected definition is skipped.
        let pasted = paste(&mut dst, &clip, DMat4::IDENTITY, |_| false);
        assert!(pasted.instances.is_empty());
        assert_eq!(dst.instances.len(), 1);
    }
}
