//! The scene on the GPU, rewritten only where it changed (spec/PERFORMANCE.md,
//! "Fifth round").
//!
//! After an edit the scene says which of its vertices and edges are the previous
//! scene's, in place (`scene_cache::Changes`): only the others are written to the
//! GPU's buffers (`Viewport::update_faces`, `Viewport::write_edges`). The model's edge
//! buffer holds the scene's edges as drawn without a selection (those of placements
//! drawn instanced left out); the highlight rewrites only the edges it draws
//! otherwise (selected, moving, stretching: [`Highlight::build_sparse`]) and puts
//! back those it no longer does, and draws the rest of what it adds (moved copies,
//! boxes, selected placements' edges) as extra edges. The edges' bounds are kept per
//! block of the buffer.

use std::ops::Range;

use egui_wgpu::wgpu;
use glam::Vec3;
use mt_render::{Segment, Viewport};

use crate::document::Scene;
use crate::live_view::Highlight;

/// Edges per block of bounds.
const BLOCK: usize = 4096;

/// Selections or moves of more entities (and of more than an eighth of the edges)
/// are drawn as one buffer made in full (small in tests, to test both ways).
const WHOLE_MIN: usize = if cfg!(test) { 64 } else { 4096 };

type Bounds = Option<(Vec3, Vec3)>;

fn union(a: Bounds, b: Bounds) -> Bounds {
    match (a, b) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
        (x, None) | (None, x) => x,
    }
}

/// See the module documentation.
#[derive(Default)]
pub struct GpuScene {
    /// The scene whose faces are on the GPU (`Scene::build`; 0: none).
    faces: u64,
    /// The scene whose edges `base` holds.
    edges: u64,
    /// The model's edge buffer as drawn without a selection.
    base: Vec<Segment>,
    /// The scene's edges left out of `base` (placements drawn instanced), ascending,
    /// and how many are left out before each.
    skipped: Vec<Range<usize>>,
    skipped_before: Vec<usize>,
    /// Bounds of the model edges of each block of `base`.
    blocks: Vec<Bounds>,
    /// Edges of `base` drawn otherwise (the highlight), ascending, as drawn.
    shown: Vec<(usize, Segment)>,
    /// The edge buffer holds a whole highlight (a large selection) instead.
    whole: bool,
}

/// The edge ranges of the placements whose edges are drawn instanced.
fn skipped(scene: &Scene) -> Vec<Range<usize>> {
    (scene.instancing.placements.iter())
        .filter(|p| p.edges_instanced(scene))
        .map(|p| p.edges.clone())
        .collect()
}

impl GpuScene {
    /// Sends the scene's faces and edges, rewriting only what changed since the scene
    /// last sent (when `scene` was built from it). The highlight is drawn again by
    /// [`GpuScene::show`].
    pub fn upload(
        &mut self,
        scene: &Scene,
        viewport: &mut Viewport,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let ch = &scene.changes;
        let from_last = |sent: u64| sent != 0 && ch.from == Some(sent);
        // Faces: the vertices that changed; meshes when they did.
        let changed = match from_last(self.faces) {
            true => ch.vertices.changed(scene.vertices.len()),
            false => std::iter::once(0..scene.vertices.len()).collect(),
        };
        let changed: &[Range<usize>] = &changed;
        viewport.update_faces(device, queue, &scene.vertices, &scene.indices, changed);
        if !(from_last(self.faces) && ch.meshes) {
            let meshes: Vec<mt_render::InstanceMesh> = (scene.instancing.meshes.iter())
                .map(|m| mt_render::InstanceMesh {
                    vertices: &m.vertices,
                    indices: &m.indices,
                    edges: &m.edges,
                })
                .collect();
            viewport.set_instance_meshes(device, &meshes);
        }
        let placements = scene.gpu_instances(0..scene.instancing.placements.len());
        viewport.set_face_instances(device, &placements);
        self.faces = scene.build;

        // Edges.
        let skipped = skipped(scene);
        let left_out: usize = skipped.iter().map(Range::len).sum();
        if from_last(self.edges)
            && !self.whole
            && skipped == self.skipped
            && self.base.len() + left_out == scene.edge_count()
        {
            let mut dirty = Vec::new();
            for r in ch.edges.changed(scene.edge_count()) {
                for (run, at) in self.runs(r) {
                    let mut k = at;
                    scene.for_each_edge_in(run, |_, s| {
                        self.base[k] = s;
                        k += 1;
                    });
                    viewport.write_edges(queue, at, &self.base[at..k]);
                    dirty.push(at..k);
                }
            }
            // What the highlight drew there is gone (both lists ascend).
            let mut d = 0;
            self.shown.retain(|(k, _)| {
                while dirty.get(d).is_some_and(|r: &Range<usize>| r.end <= *k) {
                    d += 1;
                }
                !dirty.get(d).is_some_and(|r| r.contains(k))
            });
            for r in &dirty {
                for b in r.start / BLOCK..r.end.div_ceil(BLOCK) {
                    self.blocks[b] = self.block_bounds(b, &[]);
                }
            }
            viewport.set_edge_bounds(self.bounds(&[]));
        } else {
            self.base = scene.flat_edges().0;
            viewport.set_edges(device, &self.base);
            self.shown.clear();
            self.whole = false;
            self.blocks = (0..self.base.len().div_ceil(BLOCK))
                .map(|b| self.block_bounds(b, &[]))
                .collect();
            self.skipped_before = (skipped.iter())
                .scan(0, |n, r| {
                    let before = *n;
                    *n += r.len();
                    Some(before)
                })
                .collect();
            self.skipped = skipped;
        }
        self.edges = scene.build;
    }

    /// The parts of edge range `r` (scene numbering) in `base`, and where each starts
    /// there.
    fn runs(&self, r: Range<usize>) -> Vec<(Range<usize>, usize)> {
        let mut out = Vec::new();
        let mut e = r.start;
        let mut j = self.skipped.partition_point(|x| x.end <= e);
        while e < r.end {
            match self.skipped.get(j) {
                Some(x) if x.start <= e => {
                    e = x.end;
                    j += 1;
                }
                next => {
                    let end = next.map_or(r.end, |x| x.start.min(r.end));
                    let before = self.skipped_before.get(j).copied().unwrap_or_else(|| {
                        self.skipped_before.last().copied().unwrap_or(0)
                            + self.skipped.last().map_or(0, Range::len)
                    });
                    out.push((e..end, e - before));
                    e = end;
                }
            }
        }
        out
    }

    /// Where edge `e` (scene numbering) is in `base`, unless left out.
    fn base_index(&self, e: usize) -> Option<usize> {
        let j = self.skipped.partition_point(|x| x.end <= e);
        if self.skipped.get(j).is_some_and(|x| x.contains(&e)) {
            return None;
        }
        let before = match self.skipped_before.get(j) {
            Some(&b) => b,
            None => {
                self.skipped_before.last().copied().unwrap_or(0)
                    + self.skipped.last().map_or(0, Range::len)
            }
        };
        Some(e - before)
    }

    /// Bounds of block `b` of `base` with `shown` (ascending) in place of its edges.
    fn block_bounds(&self, b: usize, shown: &[(usize, Segment)]) -> Bounds {
        let r = b * BLOCK..((b + 1) * BLOCK).min(self.base.len());
        let mut bounds = None;
        let mut s = shown.partition_point(|(k, _)| *k < r.start);
        for k in r {
            let seg = match shown.get(s) {
                Some((at, seg)) if *at == k => {
                    s += 1;
                    seg
                }
                _ => &self.base[k],
            };
            if seg.flags & Segment::MODEL_EDGE != 0 {
                let (a, c) = (Vec3::from(seg.a), Vec3::from(seg.b));
                bounds = union(bounds, Some((a.min(c), a.max(c))));
            }
        }
        bounds
    }

    /// Bounds of `base` with `shown` in place.
    fn bounds(&self, shown: &[(usize, Segment)]) -> Bounds {
        // Blocks where a shown edge moved are bounded again.
        let mut moved: Vec<usize> = (shown.iter())
            .filter(|(k, s)| s.a != self.base[*k].a || s.b != self.base[*k].b)
            .map(|(k, _)| k / BLOCK)
            .collect();
        moved.dedup();
        let mut bounds = None;
        for (b, &block) in self.blocks.iter().enumerate() {
            let block = if moved.binary_search(&b).is_ok() {
                self.block_bounds(b, shown)
            } else {
                block
            };
            bounds = union(bounds, block);
        }
        bounds
    }

    /// Draws `highlight` (of the scene last uploaded): rewrites the edges it draws
    /// otherwise than last time, puts back those it no longer does, and sends its
    /// extra edges, selected faces and the placements drawn instanced.
    pub fn show(
        &mut self,
        highlight: &Highlight,
        viewport: &mut Viewport,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        // A large selection (Select All) or move: everything is drawn otherwise
        // anyway, as one buffer made in full.
        let many = highlight.selection.items().len() + highlight.live.map_or(0, |l| l.0.len());
        if many > WHOLE_MIN && many > self.base.len() / 8 {
            let (edges, faces, instanced) = highlight.build();
            viewport.set_edges(device, &edges);
            self.whole = true;
            self.shown.clear();
            self.finish(highlight, Vec::new(), faces, instanced, viewport, device);
            return;
        }
        if self.whole {
            // Back to the scene's edges.
            viewport.set_edges(device, &self.base);
            self.whole = false;
        }
        let sparse = highlight.build_sparse();
        let shown: Vec<(usize, Segment)> = (sparse.edges.into_iter())
            .filter_map(|(e, s)| Some((self.base_index(e)?, s)))
            .collect();
        // Writes: what is shown now and was not (or differently), and what was shown
        // and is no longer (put back).
        let mut writes: Vec<(usize, Segment)> = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < self.shown.len() || j < shown.len() {
            match (self.shown.get(i), shown.get(j)) {
                (Some(old), Some(new)) if old.0 == new.0 => {
                    if bytemuck::bytes_of(&old.1) != bytemuck::bytes_of(&new.1) {
                        writes.push(*new);
                    }
                    i += 1;
                    j += 1;
                }
                (Some(old), Some(new)) if old.0 > new.0 => {
                    writes.push(*new);
                    j += 1;
                }
                (Some(old), _) => {
                    writes.push((old.0, self.base[old.0]));
                    i += 1;
                }
                (None, Some(new)) => {
                    writes.push(*new);
                    j += 1;
                }
                (None, None) => break,
            }
        }
        // In runs of consecutive edges.
        let mut k = 0;
        while k < writes.len() {
            let start = writes[k].0;
            let mut run = vec![writes[k].1];
            k += 1;
            while k < writes.len() && writes[k].0 == start + run.len() {
                run.push(writes[k].1);
                k += 1;
            }
            viewport.write_edges(queue, start, &run);
        }
        viewport.set_edge_bounds(self.bounds(&shown));
        self.shown = shown;
        self.finish(
            highlight,
            sparse.extra,
            sparse.faces,
            sparse.instanced,
            viewport,
            device,
        );
    }

    /// Sends the rest of a highlight: extra edges, selected faces, edge instances.
    fn finish(
        &self,
        highlight: &Highlight,
        extra: Vec<Segment>,
        faces: Vec<mt_render::FaceVertex>,
        instanced: Vec<usize>,
        viewport: &mut Viewport,
        device: &wgpu::Device,
    ) {
        viewport.set_extra_edges(device, &extra);
        let indices: Vec<u32> = (0..faces.len() as u32).collect();
        viewport.set_selected_faces(device, &faces, &indices);
        let instances = highlight.scene.gpu_instances(instanced);
        viewport.set_edge_instances(device, &instances);
    }

    /// The model's edges as drawn now (`base` with the highlight in place), for tests.
    #[cfg(test)]
    pub fn drawn(&self) -> Vec<Segment> {
        let mut out = self.base.clone();
        for (k, s) in &self.shown {
            out[*k] = *s;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Owner};
    use crate::selection::{Mode, Selection};
    use glam::{DMat4, DVec3};

    fn bytes<T: bytemuck::Pod>(v: &[T]) -> &[u8] {
        bytemuck::cast_slice(v)
    }

    /// Kept between edits and selections, the edge buffer holds the scene's edges with
    /// the highlight's changes, as a full upload would, and its bounds are theirs.
    #[test]
    fn kept_edges_are_what_a_full_upload_holds() {
        let Some(g) = crate::bench::gpu() else {
            eprintln!("SKIPPED: no wgpu adapter");
            return;
        };
        let spec = mt_spec::Spec::builtin().unwrap();
        let style = mt_render::Style::from_spec(&spec.default_model);
        let mut vp = Viewport::new(&g.device, style);
        let mut doc = crate::scene_cache::tests::model();
        let mut scene = doc.scene_from(None);
        let mut gpu = GpuScene::default();
        type Edit = fn(&mut Document);
        let edits: [(&str, Edit); 6] = [
            ("first", |_| {}),
            ("move a face", |d| {
                let f = d.active().faces.keys().nth(200).unwrap();
                d.edit(|e| {
                    let xf = DMat4::from_translation(DVec3::Z);
                    crate::modify::apply(e, &[Owner::Face(f)], xf, false);
                    true
                });
            }),
            ("move an instance", |d| {
                d.edit(|e| {
                    let xf = DMat4::from_translation(DVec3::X);
                    crate::modify::apply(e, &[Owner::Instance(1)], xf, false);
                    true
                });
            }),
            ("move a tile", |d| {
                d.edit(|e| {
                    let xf = DMat4::from_translation(DVec3::Y);
                    crate::modify::apply(e, &[Owner::Instance(13)], xf, false);
                    true
                });
            }),
            ("erase", |d| {
                let e = d.active().edges.keys().nth(500).unwrap();
                d.erase(&[Owner::Edge(e)], false, false);
            }),
            ("color by tag", |d| d.model.color_by_tag = true),
        ];
        for (name, edit) in edits {
            edit(&mut doc);
            scene = doc.scene_from(Some(scene));
            gpu.upload(&scene, &mut vp, &g.device, &g.queue);
            assert!(bytes(&gpu.base) == bytes(&scene.flat_edges().0), "{name}");
            let e = doc.active().edges.keys().nth(50).unwrap();
            let selections = [
                vec![],
                vec![Owner::Edge(e)],
                vec![Owner::Instance(0), Owner::Instance(8)],
                crate::live_view::all_owners(&scene),
                vec![Owner::Edge(e)],
            ];
            for (k, sel) in selections.into_iter().enumerate() {
                let mut selection = Selection::default();
                selection.apply(Mode::Replace, sel.clone());
                let live = (k == 4).then(|| (sel, DMat4::from_translation(DVec3::Z * 9.0), false));
                let stretch = Default::default();
                let h = Highlight {
                    scene: &scene,
                    selection: &selection,
                    document: Some(&doc),
                    stretch: &stretch,
                    box_only: false,
                    live: live.as_ref(),
                    ctx_xf: DMat4::IDENTITY,
                    inst_bounds: &Default::default(),
                    ghost: None,
                    cut_lines: &[],
                };
                gpu.show(&h, &mut vp, &g.device, &g.queue);
                if gpu.whole {
                    // Sent as `build` makes it.
                    assert!(k == 3, "{name} {k}: only Select All is large");
                    continue;
                }
                let mut want = gpu.base.clone();
                for (e, s) in h.build_sparse().edges {
                    want[gpu.base_index(e).unwrap()] = s;
                }
                let drawn = gpu.drawn();
                assert!(bytes(&drawn) == bytes(&want), "{name} {k}");
                let bounds = gpu.bounds(&gpu.shown);
                assert_eq!(bounds, mt_render::model_edge_bounds(&drawn), "{name} {k}");
            }
        }
    }
}
