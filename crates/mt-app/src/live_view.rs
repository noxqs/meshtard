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
    /// selection boxes, the ghost, guides and cut lines) and the selected faces'
    /// triangles (unindexed).
    pub fn build(&self) -> (Vec<Segment>, Vec<FaceVertex>) {
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
        // Locked groups and components are highlighted red instead of blue.
        let highlight = |o: &Option<Owner>| match (o, self.document) {
            (Some(o), Some(doc)) if doc.is_locked(*o) => lock_color,
            _ => sel_color,
        };
        // Compatibility > Bounding box only: selected groups and components show
        // their box, not their edges.
        let box_only = self.box_only;
        for (seg, owner) in scene.edges.iter().zip(&scene.edge_owner) {
            let mut seg = *seg;
            let selected = owner.is_some_and(|o| {
                self.selection.contains(&o) && !(box_only && matches!(o, Owner::Instance(_)))
            });
            let color = if selected {
                highlight(owner)
            } else {
                sel_color
            };
            if let Some(o) = owner
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
        let mut vertices: Vec<FaceVertex> = Vec::new();
        for (t, owner) in scene.tri_owner.iter().enumerate() {
            let Some(owner @ Owner::Face(_)) = owner.filter(|o| self.selection.contains(o)) else {
                continue;
            };
            let xf = moved(&Some(owner)).map(|(xf, _)| xf);
            for k in 0..3 {
                let mut v = scene.vertices[scene.indices[3 * t + k] as usize];
                if let Some(xf) = xf {
                    v.position = tp(xf, v.position);
                }
                v.position = self.stretch.map(&owner, v.position).unwrap_or(v.position);
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
        (edges, vertices)
    }
}

/// Indices of the scene's triangles that stay in place while `targets` (and the
/// stretched faces) are drawn moved.
pub fn kept_indices(scene: &Scene, targets: &[Owner], stretch: &Stretch) -> Vec<u32> {
    let set: FxSet<Owner> = targets.iter().copied().collect();
    scene
        .indices
        .chunks(3)
        .zip(&scene.tri_owner)
        .filter(|(_, o)| !o.is_some_and(|o| set.contains(&o) || stretch.contains(&o)))
        .flat_map(|(t, _)| t.iter().copied())
        .collect()
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
            let tri = [0, 1, 2].map(|k| scene.vertices[scene.indices[3 * t + k] as usize]);
            vertices.extend(stretch.triangle(&o, tri));
            continue;
        }
        if !owner.is_some_and(|o| set.contains(&o)) {
            continue;
        }
        for k in 0..3 {
            let mut v = scene.vertices[scene.indices[3 * t + k] as usize];
            v.position = w.transform_point3(f(v.position)).as_vec3().into();
            v.normal = (n * f(v.normal)).normalize_or_zero().as_vec3().into();
            vertices.push(v);
        }
    }
    vertices
}

/// Every selectable entity drawn, each once, in drawing order (Edit > Select All).
pub fn all_owners(scene: &Scene) -> Vec<Owner> {
    let mut seen = FxSet::default();
    scene
        .tri_owner
        .iter()
        .chain(&scene.edge_owner)
        .filter_map(|o| *o)
        .filter(|o| seen.insert(*o))
        .collect()
}
