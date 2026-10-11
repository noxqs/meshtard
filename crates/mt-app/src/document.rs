//! The open model: loading a `.skp` into the kernel and turning it into renderer input.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use glam::{DMat3, DMat4, DVec3};
use mt_kernel::{
    DefinitionId, EdgeId, Entities, FaceId, MaterialId, Model, import, tessellate, texture,
    visibility,
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
    /// The rest of Model Info > Units (format, precision, snapping). Not written to
    /// the .skp: the writer keeps its scaffold's units.
    pub unit_options: mt_infer::units::UnitsOptions,
    /// The model and edit context before each edit, most recent last (Edit > Undo).
    undo: Vec<Snapshot>,
    /// Undone states, most recent last (Edit > Redo).
    redo: Vec<Snapshot>,
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
    /// What the last Edit > Hide hid, and the context it was in (Edit > Unhide > Last).
    /// Not part of undo: an undo can leave it naming other entities.
    last_hidden: Option<(Vec<usize>, Vec<Owner>)>,
}

/// A state Undo and Redo go back to: the model, the edit context, the state id and
/// the units.
type Snapshot = (Model, Vec<usize>, u64, Units);
type Units = (mt_infer::units::LengthUnit, mt_infer::units::UnitsOptions);

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

impl Owner {
    pub fn item(self) -> visibility::Item {
        use visibility::Item;
        match self {
            Self::Face(f) => Item::Face(f),
            Self::Edge(e) => Item::Edge(e),
            Self::Instance(i) => Item::Instance(i),
            Self::Section(i) => Item::Section(i),
            Self::Dimension(i) => Item::Dimension(i),
            Self::Text(i) => Item::Text(i),
        }
    }

    fn from_item(item: visibility::Item) -> Self {
        use visibility::Item;
        match item {
            Item::Face(f) => Self::Face(f),
            Item::Edge(e) => Self::Edge(e),
            Item::Instance(i) => Self::Instance(i),
            Item::Section(i) => Self::Section(i),
            Item::Dimension(i) => Self::Dimension(i),
            Item::Text(i) => Self::Text(i),
        }
    }
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
    /// Indices into `edges` of hidden entities drawn by View > Hidden Geometry (not
    /// snapped to).
    pub hidden_edges: HashSet<usize>,
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

/// What an instance passes to its contents.
#[derive(Clone, Copy, Default)]
struct Inherited {
    /// Paints faces without a material of their own.
    material: Option<MaterialId>,
    /// Inside a hidden instance (drawn only with View > Hidden Geometry).
    hidden: bool,
}

/// Edge color outside the open group or component (draft; SketchUp fades the rest of
/// the model, amount unverified).
const FADED_EDGE: [f32; 4] = [0.62, 0.62, 0.62, 1.0];

/// Lightens a face color for geometry outside the open context.
fn fade(c: [f32; 4]) -> [f32; 4] {
    let l = |x: f32| x + (1.0 - x) * 0.55;
    [l(c[0]), l(c[1]), l(c[2]), c[3]]
}

/// How View > Hidden Geometry draws hidden entities (drafts: SketchUp shows hidden
/// edges dashed and hidden faces as a faded mesh; colors and dash lengths unverified).
const HIDDEN_EDGE: [f32; 4] = [0.45, 0.45, 0.45, 1.0];
const HIDDEN_DASH: f32 = 4.0;
const HIDDEN_MESH_DASH: f32 = 2.0;
const HIDDEN_FACE_ALPHA: f32 = 0.35;

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
        let units = src
            .units
            .as_deref()
            .and_then(mt_infer::units::LengthUnit::from_name)
            .unwrap_or(mt_infer::units::LengthUnit::Inch);
        Ok(Self {
            path: path.to_owned(),
            model: import::from_skp(&src),
            units,
            saved_camera: src.camera,
            // The reader gets only the unit; whole millimeters as in SketchUp's
            // metric templates (unverified), two decimals otherwise.
            unit_options: mt_infer::units::UnitsOptions {
                precision: if units == mt_infer::units::LengthUnit::Millimeter {
                    0
                } else {
                    2
                },
                ..Default::default()
            },
            undo: Vec::new(),
            redo: Vec::new(),
            context: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
            has_file: true,
            last_hidden: None,
        })
    }

    /// A new empty model, as SketchUp shows at startup.
    pub fn untitled() -> Self {
        Self {
            path: PathBuf::from("Untitled.skp"),
            model: Model::default(),
            saved_camera: None,
            units: mt_infer::units::LengthUnit::Inch,
            unit_options: Default::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            context: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
            has_file: false,
            last_hidden: None,
        }
    }

    /// Id of the current model state (changes with every edit, undo and redo).
    pub fn state(&self) -> u64 {
        self.state
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
        // Locked groups and components cannot be opened.
        if self.active().instances.get(index).is_none_or(|i| i.locked) {
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
        let before = (
            self.model.clone(),
            self.context.clone(),
            self.state,
            (self.units, self.unit_options),
        );
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
            self.record(before);
            self.fix_context();
        }
        changed
    }

    /// An edit of the whole model (every definition, materials, tags) as one undo
    /// step, whatever context is open.
    pub fn edit_whole(&mut self, f: impl FnOnce(&mut Model) -> bool) -> bool {
        let before = (
            self.model.clone(),
            self.context.clone(),
            self.state,
            (self.units, self.unit_options),
        );
        let changed = f(&mut self.model);
        if changed {
            self.record(before);
            self.fix_context();
        }
        changed
    }

    /// Id of the current state: it changes with every edit, undo and redo.
    pub fn state_id(&self) -> u64 {
        self.state
    }

    fn record(&mut self, before: Snapshot) {
        self.undo.push(before);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.state = self.next_state;
        self.next_state += 1;
    }

    /// Model Info > Units: sets the unit and options as one undo step. Returns
    /// whether anything changed.
    pub fn set_units(
        &mut self,
        units: mt_infer::units::LengthUnit,
        options: mt_infer::units::UnitsOptions,
    ) -> bool {
        if (units, options) == (self.units, self.unit_options) {
            return false;
        }
        let before = (
            self.model.clone(),
            self.context.clone(),
            self.state,
            (self.units, self.unit_options),
        );
        self.units = units;
        self.unit_options = options;
        self.record(before);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        let now = self.restore(snapshot);
        self.redo.push(now);
        self.fix_context();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let now = self.restore(snapshot);
        self.undo.push(now);
        self.fix_context();
        true
    }

    /// Makes `snapshot` current; returns the state it replaced.
    fn restore(&mut self, (model, context, state, units): Snapshot) -> Snapshot {
        let old_units = (self.units, self.unit_options);
        (self.units, self.unit_options) = units;
        (
            std::mem::replace(&mut self.model, model),
            std::mem::replace(&mut self.context, context),
            std::mem::replace(&mut self.state, state),
            old_units,
        )
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
            two_point: false,
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
            Inherited::default(),
            Level::Context(&self.context),
            &mut active,
            &mut scene,
        );
        let f = |p: DVec3| p.as_vec3().to_array();
        self.section_planes(&mut scene);
        crate::annot::collect(self, &mut scene);
        let rest_hidden = !self.context.is_empty() && self.model.style.hide_rest_of_model;
        let guides = if self.model.style.guides && !rest_hidden {
            &root.guides[..]
        } else {
            &[]
        };
        for g in guides {
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
            Inherited::default(),
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
            if sp.hidden && !self.model.style.hidden_geometry {
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
        inherited: Inherited,
        level: Level,
        active: &mut HashSet<DefinitionId>,
        out: &mut Scene,
    ) {
        let show_hidden = self.model.style.hidden_geometry;
        // View > Component Edit > Hide Rest Of Model: only the open context's path is
        // walked outside it.
        let editing = !self.context.is_empty();
        let hide_rest = editing && self.model.style.hide_rest_of_model;
        if hide_rest && matches!(level, Level::Outside) {
            return;
        }
        let rest_here = hide_rest && matches!(level, Level::Context([_, ..]));
        // View > Component Edit > Hide Similar Components: other instances of the open
        // definition.
        let open_def = (editing && self.model.style.hide_similar_components)
            .then(|| Self::path_definitions(&self.model, &self.context))
            .flatten()
            .and_then(|d| d.last().copied());
        let ghost = inherited.hidden;
        let inherited = inherited.material;
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
            // Edges off: the renderer draws only the profiles among model edges, so
            // edges are left out only when profiles are off too. View > Hidden
            // Geometry shows hidden edges (and soft ones) dashed.
            let style = &self.model.style;
            let hidden = ghost || e.attrs.hidden || e.attrs.soft;
            if rest_here
                || (!style.edges && !style.profiles)
                || (hidden && !show_hidden)
                || !self.layer_visible(e.attrs.layer)
            {
                continue;
            }
            let mut seg = Segment::line(
                world(ents.pos(e.start)),
                world(ents.pos(e.end)),
                if hidden { HIDDEN_EDGE } else { edge_color },
                [EDGE_WIDTH, if hidden { HIDDEN_DASH } else { 0.0 }],
            );
            if hidden {
                out.hidden_edges.insert(out.edges.len());
                out.edges.push(seg);
                out.edge_owner.push(owner_of(Owner::Edge(id)));
                continue;
            }
            seg.flags = Segment::MODEL_EDGE;
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
            let hidden = ghost || face.attrs.hidden;
            if rest_here || (hidden && !show_hidden) || !self.layer_visible(face.attrs.layer) {
                continue;
            }
            let front_material = face.attrs.material.or(inherited);
            let back_material = face.attrs.back_material.or(inherited);
            let mut front = self.color(front_material, self.model.front_color);
            let mut back = self.color(back_material, self.model.back_color);
            if faded || hidden {
                front = fade(front);
                back = fade(back);
            }
            if hidden {
                front[3] = front[3].min(HIDDEN_FACE_ALPHA);
                back[3] = back[3].min(HIDDEN_FACE_ALPHA);
            }
            // Texture mapping per side, in the face's own coordinates. Faded geometry
            // (outside the open context) shows its lightened colors, not textures.
            let side = |m: Option<MaterialId>, is_back: bool| {
                let i = m.filter(|_| !faded && !hidden)?;
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
                if hidden {
                    // A hidden face shows as a mesh of its triangles.
                    for k in 0..3 {
                        let seg = Segment::line(
                            f32x3(tri[k]),
                            f32x3(tri[(k + 1) % 3]),
                            HIDDEN_EDGE,
                            [EDGE_WIDTH, HIDDEN_MESH_DASH],
                        );
                        out.hidden_edges.insert(out.edges.len());
                        out.edges.push(seg);
                        out.edge_owner.push(owner_of(Owner::Face(f)));
                    }
                }
                out.triangles.push(tri);
                out.tri_owner.push(owner_of(Owner::Face(f)));
            }
        }

        for (index, i) in ents.instances.iter().enumerate() {
            if (i.hidden && !show_hidden) || !self.layer_visible(i.layer) {
                continue;
            }
            let on_path = matches!(level, Level::Context([first, ..]) if *first == index);
            if (rest_here && !on_path) || (open_def == Some(i.definition) && !on_path) {
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
            let parent = Inherited {
                material: i.material.or(inherited),
                hidden: ghost || i.hidden,
            };
            self.collect(&def.entities, xf * i.transform, parent, child, active, out);
            active.remove(&i.definition);
        }
    }

    /// Deletes entities like SketchUp's Delete: edges are erased (their faces go with
    /// them, coplanar neighbors merge), faces are removed leaving their edges, components
    /// are removed.
    pub fn delete(&mut self, items: &[Owner]) {
        let items: Vec<Owner> = items
            .iter()
            .copied()
            .filter(|o| !self.is_locked(*o))
            .collect();
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
            // Locked groups and components are not erased.
            if root.instances.get(i).is_some_and(|x| !x.locked) {
                root.instances.remove(i);
            }
        }
    }

    /// The Eraser on picked entities. Erasing one segment of a curve erases the curve.
    /// `hide` (Shift) hides edges instead, `soften` (Ctrl) softens and smooths them,
    /// both together unsoften.
    pub fn erase(&mut self, items: &[Owner], hide: bool, soften: bool) {
        let items: Vec<Owner> = items
            .iter()
            .copied()
            .filter(|o| !self.is_locked(*o))
            .collect();
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

    /// Whether `o` is a locked group or component of the active context.
    pub fn is_locked(&self, o: Owner) -> bool {
        visibility::is_locked(self.active(), o.item())
    }

    /// Edit > Hide: hides `items` of the active context and remembers them for
    /// Edit > Unhide > Last.
    pub fn hide(&mut self, items: &[Owner]) -> bool {
        let items: Vec<_> = items.iter().map(|o| o.item()).collect();
        let mut changed = Vec::new();
        let done = self.edit(|ents| {
            changed = visibility::set_hidden(ents, &items, true);
            !changed.is_empty()
        });
        if done {
            let hidden = changed.into_iter().map(Owner::from_item).collect();
            self.last_hidden = Some((self.context.clone(), hidden));
        }
        done
    }

    /// Edit > Unhide > Selected.
    pub fn unhide(&mut self, items: &[Owner]) -> bool {
        let items: Vec<_> = items.iter().map(|o| o.item()).collect();
        self.edit(|ents| !visibility::set_hidden(ents, &items, false).is_empty())
    }

    /// Edit > Unhide > Last: shows what the last Edit > Hide hid, in the context it was
    /// hidden in (the open context stays as it is).
    pub fn unhide_last(&mut self) -> bool {
        let Some((context, items)) = self.last_hidden.clone() else {
            return false;
        };
        if Self::path_definitions(&self.model, &context).is_none() {
            return false;
        }
        let current = std::mem::replace(&mut self.context, context);
        let done = self.unhide(&items);
        self.context = current.clone();
        if done {
            // Undo returns to the context the command ran in.
            if let Some(step) = self.undo.last_mut() {
                step.1 = current;
            }
            self.last_hidden = None;
        }
        self.fix_context();
        done
    }

    /// Edit > Unhide > All: every hidden entity of the active context (not inside its
    /// groups and components; unverified against SketchUp).
    pub fn unhide_all(&mut self) -> bool {
        self.edit(visibility::unhide_all)
    }

    /// Edit > Lock and Edit > Unlock > Selected.
    pub fn lock(&mut self, items: &[Owner], locked: bool) -> bool {
        let items: Vec<_> = items.iter().map(|o| o.item()).collect();
        self.edit(|ents| visibility::set_locked(ents, &items, locked))
    }

    /// Edit > Unlock > All: every group and component in the model.
    pub fn unlock_all(&mut self) -> bool {
        self.edit_model(|model, ents| {
            let all: Vec<_> = (0..ents.instances.len())
                .map(visibility::Item::Instance)
                .collect();
            visibility::unlock_all(model) | visibility::set_locked(ents, &all, false)
        })
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
            two_point: false,
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
    fn units_change_is_one_undo_step() {
        use mt_infer::units::{LengthFormat, LengthUnit, UnitsOptions};
        let mut doc = Document::untitled();
        let opts = UnitsOptions {
            format: LengthFormat::Decimal,
            precision: 0,
            ..Default::default()
        };
        assert!(doc.set_units(LengthUnit::Millimeter, opts));
        assert!(!doc.set_units(LengthUnit::Millimeter, opts), "no change");
        assert!(doc.is_modified());
        assert!(doc.undo());
        assert_eq!(doc.units, LengthUnit::Inch);
        assert_eq!(doc.unit_options, UnitsOptions::default());
        assert!(!doc.can_undo());
        assert!(doc.redo());
        assert_eq!(
            (doc.units, doc.unit_options),
            (LengthUnit::Millimeter, opts)
        );
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

    /// A 10" square face at the root and a group made from a copy of it.
    fn square_and_group() -> Document {
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
        doc.edit(|root| {
            for i in 0..4 {
                let up = DVec3::Z * 20.0;
                mt_kernel::add_line(root, p[i] + up, p[(i + 1) % 4] + up);
            }
            true
        });
        doc
    }

    #[test]
    fn hide_unhide_with_undo() {
        let mut doc = square_and_group();
        let face = doc.model.root.faces.keys().next().unwrap();
        let visible_tris = doc.scene().triangles.len();
        assert!(doc.hide(&[Owner::Face(face), Owner::Instance(0)]));
        assert!(doc.scene().triangles.is_empty());
        assert!(
            !doc.hide(&[Owner::Face(face)]),
            "already hidden: no undo step"
        );
        // Hidden Geometry shows them, selectable, and marks their edges as hidden.
        doc.model.style.hidden_geometry = true;
        let scene = doc.scene();
        assert_eq!(scene.triangles.len(), visible_tris);
        assert!(scene.tri_owner.contains(&Some(Owner::Instance(0))));
        assert!(scene.tri_owner.contains(&Some(Owner::Face(face))));
        assert!(!scene.hidden_edges.is_empty());
        assert!(
            scene
                .hidden_edges
                .iter()
                .all(|&i| scene.edges[i].style[1] > 0.0),
            "hidden edges are dashed"
        );
        doc.model.style.hidden_geometry = false;
        assert!(doc.unhide(&[Owner::Instance(0)]));
        assert!(!doc.model.root.instances[0].hidden);
        assert!(doc.model.root.faces[face].attrs.hidden);
        // Unhide Last brings back the rest of the last hidden set.
        assert!(doc.unhide_last());
        assert!(!doc.model.root.faces[face].attrs.hidden);
        assert!(!doc.unhide_last(), "used up");
        assert!(doc.undo());
        assert!(doc.model.root.faces[face].attrs.hidden);
        assert!(doc.unhide_all());
        assert!(!doc.model.root.faces[face].attrs.hidden);
        assert!(!doc.unhide_all());
    }

    #[test]
    fn unhide_last_works_from_another_context() {
        let mut doc = square_and_group();
        assert!(doc.open_instance(0));
        let face = doc.active().faces.keys().next().unwrap();
        assert!(doc.hide(&[Owner::Face(face)]));
        doc.close_instance();
        assert!(doc.unhide_last());
        assert!(doc.context.is_empty(), "the open context does not change");
        let def = doc.model.root.instances[0].definition;
        assert!(!doc.model.definitions[def].entities.faces[face].attrs.hidden);
        assert!(doc.undo());
        assert!(doc.context.is_empty());
        assert!(doc.model.definitions[def].entities.faces[face].attrs.hidden);
    }

    #[test]
    fn hide_rest_of_model_and_similar_components() {
        let mut doc = square_and_group();
        // A second instance of the group's definition, moved aside.
        let mut other = doc.model.root.instances[0].clone();
        other.transform = DMat4::from_translation(DVec3::X * 50.0);
        doc.model.root.instances.push(other);
        let tris = |doc: &Document| doc.scene().triangles.len();
        let all = tris(&doc);
        doc.model.style.hide_rest_of_model = true;
        doc.model.style.hide_similar_components = true;
        assert_eq!(tris(&doc), all, "nothing hidden at the root");
        assert!(doc.open_instance(0));
        let face = 2; // one square: two triangles
        doc.model.style.hide_rest_of_model = false;
        assert_eq!(tris(&doc), all - face, "the other instance is hidden");
        doc.model.style.hide_similar_components = false;
        doc.model.style.hide_rest_of_model = true;
        assert_eq!(tris(&doc), face, "only the open group");
        doc.model.style.hide_rest_of_model = false;
        assert_eq!(tris(&doc), all, "faded, not hidden");
    }

    #[test]
    fn locked_instances_resist_edits() {
        let mut doc = square_and_group();
        assert!(doc.lock(&[Owner::Instance(0)], true));
        assert!(doc.is_locked(Owner::Instance(0)));
        assert!(!doc.open_instance(0), "a locked group does not open");
        doc.delete(&[Owner::Instance(0)]);
        assert_eq!(doc.model.root.instances.len(), 1, "not deleted");
        doc.erase(&[Owner::Instance(0)], false, false);
        assert_eq!(doc.model.root.instances.len(), 1, "not erased");
        assert!(doc.lock(&[Owner::Instance(0)], false));
        assert!(doc.lock(&[Owner::Instance(0)], true));
        assert!(doc.unlock_all());
        assert!(!doc.is_locked(Owner::Instance(0)));
        assert!(!doc.unlock_all());
        assert!(doc.undo());
        assert!(doc.is_locked(Owner::Instance(0)), "unlock is an undo step");
    }

    #[test]
    fn guides_toggle_hides_guides() {
        let mut doc = Document::untitled();
        doc.model.root.guides.push(mt_kernel::Guide {
            point: DVec3::ZERO,
            dir: Some(DVec3::X),
        });
        assert_eq!(doc.scene().guides.len(), 1);
        doc.model.style.guides = false;
        assert!(doc.scene().guides.is_empty());
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
