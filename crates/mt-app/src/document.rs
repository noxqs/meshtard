//! The open model: loading a `.skp` into the kernel and turning it into renderer input.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use glam::{DMat3, DMat4, DVec3};
use mt_kernel::{
    DefinitionId, EdgeId, Entities, FaceId, MaterialId, Model, import, tessellate, texture,
};
use mt_render::{Camera, FaceVertex, Segment};

/// Edge color and width of the default style. Draft; check against SketchUp 2026.
const EDGE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const EDGE_WIDTH: f32 = 1.0;

pub struct Document {
    pub path: PathBuf,
    pub model: Model,
    /// The view the file was saved with.
    saved_camera: Option<mt_skp::ViewCamera>,
    /// Unit for typed and displayed lengths.
    pub units: mt_infer::units::LengthUnit,
    /// The model and edit context before each edit, most recent last (Edit > Undo).
    undo: Vec<(Model, Vec<usize>, u64)>,
    /// Undone states, most recent last (Edit > Redo).
    redo: Vec<(Model, Vec<usize>, u64)>,
    /// Id of the current model state, and of the last saved one (unsaved changes when
    /// they differ).
    state: u64,
    saved_state: u64,
    next_state: u64,
    /// The document has a file of its own (File > Save writes there without asking).
    pub has_file: bool,
    /// The group or component open for editing: instance indices from the root down
    /// (empty: editing the model root).
    pub context: Vec<usize>,
}

/// Undo steps kept.
const UNDO_LIMIT: usize = 100;

/// A top-level entity of the active edit context: what a click selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    Face(FaceId),
    Edge(EdgeId),
    /// Index into the context's `instances`.
    Instance(usize),
    /// Index into the context's `section_planes`.
    Section(usize),
    /// Index into the context's `dimensions`.
    Dimension(usize),
    /// Index into the context's `texts`.
    Text(usize),
}

/// Everything the viewport draws for the model, with the owner of each piece.
#[derive(Default)]
pub struct Scene {
    pub edges: Vec<Segment>,
    /// `None` for geometry outside the open group or component (not selectable).
    pub edge_owner: Vec<Option<Owner>>,
    pub vertices: Vec<FaceVertex>,
    pub indices: Vec<u32>,
    /// World-space triangles for picking; triangle `i` is `indices[3 * i..3 * i + 3]`.
    pub triangles: Vec<[DVec3; 3]>,
    pub tri_owner: Vec<Option<Owner>>,
    /// Construction guides drawn as dashed lines (not pickable as edges).
    pub guides: Vec<Segment>,
    /// The active section cut in world space: point and normal (removed side).
    pub section: Option<(DVec3, DVec3)>,
    /// Dimension and text labels, drawn over the view (their lines are in `edges`).
    pub annotations: Vec<crate::annot::Annotation>,
    /// The textures `FaceVertex::texture` indexes, for `Viewport::set_textures`.
    pub textures: Vec<mt_render::TextureSource>,
    /// Index into `textures` of each model material (`FaceVertex::NO_TEXTURE` when
    /// untextured).
    texture_of: Vec<u32>,
}

/// Where entities being collected sit relative to the open context.
#[derive(Clone, Copy)]
enum Level<'a> {
    /// On the path to the context; the remaining instance indices (empty: this is it).
    Context(&'a [usize]),
    /// Inside a top-level instance of the context: selected as that instance.
    Inside(Owner),
    /// Elsewhere: faded and not selectable.
    Outside,
}

/// Edge color outside the open group or component (draft; SketchUp fades the rest of
/// the model, amount unverified).
const FADED_EDGE: [f32; 4] = [0.62, 0.62, 0.62, 1.0];

/// Lightens a face color for geometry outside the open context.
fn fade(c: [f32; 4]) -> [f32; 4] {
    let l = |x: f32| x + (1.0 - x) * 0.55;
    [l(c[0]), l(c[1]), l(c[2]), c[3]]
}

/// Section plane outline colors (drafts; SketchUp's are unverified).
const SECTION_COLOR: [f32; 4] = [0.85, 0.55, 0.2, 1.0];
const SECTION_ACTIVE: [f32; 4] = [0.95, 0.4, 0.05, 1.0];

/// Half-length of drawn guide lines, in inches.
const GUIDE_EXTENT: f64 = 1e5;
/// Guide color and dash length (draft).
const GUIDE_COLOR: [f32; 4] = [0.25, 0.25, 0.25, 1.0];

impl Document {
    pub fn open(path: &Path) -> Result<Self, mt_skp::Error> {
        let src = mt_skp::read(path)?;
        Ok(Self {
            path: path.to_owned(),
            model: import::from_skp(&src),
            units: src
                .units
                .as_deref()
                .and_then(mt_infer::units::LengthUnit::from_name)
                .unwrap_or(mt_infer::units::LengthUnit::Inch),
            saved_camera: src.camera,
            undo: Vec::new(),
            redo: Vec::new(),
            context: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
            has_file: true,
        })
    }

    /// A new empty model, as SketchUp shows at startup.
    pub fn untitled() -> Self {
        Self {
            path: PathBuf::from("Untitled.skp"),
            model: Model::default(),
            saved_camera: None,
            units: mt_infer::units::LengthUnit::Inch,
            undo: Vec::new(),
            redo: Vec::new(),
            context: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
            has_file: false,
        }
    }

    /// Whether the model changed since it was opened or last saved.
    pub fn is_modified(&self) -> bool {
        self.state != self.saved_state
    }

    /// Records that the current state was saved to `path`.
    pub fn mark_saved(&mut self, path: &Path) {
        self.path = path.to_owned();
        self.has_file = true;
        self.saved_state = self.state;
    }

    /// Definitions along a context path; `None` if the path is no longer valid.
    fn path_definitions(model: &Model, path: &[usize]) -> Option<Vec<DefinitionId>> {
        let mut ents = &model.root;
        let mut out = Vec::new();
        for &i in path {
            let d = ents.instances.get(i)?.definition;
            ents = &model.definitions.get(d)?.entities;
            out.push(d);
        }
        Some(out)
    }

    /// The entities being edited: the root, or the open group's or component's
    /// definition.
    pub fn active(&self) -> &Entities {
        match Self::path_definitions(&self.model, &self.context).and_then(|d| d.last().copied()) {
            Some(d) => &self.model.definitions[d].entities,
            None => &self.model.root,
        }
    }

    /// Context space to world space.
    pub fn context_transform(&self) -> DMat4 {
        let mut ents = &self.model.root;
        let mut xf = DMat4::IDENTITY;
        for &i in &self.context {
            let Some(inst) = ents.instances.get(i) else {
                break;
            };
            xf *= inst.transform;
            ents = &self.model.definitions[inst.definition].entities;
        }
        xf
    }

    /// Whether `def` is open for editing (or contains the open context): placing it
    /// in the open context would make it contain itself.
    pub fn in_context_path(&self, def: DefinitionId) -> bool {
        Self::path_definitions(&self.model, &self.context).is_some_and(|d| d.contains(&def))
    }

    /// Opens instance `index` of the active context for editing.
    pub fn open_instance(&mut self, index: usize) -> bool {
        if index >= self.active().instances.len() {
            return false;
        }
        self.context.push(index);
        true
    }

    /// Closes the open group or component (Edit > Close Group/Component).
    pub fn close_instance(&mut self) -> bool {
        self.context.pop().is_some()
    }

    fn fix_context(&mut self) {
        while Self::path_definitions(&self.model, &self.context).is_none() {
            self.context.pop();
        }
    }

    /// Runs an edit of the active context; when `f` reports a change, the state before
    /// it becomes an undo step.
    pub fn edit(&mut self, f: impl FnOnce(&mut Entities) -> bool) -> bool {
        self.edit_model(|_, ents| f(ents))
    }

    /// Like `edit`, with the rest of the model too. The active context's entities are
    /// taken out of the model while `f` runs (the model holds an empty one there).
    pub fn edit_model(&mut self, f: impl FnOnce(&mut Model, &mut Entities) -> bool) -> bool {
        let before = (self.model.clone(), self.context.clone(), self.state);
        let def =
            Self::path_definitions(&self.model, &self.context).and_then(|d| d.last().copied());
        let mut ents = match def {
            Some(d) => std::mem::take(&mut self.model.definitions[d].entities),
            None => std::mem::take(&mut self.model.root),
        };
        let changed = f(&mut self.model, &mut ents);
        match def.and_then(|d| self.model.definitions.get_mut(d)) {
            Some(d) => d.entities = ents,
            None if def.is_none() => self.model.root = ents,
            None => {}
        }
        if changed {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.state = self.next_state;
            self.next_state += 1;
            self.fix_context();
        }
        changed
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some((model, context, state)) = self.undo.pop() else {
            return false;
        };
        let now = (
            std::mem::replace(&mut self.model, model),
            std::mem::replace(&mut self.context, context),
            std::mem::replace(&mut self.state, state),
        );
        self.redo.push(now);
        self.fix_context();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((model, context, state)) = self.redo.pop() else {
            return false;
        };
        let now = (
            std::mem::replace(&mut self.model, model),
            std::mem::replace(&mut self.context, context),
            std::mem::replace(&mut self.state, state),
        );
        self.undo.push(now);
        self.fix_context();
        true
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned());
        // An asterisk marks unsaved changes (SketchUp's Windows title convention for
        // this is unverified).
        let star = if self.is_modified() { "*" } else { "" };
        format!("{star}{name} - meshtard")
    }

    /// The view the model was saved with, which SketchUp reopens at.
    pub fn saved_camera(&self) -> Option<Camera> {
        let c = self.saved_camera.as_ref()?;
        let mut camera = Camera {
            eye: c.eye.into(),
            target: c.target.into(),
            fov_deg: c.fov,
            perspective: !c.parallel,
        };
        // Parallel views store their visible height; our camera derives it from the
        // field of view at the target distance.
        let distance = camera.distance();
        if c.parallel && c.ortho_height > 0.0 && distance > 0.0 {
            camera.fov_deg = (2.0 * (c.ortho_height / 2.0 / distance).atan()).to_degrees();
        }
        Some(camera)
    }

    /// Visible faces and edges in world space. Hidden entities, entities on hidden tags,
    /// and soft edges are left out.
    pub fn scene(&self) -> Scene {
        let mut scene = Scene::default();
        for m in &self.model.materials {
            let layer = match &m.texture {
                Some(t) => {
                    scene.textures.push(mt_render::TextureSource {
                        data: t.data.clone(),
                        tint: t.colorize.then_some(m.color),
                    });
                    scene.textures.len() as u32 - 1
                }
                None => FaceVertex::NO_TEXTURE,
            };
            scene.texture_of.push(layer);
        }
        let mut active = HashSet::new();
        let root = &self.model.root;
        self.collect(
            root,
            DMat4::IDENTITY,
            None,
            Level::Context(&self.context),
            &mut active,
            &mut scene,
        );
        let f = |p: DVec3| p.as_vec3().to_array();
        self.section_planes(&mut scene);
        crate::annot::collect(self, &mut scene);
        for g in &root.guides {
            match g.dir {
                Some(d) => scene.guides.push(Segment::line(
                    f(g.point - d * GUIDE_EXTENT),
                    f(g.point + d * GUIDE_EXTENT),
                    GUIDE_COLOR,
                    [1.0, 3.0],
                )),
                None => {
                    // A small cross.
                    for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                        scene.guides.push(Segment::line(
                            f(g.point - axis * 0.5),
                            f(g.point + axis * 0.5),
                            GUIDE_COLOR,
                            [1.0, 0.0],
                        ));
                    }
                }
            }
        }
        scene
    }

    /// Faces and edges of entities that are not in the model (the clipboard while
    /// Paste places it), in their own coordinates, drawn with the model's materials,
    /// tags and style.
    pub fn ghost_scene(&self, ents: &Entities) -> Scene {
        let mut scene = Scene::default();
        // The same texture numbering as `scene` (the viewport holds those textures).
        let mut next = 0;
        scene.texture_of = self
            .model
            .materials
            .iter()
            .map(|m| match m.texture {
                Some(_) => {
                    next += 1;
                    next - 1
                }
                None => FaceVertex::NO_TEXTURE,
            })
            .collect();
        self.collect(
            ents,
            DMat4::IDENTITY,
            None,
            // Owners are not used: the ghost is not pickable.
            Level::Inside(Owner::Instance(usize::MAX)),
            &mut HashSet::new(),
            &mut scene,
        );
        scene
    }

    /// Section plane objects of the open context (selectable) and its active cut.
    fn section_planes(&self, scene: &mut Scene) {
        let ents = self.active();
        let xf = self.context_transform();
        let normal_xf = DMat3::from_mat4(xf).inverse().transpose();
        if self.model.style.section_cuts
            && let Some(sp) = ents.active_section.and_then(|i| ents.section_planes.get(i))
        {
            scene.section = Some((
                xf.transform_point3(sp.point),
                (normal_xf * sp.normal).normalize_or_zero(),
            ));
        }
        if !self.model.style.section_planes {
            return;
        }
        // Plane size from the model's extent (SketchUp sizes it to the model; exact
        // rule unverified).
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for t in &scene.triangles {
            for p in t {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
        }
        let half = if lo.is_finite() {
            ((hi - lo).length() * 0.35).max(12.0)
        } else {
            24.0
        };
        let f = |p: DVec3| p.as_vec3().to_array();
        for (i, sp) in ents.section_planes.iter().enumerate() {
            if sp.hidden {
                continue;
            }
            let c = xf.transform_point3(sp.point);
            let n = (normal_xf * sp.normal).normalize_or_zero();
            let (u, v) = n.any_orthonormal_pair();
            let corner = |a: f64, b: f64| c + (u * a + v * b) * half;
            let pts = [
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ];
            let active = ents.active_section == Some(i);
            let color = if active {
                SECTION_ACTIVE
            } else {
                SECTION_COLOR
            };
            let mut lines: Vec<(DVec3, DVec3)> =
                (0..4).map(|k| (pts[k], pts[(k + 1) % 4])).collect();
            // Arrows at two corners pointing to the kept side (the removed side would
            // clip them; SketchUp's arrow direction is unverified).
            for p in [pts[0], pts[2]] {
                let tip = p - n * half * 0.25;
                lines.push((p, tip));
                lines.push((tip, tip + n * half * 0.08 + u * half * 0.05));
                lines.push((tip, tip + n * half * 0.08 - u * half * 0.05));
            }
            for (a, b) in lines {
                scene
                    .edges
                    .push(Segment::line(f(a), f(b), color, [EDGE_WIDTH, 0.0]));
                scene.edge_owner.push(Some(Owner::Section(i)));
            }
        }
    }

    fn color(&self, m: Option<MaterialId>, fallback: [u8; 3]) -> [f32; 4] {
        let (rgb, a) = match m.and_then(|i| self.model.materials.get(i)) {
            Some(mat) => (mat.color, mat.opacity as f32),
            None => (fallback, 1.0),
        };
        [
            f32::from(rgb[0]) / 255.0,
            f32::from(rgb[1]) / 255.0,
            f32::from(rgb[2]) / 255.0,
            a,
        ]
    }

    fn layer_visible(&self, layer: Option<usize>) -> bool {
        layer.is_none_or(|l| self.model.layers.get(l).is_none_or(|x| x.visible))
    }

    fn collect(
        &self,
        ents: &Entities,
        xf: DMat4,
        inherited: Option<MaterialId>,
        level: Level,
        active: &mut HashSet<DefinitionId>,
        out: &mut Scene,
    ) {
        // Outside the open context: faded, not selectable.
        let faded = matches!(level, Level::Outside | Level::Context([_, ..]));
        let owner_of = |own: Owner| match level {
            Level::Context([]) => Some(own),
            Level::Inside(o) => Some(o),
            _ => None,
        };
        let edge_color = if faded { FADED_EDGE } else { EDGE_COLOR };
        let world = |p: DVec3| {
            let q = xf.transform_point3(p);
            [q.x as f32, q.y as f32, q.z as f32]
        };
        let normal_xf = DMat3::from_mat4(xf).inverse().transpose();
        let world_normal = |f: FaceId| (normal_xf * ents.faces[f].normal).normalize_or_zero();
        let f32x3 = |n: DVec3| [n.x as f32, n.y as f32, n.z as f32];

        for (id, e) in &ents.edges {
            // Styles > Edges off hides all edges (profiles too, in this draft).
            if !self.model.style.edges
                || e.attrs.hidden
                || e.attrs.soft
                || !self.layer_visible(e.attrs.layer)
            {
                continue;
            }
            let mut seg = Segment::line(
                world(ents.pos(e.start)),
                world(ents.pos(e.end)),
                edge_color,
                [EDGE_WIDTH, 0.0],
            );
            match e.faces[..] {
                [] => {}
                [f] => {
                    seg.n1 = f32x3(world_normal(f));
                    seg.profile = Segment::BORDER;
                }
                [f, g, ..] => {
                    seg.n1 = f32x3(world_normal(f));
                    seg.n2 = f32x3(world_normal(g));
                    seg.profile = Segment::SILHOUETTE;
                }
            }
            out.edges.push(seg);
            out.edge_owner.push(owner_of(Owner::Edge(id)));
        }

        for (f, face) in &ents.faces {
            if face.attrs.hidden || !self.layer_visible(face.attrs.layer) {
                continue;
            }
            let front_material = face.attrs.material.or(inherited);
            let back_material = face.attrs.back_material.or(inherited);
            let mut front = self.color(front_material, self.model.front_color);
            let mut back = self.color(back_material, self.model.back_color);
            if faded {
                front = fade(front);
                back = fade(back);
            }
            // Texture mapping per side, in the face's own coordinates. Faded geometry
            // (outside the open context) shows its lightened colors, not textures.
            let side = |m: Option<MaterialId>, is_back: bool| {
                let i = m.filter(|_| !faded)?;
                let layer = *out.texture_of.get(i)?;
                let map = texture::face_mapping(face, is_back, self.model.materials.get(i)?)?;
                // Shift by whole repeats so the numbers stay small in f32.
                let first = ents.loop_points(&face.outer).first().copied()?;
                let shift = texture::uv_at(&map, first).floor();
                Some((layer, map, shift))
            };
            let (front_tex, back_tex) = (side(front_material, false), side(back_material, true));
            let uv = |t: &Option<(u32, mt_skp::uv::UvMapping, glam::DVec2)>, p: DVec3| {
                t.as_ref().map_or([0.0; 2], |(_, map, shift)| {
                    (texture::uv_at(map, p) - *shift).as_vec2().to_array()
                })
            };
            let layer =
                |t: &Option<(u32, _, _)>| t.as_ref().map_or(FaceVertex::NO_TEXTURE, |t| t.0);
            let texture = [layer(&front_tex), layer(&back_tex)];
            let n = world_normal(f);
            for local in tessellate::triangulate(ents, f) {
                let [a, b, c] = local.map(|p| xf.transform_point3(p));
                // A mirroring transform flips the winding; keep it counter-clockwise
                // around the front normal.
                let order = if (b - a).cross(c - a).dot(n) < 0.0 {
                    [0, 2, 1]
                } else {
                    [0, 1, 2]
                };
                let world = [a, b, c];
                let tri = order.map(|k| world[k]);
                for k in order {
                    let [fu, fv] = uv(&front_tex, local[k]);
                    let [bu, bv] = uv(&back_tex, local[k]);
                    out.indices.push(out.vertices.len() as u32);
                    out.vertices.push(FaceVertex {
                        uv: [fu, fv, bu, bv],
                        texture,
                        ..FaceVertex::new(f32x3(world[k]), f32x3(n), front, back)
                    });
                }
                out.triangles.push(tri);
                out.tri_owner.push(owner_of(Owner::Face(f)));
            }
        }

        for (index, i) in ents.instances.iter().enumerate() {
            if i.hidden || !self.layer_visible(i.layer) {
                continue;
            }
            let Some(def) = self.model.definitions.get(i.definition) else {
                continue;
            };
            // A definition containing itself would recurse forever; skip it.
            if !active.insert(i.definition) {
                continue;
            }
            let child = match level {
                Level::Context([]) => Level::Inside(Owner::Instance(index)),
                Level::Context([first, rest @ ..]) if *first == index => Level::Context(rest),
                Level::Inside(o) => Level::Inside(o),
                _ => Level::Outside,
            };
            let material = i.material.or(inherited);
            self.collect(
                &def.entities,
                xf * i.transform,
                material,
                child,
                active,
                out,
            );
            active.remove(&i.definition);
        }
    }

    /// Deletes entities like SketchUp's Delete: edges are erased (their faces go with
    /// them, coplanar neighbors merge), faces are removed leaving their edges, components
    /// are removed.
    pub fn delete(&mut self, items: &[Owner]) {
        let items = items.to_vec();
        self.edit(|root| {
            Self::delete_in(root, &items);
            !items.is_empty()
        });
    }

    fn delete_in(root: &mut Entities, items: &[Owner]) {
        let mut instances: Vec<usize> = Vec::new();
        let mut sections: Vec<usize> = Vec::new();
        crate::annot::delete(root, items);
        for item in items {
            match *item {
                Owner::Section(i) => sections.push(i),
                Owner::Dimension(_) | Owner::Text(_) => {}
                Owner::Face(f) => {
                    root.remove_face(f);
                }
                Owner::Edge(e) => {
                    if root.edges.contains_key(e) {
                        mt_kernel::erase_edge(root, e);
                    }
                }
                Owner::Instance(i) => instances.push(i),
            }
        }
        sections.sort_unstable();
        sections.dedup();
        for i in sections.into_iter().rev() {
            if i < root.section_planes.len() {
                root.section_planes.remove(i);
                root.active_section = match root.active_section {
                    Some(a) if a == i => None,
                    Some(a) if a > i => Some(a - 1),
                    a => a,
                };
            }
        }
        instances.sort_unstable();
        instances.dedup();
        for i in instances.into_iter().rev() {
            if i < root.instances.len() {
                root.instances.remove(i);
            }
        }
    }

    /// The Eraser on picked entities. Erasing one segment of a curve erases the curve.
    /// `hide` (Shift) hides edges instead, `soften` (Ctrl) softens and smooths them,
    /// both together unsoften.
    pub fn erase(&mut self, items: &[Owner], hide: bool, soften: bool) {
        let items = items.to_vec();
        self.edit(|root| {
            Self::erase_in(root, &items, hide, soften);
            !items.is_empty()
        });
    }

    fn erase_in(root: &mut Entities, items: &[Owner], hide: bool, soften: bool) {
        let mut erase: Vec<Owner> = Vec::new();
        for item in items {
            match (*item, hide, soften) {
                (Owner::Edge(e), true, true) => {
                    if let Some(edge) = root.edges.get_mut(e) {
                        edge.attrs.soft = false;
                        edge.attrs.smooth = false;
                    }
                }
                (Owner::Edge(e), false, true) => {
                    if let Some(edge) = root.edges.get_mut(e) {
                        edge.attrs.soft = true;
                        edge.attrs.smooth = true;
                    }
                }
                (Owner::Edge(e), true, false) => {
                    if let Some(edge) = root.edges.get_mut(e) {
                        edge.attrs.hidden = true;
                    }
                }
                (Owner::Edge(e), false, false) => {
                    match root.edges.get(e).and_then(|x| x.attrs.curve) {
                        Some(c) => mt_kernel::erase_curve(root, c),
                        None => erase.push(*item),
                    }
                }
                (Owner::Instance(_) | Owner::Dimension(_) | Owner::Text(_), false, false) => {
                    erase.push(*item)
                }
                _ => {}
            }
        }
        Self::delete_in(root, &erase);
    }

    pub fn extents_camera(&self, fov_deg: f64) -> Option<Camera> {
        let (min, max) = crate::camera_cmd::scene_bounds(&self.scene())?;
        let center = (min + max) / 2.0;
        let radius = ((max - min).length() / 2.0).max(1.0);
        let distance = radius / (fov_deg.to_radians() / 2.0).sin();
        let dir = DVec3::new(-0.5, -0.8, 0.35).normalize();
        Some(Camera {
            eye: center + dir * distance,
            target: center,
            fov_deg,
            perspective: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/openskp")
            .join(name)
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut doc = Document::untitled();
        let a = DVec3::ZERO;
        let b = DVec3::new(10.0, 0.0, 0.0);
        assert!(!doc.edit(|_| false), "no change, no undo step");
        assert!(!doc.can_undo());
        doc.edit(|root| !mt_kernel::add_line(root, a, b).is_empty());
        assert_eq!(doc.model.root.edges.len(), 1);
        assert!(doc.undo());
        assert_eq!(doc.model.root.edges.len(), 0);
        assert!(doc.redo());
        assert_eq!(doc.model.root.edges.len(), 1);
        assert!(!doc.redo());
        doc.undo();
        doc.edit(|root| !mt_kernel::add_line(root, a, DVec3::Y).is_empty());
        assert!(!doc.can_redo(), "a new edit clears redo");
    }

    #[test]
    fn edit_inside_group_and_undo() {
        let mut doc = Document::untitled();
        let p = [
            DVec3::ZERO,
            DVec3::X * 10.0,
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::Y * 10.0,
        ];
        doc.edit(|root| {
            for i in 0..4 {
                mt_kernel::add_line(root, p[i], p[(i + 1) % 4]);
            }
            true
        });
        doc.edit_model(|model, ents| {
            let sel = mt_kernel::group::Selection {
                edges: ents.edges.keys().collect(),
                faces: ents.faces.keys().collect(),
                instances: vec![],
            };
            mt_kernel::group::make_group(&mut model.definitions, ents, &sel, "Group#1".into(), true)
                .is_some()
        });
        // Move the group so context space differs from world space.
        doc.model.root.instances[0].transform = DMat4::from_translation(DVec3::Z * 5.0);
        assert!(doc.open_instance(0));
        assert_eq!(
            doc.context_transform().transform_point3(DVec3::ZERO),
            DVec3::Z * 5.0
        );
        // A diagonal drawn inside splits the group's face, not the (empty) root.
        doc.edit(|ents| !mt_kernel::add_line(ents, p[0], p[2]).is_empty());
        assert_eq!(doc.active().faces.len(), 2);
        assert!(doc.model.root.faces.is_empty());
        let scene = doc.scene();
        assert!(
            scene
                .tri_owner
                .iter()
                .all(|o| matches!(o, Some(Owner::Face(_))))
        );
        assert!(
            scene
                .vertices
                .iter()
                .all(|v| (v.position[2] - 5.0).abs() < 1e-6)
        );
        doc.close_instance();
        assert!(
            doc.scene()
                .tri_owner
                .iter()
                .all(|o| *o == Some(Owner::Instance(0)))
        );
        assert!(doc.undo(), "undo the diagonal; reopens the group");
        assert_eq!(doc.context, vec![0]);
        assert_eq!(doc.active().faces.len(), 1);
    }

    #[test]
    fn opens_vff_file_with_saved_view() {
        let doc = Document::open(&corpus("Untitled.skp")).unwrap();
        assert_eq!(doc.title(), "Untitled.skp - meshtard");
        let cam = doc.saved_camera().unwrap();
        assert!((cam.eye.x - 452.09).abs() < 0.01);
        let scene = doc.scene();
        assert!(!scene.edges.is_empty());
        assert!(!scene.indices.is_empty());
        assert_eq!(scene.indices.len() % 3, 0);
    }

    #[test]
    fn instances_are_placed_in_world_space() {
        let doc = Document::open(&corpus("Untitled.skp")).unwrap();
        let root_only = doc.model.root.edges.len();
        assert!(doc.scene().edges.len() > root_only);
    }

    #[test]
    fn every_corpus_file_builds_a_scene() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/openskp");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|x| x == "skp") {
                let scene = Document::open(&path).unwrap().scene();
                assert!(
                    scene
                        .indices
                        .iter()
                        .all(|&i| (i as usize) < scene.vertices.len())
                );
            }
        }
    }
}
