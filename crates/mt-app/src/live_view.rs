//! What the viewport draws over the model for the selection and a running Move,
//! Rotate or Scale: the CPU side of `App::update_highlight` and `App::update_live`,
//! kept apart from the GPU upload so the large-model benchmark (`bench`) runs it.

use std::collections::HashMap;

use glam::{DMat3, DMat4, DVec3};
use mt_kernel::spatial::FxSet;
use mt_render::{FaceVertex, Segment};

use crate::document::{Document, Owner, Scene};
use crate::move_extra::Stretch;
use crate::selection::{self, Selection};

/// Inputs of [`Highlight::build`].
pub struct Highlight<'a> {
    pub scene: &'a Scene,
    pub selection: &'a Selection,
    pub document: Option<&'a Document>,
    /// The stretch of `live`; ignored without one.
    pub stretch: &'a Stretch,
    /// Compatibility > Bounding box only.
    pub box_only: bool,
    /// The running operation: targets, transform (context space), copy.
    pub live: Option<&'a (Vec<Owner>, DMat4, bool)>,
    pub ctx_xf: DMat4,
    pub inst_bounds: &'a HashMap<usize, (DVec3, DVec3)>,
    /// The pasted clipboard and where it is drawn.
    pub ghost: Option<(&'a Scene, DMat4)>,
    pub cut_lines: &'a [Segment],
}

impl Highlight<'_> {
    /// The edges to draw (model edges with selected ones colored, moved targets,
    /// selection boxes, the ghost, guides and cut lines), the selected faces'
    /// triangles (unindexed) and the placements whose edges are drawn instanced (their
    /// edges left out of the first list: those neither selected nor moving).
    pub fn build(&self) -> (Vec<Segment>, Vec<FaceVertex>, Vec<usize>) {
        let scene = self.scene;
        // Live targets: owner set, world-space transform, copy.
        let live = self.live.map(|(t, xf, copy)| {
            let set: FxSet<Owner> = t.iter().copied().collect();
            (set, self.ctx_xf * *xf * self.ctx_xf.inverse(), *copy)
        });
        let moved = |o: &Option<Owner>| match (&live, o) {
            (Some((set, xf, copy)), Some(o)) if set.contains(o) => Some((*xf, *copy)),
            _ => None,
        };
        let tp = |xf: DMat4, p: [f32; 3]| -> [f32; 3] {
            xf.transform_point3(glam::Vec3::from(p).as_dvec3())
                .as_vec3()
                .into()
        };
        // Styles > Modeling colors.
        let (sel_color, lock_color) = self.document.map_or((crate::SELECTED, crate::LOCKED), |d| {
            let mc = &d.model.style.modeling_colors;
            (
                crate::style_view::rgba(mc.selected),
                crate::style_view::rgba(mc.locked),
            )
        });
        let mut edges = Vec::with_capacity(scene.edges.len());
        let mut copies = Vec::new();
        // Locked groups and components are highlighted red instead of blue (only
        // instances lock; the active context is looked up once).
        let active = self.document.map(Document::active);
        let highlight = |o: &Option<Owner>| match (o, active) {
            (Some(o @ Owner::Instance(_)), Some(ents))
                if mt_kernel::visibility::is_locked(ents, o.item()) =>
            {
                lock_color
            }
            _ => sel_color,
        };
        // The stretch belongs to the live operation: without one it is stale.
        let stretching = self.live.is_some() && !self.stretch.owners.is_empty();
        // Compatibility > Bounding box only: selected groups and components show
        // their box, not their edges.
        let box_only = self.box_only;
        let selected = |o: Owner| {
            self.selection.contains(&o) && !(box_only && matches!(o, Owner::Instance(_)))
        };
        // Placements drawn instanced: left out here, except selected or moving ones.
        let mut instanced = Vec::new();
        let mut skip = Vec::new();
        for (j, p) in scene.instancing.placements.iter().enumerate() {
            let touched = p.owner.is_some_and(|o| {
                selected(o)
                    || moved(&Some(o)).is_some()
                    || (stretching && self.stretch.contains(&o))
            });
            if p.edges_instanced(scene) && !touched {
                instanced.push(j);
                skip.push(p.edges.clone());
            }
        }
        let mut from = 0;
        let runs = (skip.iter())
            .map(|r| {
                let run = from..r.start;
                from = r.end;
                run
            })
            .collect::<Vec<_>>()
            .into_iter()
            .chain(std::iter::once(from..scene.edges.len()));
        for k in runs.flatten() {
            let (seg, owner) = (&scene.edges[k], &scene.edge_owner[k]);
            let mut seg = *seg;
            let selected = owner.is_some_and(selected);
            let color = if selected {
                highlight(owner)
            } else {
                sel_color
            };
            if stretching
                && let Some(o) = owner
                && let (Some(a), Some(b)) = (self.stretch.map(o, seg.a), self.stretch.map(o, seg.b))
            {
                (seg.a, seg.b) = (a, b);
            }
            match moved(owner) {
                Some((xf, copy)) => {
                    let mut c = seg;
                    (c.a, c.b) = (tp(xf, c.a), tp(xf, c.b));
                    if selected {
                        c.color = color;
                    }
                    if copy {
                        copies.push(c);
                        if selected {
                            seg.color = color;
                        }
                    } else {
                        seg = c;
                    }
                }
                None if selected => seg.color = color,
                None => {}
            }
            edges.push(seg);
        }
        edges.extend(copies);
        // Selected faces' triangles. A face's triangles are consecutive: its owner is
        // looked up once.
        let mut selected = Vec::new();
        let mut last: Option<(Option<Owner>, bool)> = None;
        for (t, owner) in scene.tri_owner.iter().enumerate() {
            let hit = match last {
                Some((o, hit)) if o == *owner => hit,
                _ => {
                    let hit =
                        matches!(owner, Some(o @ Owner::Face(_)) if self.selection.contains(o));
                    last = Some((*owner, hit));
                    hit
                }
            };
            if hit {
                selected.push(t);
            }
        }
        let mut vertices: Vec<FaceVertex> = Vec::with_capacity(3 * selected.len());
        for t in selected {
            let owner = scene.tri_owner[t].expect("a selected face");
            let xf = moved(&Some(owner)).map(|(xf, _)| xf);
            for mut v in scene.tri_vertices(t) {
                if let Some(xf) = xf {
                    v.position = tp(xf, v.position);
                }
                if stretching {
                    v.position = self.stretch.map(&owner, v.position).unwrap_or(v.position);
                }
                vertices.push(v);
            }
        }
        // Selected components and groups show their bounding box in their own axes.
        if let Some(doc) = self.document {
            let ents = doc.active();
            for o in self.selection.items() {
                let Owner::Instance(i) = *o else { continue };
                let (Some(inst), Some((min, max))) =
                    (ents.instances.get(i), self.inst_bounds.get(&i))
                else {
                    continue;
                };
                let mut xf = doc.drawn_transform(self.ctx_xf * inst.transform, inst);
                if let Some((m, false)) = moved(&Some(*o)) {
                    xf = m * xf;
                }
                let b = selection::OrientedBox::from_aabb(*min, *max).transformed(xf);
                let color = if inst.locked { lock_color } else { sel_color };
                for [a, c] in b.edges() {
                    let f = |p: DVec3| p.as_vec3().to_array();
                    edges.push(Segment::line(f(a), f(c), color, [1.0, 0.0]));
                }
            }
        }
        if let Some((ghost, xf)) = self.ghost {
            edges.extend(ghost.edges.iter().map(|s| {
                let mut s = *s;
                (s.a, s.b) = (tp(xf, s.a), tp(xf, s.b));
                s
            }));
        }
        edges.extend(scene.guides.iter().copied());
        edges.extend(self.cut_lines.iter().copied());
        (edges, vertices, instanced)
    }
}

/// Indices of the scene's flattened triangles that stay in place while `targets` (and
/// the stretched faces) are drawn moved, and the placements that do.
pub fn kept_indices(scene: &Scene, targets: &[Owner], stretch: &Stretch) -> (Vec<u32>, Vec<usize>) {
    let set: FxSet<Owner> = targets.iter().copied().collect();
    let kept = |o: Option<Owner>| !o.is_some_and(|o| set.contains(&o) || stretch.contains(&o));
    let indices = scene
        .flat_tris()
        .filter(|&(t, _)| kept(scene.tri_owner[t]))
        .flat_map(|(_, i)| scene.indices[3 * i..3 * i + 3].iter().copied())
        .collect();
    (indices, scene.placements_where(kept))
}

/// The moved targets' and stretched faces' triangles (unindexed); `w` is the world
/// space transform.
pub fn preview_faces(
    scene: &Scene,
    targets: &[Owner],
    w: DMat4,
    stretch: &Stretch,
) -> Vec<FaceVertex> {
    let set: FxSet<Owner> = targets.iter().copied().collect();
    let n = DMat3::from_mat4(w).inverse().transpose();
    let f = |p: [f32; 3]| glam::Vec3::from(p).as_dvec3();
    let mut vertices = Vec::new();
    for (t, owner) in scene.tri_owner.iter().enumerate() {
        if let Some(o) = owner.filter(|o| stretch.contains(o)) {
            vertices.extend(stretch.triangle(&o, scene.tri_vertices(t)));
            continue;
        }
        if !owner.is_some_and(|o| set.contains(&o)) {
            continue;
        }
        for mut v in scene.tri_vertices(t) {
            v.position = w.transform_point3(f(v.position)).as_vec3().into();
            v.normal = (n * f(v.normal)).normalize_or_zero().as_vec3().into();
            vertices.push(v);
        }
    }
    vertices
}

/// Every selectable entity drawn, each once, in drawing order (Edit > Select All).
pub fn all_owners(scene: &Scene) -> Vec<Owner> {
    let mut seen = selection::OwnerSet::default();
    // A face's triangles are consecutive: repeats of the previous owner are skipped
    // before the lookup.
    let mut last = None;
    scene
        .tri_owner
        .iter()
        .chain(&scene.edge_owner)
        .filter(|o| std::mem::replace(&mut last, **o) != **o)
        .filter_map(|o| *o)
        .filter(|o| seen.insert(*o))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edges `Highlight` draws, as endpoint pairs.
    fn drawn(
        doc: &Document,
        scene: &Scene,
        live: Option<&(Vec<Owner>, DMat4, bool)>,
        stretch: &Stretch,
    ) -> Vec<([f32; 3], [f32; 3])> {
        let (edges, _, _) = Highlight {
            scene,
            selection: &Selection::default(),
            document: Some(doc),
            stretch,
            box_only: false,
            live,
            ctx_xf: DMat4::IDENTITY,
            inst_bounds: &HashMap::new(),
            ghost: None,
            cut_lines: &[],
        }
        .build();
        edges.iter().map(|s| (s.a, s.b)).collect()
    }

    /// Move a box's top edge, then undo: the box is drawn as it is, not with the
    /// edges at the moved corners stretched there as during the move. The app keeps
    /// the move's stretch until the next live operation; it used to draw it with no
    /// operation running, so after the undo the edges met the moved corners ("the box
    /// half restored", and after a second undo the rectangle "with the moved corner").
    #[test]
    fn a_finished_moves_stretch_is_not_drawn() {
        let mut doc = Document::untitled();
        doc.edit(|e| {
            let p = [(0.0, 0.0), (10.0, 0.0), (10.0, 8.0), (0.0, 8.0)]
                .map(|(x, y)| DVec3::new(x, y, 0.0));
            for i in 0..4 {
                mt_kernel::add_line(e, p[i], p[(i + 1) % 4]);
            }
            let f = e.faces.keys().next().unwrap();
            mt_kernel::push_pull::push_pull(e, f, -5.0, false).is_some()
        });
        let top = (doc.active().edges.keys())
            .find(|&k| {
                let (e, ents) = (&doc.active().edges[k], doc.active());
                ents.pos(e.start).z > 1.0 && ents.pos(e.end).z > 1.0
            })
            .unwrap();
        let box_scene = doc.scene();
        let box_edges = drawn(&doc, &box_scene, None, &Stretch::default());
        assert_eq!(box_edges.len(), 12);
        // The move as it runs: connected edges stretch.
        let live = (
            vec![Owner::Edge(top)],
            DMat4::from_translation(DVec3::new(3.0, 2.0, 1.0)),
            false,
        );
        let stretch =
            crate::move_extra::live_stretch(Some(&doc), &Some(live.clone()), DMat4::IDENTITY);
        assert!(!stretch.owners.is_empty());
        assert_ne!(drawn(&doc, &box_scene, Some(&live), &stretch), box_edges);
        // Committed, then undone: no operation runs, the stretch is the move's.
        doc.edit(|e| {
            crate::modify::apply(e, &live.0, live.1, false);
            true
        });
        assert!(doc.undo());
        let scene = doc.scene();
        assert_eq!(drawn(&doc, &scene, None, &stretch), box_edges);
    }

    /// The bit sets give what a hash set gives, for every corpus file (with hidden
    /// geometry shown: hidden faces add edges owned by their face).
    #[test]
    fn all_owners_matches_a_hash_set() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/openskp");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "skp") {
                continue;
            }
            let Ok(mut doc) = Document::open(&path) else {
                continue;
            };
            doc.model.style.hidden_geometry = true;
            let scene = doc.scene();
            let mut seen = FxSet::default();
            let want: Vec<Owner> = (scene.tri_owner.iter())
                .chain(&scene.edge_owner)
                .filter_map(|o| *o)
                .filter(|o| seen.insert(*o))
                .collect();
            assert_eq!(all_owners(&scene), want, "{}", path.display());
        }
    }
}
