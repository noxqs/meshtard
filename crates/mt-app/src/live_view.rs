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

/// What the highlight draws for one scene and selection, worked out once
/// ([`Highlight::prepare`]).
struct Prepared<'a> {
    /// Live targets: owner set, world-space transform, copy.
    live: Option<(FxSet<Owner>, DMat4, bool)>,
    sel_color: [f32; 4],
    lock_color: [f32; 4],
    /// The open context (locked instances are highlighted red).
    active: Option<&'a mt_kernel::Entities>,
    /// The stretch of a running operation.
    stretching: bool,
}

/// The highlight as changes to the model's edges ([`Highlight::build_sparse`]).
pub struct Sparse {
    /// Model edges drawn otherwise than the scene has them: edge number and segment,
    /// ascending (those of placements drawn instanced are never here).
    pub edges: Vec<(usize, Segment)>,
    /// Edges drawn after the model's: placements drawn instanced that are selected or
    /// moving (flattened), moved copies, selection boxes, the ghost, guides and cut
    /// lines.
    pub extra: Vec<Segment>,
    /// The selected faces' triangles (unindexed).
    pub faces: Vec<FaceVertex>,
    /// The placements whose edges are drawn instanced.
    pub instanced: Vec<usize>,
}

fn tp(xf: DMat4, p: [f32; 3]) -> [f32; 3] {
    xf.transform_point3(glam::Vec3::from(p).as_dvec3())
        .as_vec3()
        .into()
}

impl<'a> Highlight<'a> {
    fn prepare(&self) -> Prepared<'a> {
        // Styles > Modeling colors.
        let (sel_color, lock_color) = self.document.map_or((crate::SELECTED, crate::LOCKED), |d| {
            let mc = &d.model.style.modeling_colors;
            (
                crate::style_view::rgba(mc.selected),
                crate::style_view::rgba(mc.locked),
            )
        });
        Prepared {
            live: self.live.map(|(t, xf, copy)| {
                let set: FxSet<Owner> = t.iter().copied().collect();
                (set, self.ctx_xf * *xf * self.ctx_xf.inverse(), *copy)
            }),
            sel_color,
            lock_color,
            active: self.document.map(Document::active),
            // The stretch belongs to the live operation: without one it is stale.
            stretching: self.live.is_some() && !self.stretch.owners.is_empty(),
        }
    }

    /// Where a moved owner goes (world-space transform) and whether it is copied.
    fn moved(p: &Prepared, o: &Option<Owner>) -> Option<(DMat4, bool)> {
        match (&p.live, o) {
            (Some((set, xf, copy)), Some(o)) if set.contains(o) => Some((*xf, *copy)),
            _ => None,
        }
    }

    /// Selected and highlighted (Compatibility > Bounding box only: selected groups
    /// and components show their box, not their edges).
    fn selected(&self, o: Owner) -> bool {
        self.selection.contains(&o) && !(self.box_only && matches!(o, Owner::Instance(_)))
    }

    /// Selected, moving or stretching: drawn otherwise than the scene has it.
    fn touched(&self, p: &Prepared, o: Owner) -> bool {
        self.selected(o)
            || Self::moved(p, &Some(o)).is_some()
            || (p.stretching && self.stretch.contains(&o))
    }

    /// Edge `k` (as the scene draws it: `seg`) as highlighted; a moved copy goes to
    /// `copies`.
    fn edge(&self, p: &Prepared, k: usize, mut seg: Segment, copies: &mut Vec<Segment>) -> Segment {
        let owner = &self.scene.edge_owner[k];
        let selected = owner.is_some_and(|o| self.selected(o));
        let color = match owner {
            // Locked groups and components are highlighted red instead of blue (only
            // instances lock).
            Some(o @ Owner::Instance(_))
                if selected
                    && p.active
                        .is_some_and(|ents| mt_kernel::visibility::is_locked(ents, o.item())) =>
            {
                p.lock_color
            }
            _ => p.sel_color,
        };
        if p.stretching
            && let Some(o) = owner
            && let (Some(a), Some(b)) = (self.stretch.map(o, seg.a), self.stretch.map(o, seg.b))
        {
            (seg.a, seg.b) = (a, b);
        }
        match Self::moved(p, owner) {
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
        seg
    }

    /// The drawn vertices of selected triangle `t`.
    fn face(&self, p: &Prepared, t: usize, out: &mut Vec<FaceVertex>) {
        let owner = self.scene.tri_owner[t].expect("a selected face");
        let xf = Self::moved(p, &Some(owner)).map(|(xf, _)| xf);
        for mut v in self.scene.tri_vertices(t) {
            if let Some(xf) = xf {
                v.position = tp(xf, v.position);
            }
            if p.stretching {
                v.position = self.stretch.map(&owner, v.position).unwrap_or(v.position);
            }
            out.push(v);
        }
    }

    /// Whether triangle `t` is of a selected face.
    fn face_selected(&self, t: usize) -> bool {
        matches!(self.scene.tri_owner[t], Some(o @ Owner::Face(_)) if self.selection.contains(&o))
    }

    /// The edges drawn after the model's that do not depend on it: selection boxes,
    /// the ghost, guides and cut lines.
    fn overlay(&self, p: &Prepared, edges: &mut Vec<Segment>) {
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
                if let Some((m, false)) = Self::moved(p, &Some(*o)) {
                    xf = m * xf;
                }
                let b = selection::OrientedBox::from_aabb(*min, *max).transformed(xf);
                let color = if inst.locked {
                    p.lock_color
                } else {
                    p.sel_color
                };
                for [a, c] in b.edges() {
                    let f = |p: DVec3| p.as_vec3().to_array();
                    edges.push(Segment::line(f(a), f(c), color, [1.0, 0.0]));
                }
            }
        }
        if let Some((ghost, xf)) = self.ghost {
            ghost.for_each_edge(|_, mut s| {
                (s.a, s.b) = (tp(xf, s.a), tp(xf, s.b));
                edges.push(s);
            });
        }
        edges.extend(self.scene.guides.iter().copied());
        edges.extend(self.cut_lines.iter().copied());
    }

    /// The placements drawn instanced (neither selected nor moving), and the edge
    /// ranges of those that are but whose edges are otherwise drawn instanced.
    fn placements(&self, p: &Prepared) -> (Vec<usize>, Vec<std::ops::Range<usize>>) {
        let scene = self.scene;
        let (mut instanced, mut touched) = (Vec::new(), Vec::new());
        for (j, x) in scene.instancing.placements.iter().enumerate() {
            if !x.edges_instanced(scene) {
                continue;
            }
            if x.owner.is_some_and(|o| self.touched(p, o)) {
                touched.push(x.edges.clone());
            } else {
                instanced.push(j);
            }
        }
        (instanced, touched)
    }

    /// The edges to draw (model edges with selected ones colored, moved targets,
    /// selection boxes, the ghost, guides and cut lines), the selected faces'
    /// triangles (unindexed) and the placements whose edges are drawn instanced (their
    /// edges left out of the first list: those neither selected nor moving). Looks
    /// at every edge and triangle: the app draws [`Highlight::build_sparse`] unless
    /// much is selected or moving.
    pub fn build(&self) -> (Vec<Segment>, Vec<FaceVertex>, Vec<usize>) {
        let scene = self.scene;
        let p = self.prepare();
        let mut edges = Vec::with_capacity(scene.edge_count());
        let mut copies = Vec::new();
        // Placements drawn instanced: left out here, except selected or moving ones.
        let (instanced, _) = self.placements(&p);
        let skip: Vec<std::ops::Range<usize>> = (instanced.iter())
            .map(|&j| scene.instancing.placements[j].edges.clone())
            .collect();
        let mut from = 0;
        let runs = (skip.iter())
            .map(|r| {
                let run = from..r.start;
                from = r.end;
                run
            })
            .collect::<Vec<_>>()
            .into_iter()
            .chain(std::iter::once(from..scene.edge_count()));
        for run in runs {
            scene.for_each_edge_in(run, |k, seg| edges.push(self.edge(&p, k, seg, &mut copies)));
        }
        edges.extend(copies);
        // Selected faces' triangles. A face's triangles are consecutive: its owner is
        // looked up once.
        let mut vertices = Vec::new();
        let mut last: Option<(Option<Owner>, bool)> = None;
        for (t, owner) in scene.tri_owner.iter().enumerate() {
            let hit = match last {
                Some((o, hit)) if o == *owner => hit,
                _ => {
                    let hit = self.face_selected(t);
                    last = Some((*owner, hit));
                    hit
                }
            };
            if hit {
                self.face(&p, t, &mut vertices);
            }
        }
        self.overlay(&p, &mut edges);
        (edges, vertices, instanced)
    }

    /// [`Highlight::build`] as changes to the scene's edges, looking only at the
    /// edges and triangles of the selected, moving and stretching entities (found
    /// with `Scene::context`): the model's edges with `edges` applied, then `extra`,
    /// are what `build` draws (the selected or moving placements' edges, which `build`
    /// draws in place, come first in `extra`).
    pub fn build_sparse(&self) -> Sparse {
        let scene = self.scene;
        let p = self.prepare();
        let (instanced, touched) = self.placements(&p);
        // Where the entities that are drawn otherwise are.
        let mut owners: Vec<Owner> = self.selection.items().to_vec();
        if let Some((set, _, _)) = &p.live {
            owners.extend(set.iter().copied());
        }
        if p.stretching {
            owners.extend(self.stretch.owners.keys().copied());
        }
        let (mut edge_ranges, tri_ranges) = scene.context_ranges(&owners);
        edge_ranges.extend(touched.iter().cloned());
        let edge_ranges = merged(edge_ranges);
        let mut edges = Vec::new();
        let mut placed = Vec::new();
        let mut copies = Vec::new();
        let mut t = 0;
        for r in edge_ranges {
            scene.for_each_edge_in(r, |k, seg| {
                while touched.get(t).is_some_and(|x| x.end <= k) {
                    t += 1;
                }
                if touched.get(t).is_some_and(|x| x.contains(&k)) {
                    placed.push(self.edge(&p, k, seg, &mut copies));
                } else if scene.edge_owner[k].is_some_and(|o| self.touched(&p, o)) {
                    edges.push((k, self.edge(&p, k, seg, &mut copies)));
                }
            });
        }
        let mut faces = Vec::new();
        for r in merged(tri_ranges) {
            for t in r {
                if self.face_selected(t) {
                    self.face(&p, t, &mut faces);
                }
            }
        }
        let mut extra = placed;
        extra.extend(copies);
        self.overlay(&p, &mut extra);
        Sparse {
            edges,
            extra,
            faces,
            instanced,
        }
    }
}

/// `ranges` sorted, overlapping and touching ones joined.
fn merged(mut ranges: Vec<std::ops::Range<usize>>) -> Vec<std::ops::Range<usize>> {
    ranges.retain(|r| !r.is_empty());
    ranges.sort_unstable_by_key(|r| r.start);
    let mut out: Vec<std::ops::Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

impl Scene {
    /// Ranges of edges and of triangles that hold every edge of `owners` and every
    /// triangle of its faces (with others), from `Scene::context`; everything when the
    /// scene was not built by `Document`.
    pub(crate) fn context_ranges(
        &self,
        owners: &[Owner],
    ) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
        use mt_kernel::slot_store::SlotStore;
        let c = &self.context;
        if !c.known {
            let all = |n: usize| std::iter::once(0..n).collect();
            return (all(self.edge_count()), all(self.tri_count()));
        }
        let mut edge_chunks = vec![false; c.edge_chunks.len()];
        let mut face_chunks = vec![false; c.face_chunks.len()];
        let mut instances = FxSet::default();
        let mut tail = false;
        for o in owners {
            match *o {
                Owner::Edge(e) => {
                    let k = SlotStore::<mt_kernel::EdgeId, ()>::chunk_of(e);
                    if let Some(x) = edge_chunks.get_mut(k) {
                        *x = true;
                    }
                }
                Owner::Face(f) => {
                    let k = SlotStore::<mt_kernel::FaceId, ()>::chunk_of(f);
                    if let Some(x) = face_chunks.get_mut(k) {
                        *x = true;
                    }
                }
                Owner::Instance(i) => {
                    instances.insert(i);
                }
                Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => tail = true,
            }
        }
        let mut edges = Vec::new();
        let mut tris = Vec::new();
        for (k, r) in c.edge_chunks.iter().enumerate() {
            if edge_chunks[k] {
                edges.push(r.clone());
            }
        }
        for (k, (e, t)) in c.face_chunks.iter().enumerate() {
            if face_chunks[k] {
                edges.push(e.clone());
                tris.push(t.clone());
            }
        }
        for (i, e, _) in &c.instances {
            if instances.contains(i) {
                edges.push(e.clone());
            }
        }
        if tail {
            edges.push(c.tail.0..self.edge_count());
        }
        (edges, tris)
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
    use crate::selection::Mode;

    fn bytes<T: bytemuck::Pod>(v: &[T]) -> &[u8] {
        bytemuck::cast_slice(v)
    }

    /// `build_sparse`'s changes to the model's edges, with its extra edges, give what
    /// `build` draws, for `sel` selected and `live` running in `doc`.
    fn sparse_matches_full(
        doc: &Document,
        sel: &[Owner],
        live: Option<(Vec<Owner>, DMat4, bool)>,
        what: &str,
    ) {
        let scene = doc.scene();
        let mut selection = Selection::default();
        selection.apply(Mode::Replace, sel.to_vec());
        let ctx_xf = doc.context_transform();
        let stretch = crate::move_extra::live_stretch(Some(doc), &live, ctx_xf);
        let ents = doc.active();
        let inst_bounds = crate::selection::instance_bounds(&scene, |i| {
            (ents.instances.get(i)).map(|inst| doc.drawn_transform(ctx_xf * inst.transform, inst))
        });
        let h = Highlight {
            scene: &scene,
            selection: &selection,
            document: Some(doc),
            stretch: &stretch,
            box_only: false,
            live: live.as_ref(),
            ctx_xf,
            inst_bounds: &inst_bounds,
            ghost: None,
            cut_lines: &[],
        };
        let (full, faces, instanced) = h.build();
        let sparse = h.build_sparse();
        assert_eq!(instanced, sparse.instanced, "{what}");
        assert!(bytes(&faces) == bytes(&sparse.faces), "{what}: faces");
        // The full list again, from the sparse one.
        let placed: Vec<(std::ops::Range<usize>, bool)> = (scene.instancing.placements.iter())
            .enumerate()
            .filter(|(_, p)| p.edges_instanced(&scene))
            .map(|(j, p)| (p.edges.clone(), !instanced.contains(&j)))
            .collect();
        let mut changed = sparse.edges.iter().peekable();
        let mut extra = sparse.extra.iter();
        let mut out = Vec::new();
        for e in 0..scene.edge_count() {
            match placed.iter().find(|(r, _)| r.contains(&e)) {
                Some((_, false)) => {}
                Some((_, true)) => out.push(*extra.next().expect("a placement's edge")),
                None => match changed.next_if(|(k, _)| *k == e) {
                    Some((_, s)) => out.push(*s),
                    None => out.push(scene.edge(e)),
                },
            }
        }
        assert!(changed.next().is_none(), "{what}: every change in order");
        out.extend(extra.copied());
        assert_eq!(out.len(), full.len(), "{what}");
        assert!(bytes(&out) == bytes(&full), "{what}: edges");
    }

    /// The highlight that looks only at the selected and moving entities draws what
    /// the one looking at every edge draws: no selection, single entities of every
    /// kind (an instanced placement, a locked one, a hidden face), everything, moves
    /// with and without a copy and with stretching, inside an open group.
    #[test]
    fn sparse_highlight_draws_what_the_full_one_draws() {
        let mut doc = crate::scene_cache::tests::model();
        doc.model.style.hidden_geometry = true;
        let f = doc.active().faces.keys().nth(40).unwrap();
        doc.hide(&[Owner::Face(f)]);
        doc.lock(&[Owner::Instance(3)], true);
        let e = doc.active().edges.keys().nth(100).unwrap();
        let g = doc.active().faces.keys().nth(300).unwrap();
        let all = all_owners(&doc.scene());
        let mv = DMat4::from_translation(DVec3::new(3.0, 1.0, 2.0));
        type Case = (&'static str, Vec<Owner>, Option<(Vec<Owner>, DMat4, bool)>);
        let cases: Vec<Case> = vec![
            ("nothing", vec![], None),
            ("an edge", vec![Owner::Edge(e)], None),
            ("a face", vec![Owner::Face(g)], None),
            ("a hidden face", vec![Owner::Face(f)], None),
            ("an instanced placement", vec![Owner::Instance(0)], None),
            ("a locked one", vec![Owner::Instance(3)], None),
            ("a group", vec![Owner::Instance(8)], None),
            ("everything", all.clone(), None),
            (
                "move a face",
                vec![Owner::Face(g)],
                Some((vec![Owner::Face(g)], mv, false)),
            ),
            (
                "copy an edge",
                vec![],
                Some((vec![Owner::Edge(e)], mv, true)),
            ),
            (
                "move a placement",
                vec![Owner::Instance(0)],
                Some((vec![Owner::Instance(0), Owner::Instance(5)], mv, false)),
            ),
            (
                "move everything",
                all.clone(),
                Some((all.clone(), mv, false)),
            ),
        ];
        for (name, sel, live) in &cases {
            sparse_matches_full(&doc, sel, live.clone(), name);
        }
        assert!(doc.open_instance(8));
        let e = doc.active().edges.keys().next().unwrap();
        let all = all_owners(&doc.scene());
        sparse_matches_full(&doc, &[Owner::Edge(e)], None, "in a group");
        sparse_matches_full(&doc, &all, Some((all.clone(), mv, true)), "copy in a group");
    }

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
