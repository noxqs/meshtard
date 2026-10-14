//! The open model: loading a `.skp` into the kernel and turning it into renderer input.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use glam::{DMat3, DMat4, DVec3};
use mt_kernel::spatial::{FxMap, FxSet};
use mt_kernel::{
    DefinitionId, EdgeId, Entities, FaceId, MaterialId, Model, import, tessellate, texture,
    visibility,
};
use mt_render::{Camera, FaceVertex, Segment};

/// Edge width of the default style (the color is the style's, see `style_view`).
/// Draft; check against SketchUp 2026.
const EDGE_WIDTH: f32 = 1.0;

pub struct Document {
    pub path: PathBuf,
    pub model: Model,
    /// The view the file was saved with.
    saved_camera: Option<mt_skp::ViewCamera>,
    /// Unit for typed and displayed lengths.
    pub units: mt_infer::units::LengthUnit,
    /// The rest of Model Info > Units (format, precision, snapping). Saved with the
    /// unit in the file's `UnitsOptions` (see [`units_to_options`]).
    pub unit_options: mt_infer::units::UnitsOptions,
    /// The model and edit context before each edit, most recent last (Edit > Undo).
    undo: Vec<Snapshot>,
    /// Copies of entities collections that undo steps share (see `undo`).
    shared: crate::undo::Shared,
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
    /// Operation names of the undo and redo steps, in step with `undo` and `redo`
    /// (Edit > Undo shows "Undo Move").
    undo_names: Vec<String>,
    redo_names: Vec<String>,
    /// Name given to the next recorded step: the app sets it to the running tool or
    /// command before edits happen.
    pub operation: String,
    /// Where the view is, for components that always face the camera (and the sun
    /// for their shadows). Not part of the model: the app sets it before building
    /// the scene; `None` draws them as placed.
    pub view: Option<crate::component_behavior::Facing>,
    /// Dimensions hidden for the current view by Model Info > Dimensions > Expert
    /// Settings, as sorted (open context or not, index) keys (see `annot`). Set by the
    /// app before the scene is built; not part of the model.
    pub dims_hidden: Vec<(bool, usize)>,
    /// Faces' triangles from earlier scene builds, reused while the face is unchanged.
    tess: std::sync::Mutex<crate::tess_cache::TessCache>,
    /// What the last [`Document::scene_from`] drew, per collection and chunk.
    pieces: std::sync::Mutex<crate::scene_cache::SceneCache>,
}

/// A state Undo and Redo go back to: the model, the edit context, the state id and
/// the units.
type Snapshot = (crate::undo::Step, Vec<usize>, u64, Units);
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
    /// The edges in world space, in drawing order ([`Scene::edge`]), except those of
    /// placements that give them from their mesh (`instancing`, `Placement::lazy_edges`).
    pub(crate) edge_list: Vec<Segment>,
    /// The owner of every edge (also the number of edges, [`Scene::edge_count`]).
    /// `None` for geometry outside the open group or component (not selectable).
    pub edge_owner: Vec<Option<Owner>>,
    pub vertices: Vec<FaceVertex>,
    pub indices: Vec<u32>,
    /// World-space triangles for picking ([`Scene::tri`]) that are not instanced:
    /// entry `i` is `indices[3 * i..3 * i + 3]`. Instanced triangles are given by their
    /// placement and mesh.
    pub(crate) tri_list: Vec<[DVec3; 3]>,
    /// The owner of every triangle (also the number of triangles, [`Scene::tri_count`]).
    pub tri_owner: Vec<Option<Owner>>,
    /// Front and back material shown on each triangle (inherited from instances
    /// included), for the Paint Bucket's Alt sampling.
    pub tri_material: Vec<[Option<MaterialId>; 2]>,
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
    pub hidden_edges: FxSet<usize>,
    /// The group or component (relative to the open context) of edges and triangles
    /// inside one, by index into `edges` / `triangles`, for the inference tooltip
    /// ("Endpoint in Group"). Shorter than `edges` / `triangles` when the last ones are
    /// in none (missing entries are `None`).
    pub edge_container: Vec<Option<mt_infer::Container>>,
    pub tri_container: Vec<Option<mt_infer::Container>>,
    /// Index into `textures` of each model material (`FaceVertex::NO_TEXTURE` when
    /// untextured).
    pub(crate) texture_of: Vec<u32>,
    /// Vertex ranges of face-camera components with "Shadows face sun", and the
    /// rotation turning each from the camera to the sun (see [`Scene::shadow_casters`]).
    pub sun_facing: Vec<(std::ops::Range<usize>, DMat4)>,
    /// Groups and components drawn per instance (see `instancing`): their triangles
    /// and edges are numbered with the others ([`Scene::tri`], [`Scene::edge`]) but
    /// not stored, and not in `vertices` / `indices`.
    pub instancing: crate::instancing::Instancing,
    /// Culling index over the edges for picking, built on first use (see
    /// [`Scene::edges_near`]).
    edge_index: std::sync::OnceLock<crate::pick_index::EdgeIndex>,
    /// Every edge and triangle in world space, made on first use
    /// ([`Scene::edges`], [`Scene::triangles`]).
    pub(crate) all_edges: std::sync::OnceLock<Vec<Segment>>,
    pub(crate) all_tris: std::sync::OnceLock<Vec<[DVec3; 3]>>,
    /// Where the open context's entities are drawn (see [`ContextIndex`]).
    pub(crate) context: ContextIndex,
    /// The previous scene's edge index, for this one's to keep what it can
    /// (`pick_index::EdgeSeed`).
    pub(crate) edge_seed: Option<crate::pick_index::EdgeSeed>,
    /// This build's id (0: not built by `Document`), see `scene_cache`.
    pub(crate) build: u64,
    /// What this scene kept of the one it was built from ([`Document::scene_from`]).
    pub changes: crate::scene_cache::Changes,
}

/// A [`Scene`] while `Document::collect` draws it: its per-edge, per-triangle and
/// per-vertex lists take ranges of the previous scene's without copying them yet
/// (`scene_cache::List`). The indices are not kept: they are always `0..n`.
#[derive(Default)]
pub(crate) struct SceneBuild {
    pub edge_list: crate::scene_cache::List<Segment>,
    pub edge_owner: crate::scene_cache::List<Option<Owner>>,
    pub vertices: crate::scene_cache::List<FaceVertex>,
    pub tri_list: crate::scene_cache::List<[DVec3; 3]>,
    pub tri_owner: crate::scene_cache::List<Option<Owner>>,
    pub tri_material: crate::scene_cache::List<[Option<MaterialId>; 2]>,
    pub guides: Vec<Segment>,
    pub hidden_edges: FxSet<usize>,
    pub edge_container: Vec<Option<mt_infer::Container>>,
    pub tri_container: Vec<Option<mt_infer::Container>>,
    pub texture_of: Vec<u32>,
    pub sun_facing: Vec<(std::ops::Range<usize>, DMat4)>,
    pub instancing: crate::instancing::Instancing,
    pub context: ContextIndex,
}

impl SceneBuild {
    /// Adds an edge (world space) and its owner.
    fn push_edge(&mut self, seg: Segment, owner: Option<Owner>) {
        self.edge_list.push(seg);
        self.edge_owner.push(owner);
    }

    /// Adds a triangle (world space) and its owner.
    fn push_tri(&mut self, tri: [DVec3; 3], owner: Option<Owner>) {
        self.tri_list.push(tri);
        self.tri_owner.push(owner);
    }

    /// The scene drawn, its lists taken from `prev` (the previous scene's) where they
    /// are kept.
    fn finish(self, prev: &mut Scene, scene: &mut Scene) {
        scene.edge_list = self.edge_list.finish(&mut prev.edge_list);
        scene.edge_owner = self.edge_owner.finish(&mut prev.edge_owner);
        scene.vertices = self.vertices.finish(&mut prev.vertices);
        scene.tri_list = self.tri_list.finish(&mut prev.tri_list);
        scene.tri_owner = self.tri_owner.finish(&mut prev.tri_owner);
        scene.tri_material = self.tri_material.finish(&mut prev.tri_material);
        // Always 0..n: the previous scene's are kept as far as they go.
        let n = scene.vertices.len();
        let mut indices = std::mem::take(&mut prev.indices);
        indices.truncate(n);
        indices.extend(indices.len() as u32..n as u32);
        scene.indices = indices;
        scene.guides = self.guides;
        scene.hidden_edges = self.hidden_edges;
        scene.edge_container = self.edge_container;
        scene.tri_container = self.tri_container;
        scene.texture_of = self.texture_of;
        scene.sun_facing = self.sun_facing;
        scene.instancing = self.instancing;
        scene.context = self.context;
    }
}

/// Where the open context's entities are drawn in a [`Scene`], so the highlight finds
/// the edges and triangles of a few selected entities without looking at all of them
/// (`live_view`). Ranges are of edge and triangle numbers ([`Scene::edge`],
/// [`Scene::tri`]); an entity's are all inside its range (with others).
#[derive(Default, Clone, Debug)]
pub(crate) struct ContextIndex {
    /// Per chunk of the context's edges (`mt_kernel::slot_store`): the edges drawn.
    pub edge_chunks: Vec<std::ops::Range<usize>>,
    /// Per chunk of its faces: the edges (hidden faces' outlines) and triangles drawn.
    pub face_chunks: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
    /// Its groups and components drawn: instance index, edges, triangles (in order).
    pub instances: Vec<(usize, std::ops::Range<usize>, std::ops::Range<usize>)>,
    /// The first edge and triangle after the model's (section planes, dimensions and
    /// texts).
    pub tail: (usize, usize),
    /// Filled in (the scene was built by `Document`).
    pub known: bool,
}

/// Ids of scene builds made so far.
static BUILDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Records that entry `i` of `edges` or `triangles` lies in container `c`.
fn set_container(list: &mut Vec<Option<mt_infer::Container>>, i: usize, c: mt_infer::Container) {
    if list.len() <= i {
        list.resize(i + 1, None);
    }
    list[i] = Some(c);
}

/// Records that entries `range` of `edges` or `triangles` lie in container `c`.
pub(crate) fn set_containers(
    list: &mut Vec<Option<mt_infer::Container>>,
    range: std::ops::Range<usize>,
    c: mt_infer::Container,
) {
    if list.len() < range.end {
        list.resize(range.end, None);
    }
    list[range].fill(Some(c));
}

/// Below this many edges picking tests every one (no index).
const EDGE_INDEX_MIN: usize = 1024;

impl Scene {
    /// Indices (ascending) of the edges that may pass within `radius` pixels of
    /// `cursor`: every edge that does is included. `None` means all edges.
    pub fn edges_near(
        &self,
        camera: &Camera,
        viewport: glam::DVec2,
        cursor: glam::DVec2,
        radius: f64,
    ) -> Option<Vec<usize>> {
        if self.edge_count() < EDGE_INDEX_MIN {
            return None;
        }
        let index = self
            .edge_index
            .get_or_init(|| crate::pick_index::EdgeIndex::build(self));
        // Edges added after the index was built: test them all.
        if index.len() != self.edge_count() {
            return None;
        }
        index.near(self, camera, viewport, cursor, radius)
    }

    /// Adds an edge (world space) and its owner.
    pub fn push_edge(&mut self, seg: Segment, owner: Option<Owner>) {
        self.edge_list.push(seg);
        self.edge_owner.push(owner);
    }

    /// Adds a triangle (world space) and its owner.
    #[cfg(test)]
    pub fn push_tri(&mut self, tri: [DVec3; 3], owner: Option<Owner>) {
        self.tri_list.push(tri);
        self.tri_owner.push(owner);
    }
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

/// State of one walk over the model ([`Document::collect`]).
#[derive(Default)]
struct Walk<'a> {
    /// Triangles kept between scene builds (`None`: compute them all).
    tess: Option<&'a mut crate::tess_cache::TessCache>,
    /// Definitions being walked (a definition containing itself is skipped).
    active: HashSet<DefinitionId>,
    /// Times each entities collection (by address) has been reached so far.
    visits: FxMap<usize, u32>,
    /// Triangles of the faces of collections reached more than once (component
    /// definitions placed many times), in their own coordinates: tessellated once.
    triangles: FxMap<(usize, FaceId), Vec<[DVec3; 3]>>,
    /// Instancing (`None`: everything flattened).
    instancing: Option<crate::instancing::Builder>,
    /// The mesh the next collection walked is a placement of, if instanced.
    next: Option<crate::instancing::Key>,
    /// Pieces kept from the previous build (`None`: draw everything).
    pieces: Option<crate::scene_cache::Pieces>,
}

/// What an instance passes to its contents.
#[derive(Clone, Copy, Default)]
struct Inherited {
    /// Paints faces without a material of their own.
    material: Option<MaterialId>,
    /// Inside a hidden instance (drawn only with View > Hidden Geometry).
    hidden: bool,
    /// Inside a group or component that is not on the open context's path (the
    /// outermost such instance).
    container: Option<mt_infer::Container>,
    /// The tag of the innermost tagged instance around: untagged entities take its
    /// color with Color by Tag and its dashes.
    layer: Option<usize>,
    /// Inside another instance of the open component (Model Info > Components fades
    /// it by its "similar" amount).
    similar: bool,
    /// Drawn flattened, not instanced (inside a "Shadows face sun" component).
    flat: bool,
}

/// Lightens a color towards white by `amount` (0 to 1) for geometry outside the open
/// context (Model Info > Components fade sliders; SketchUp's scale is unverified).
fn fade_by(c: [f32; 4], amount: f32) -> [f32; 4] {
    let a = amount.clamp(0.0, 1.0);
    let l = |x: f32| x + (1.0 - x) * a;
    [l(c[0]), l(c[1]), l(c[2]), c[3]]
}

/// Edges fade a little more than faces (black edges at the default 0.55 fade come
/// out at 0.62 gray, as before the fade became a setting).
const EDGE_FADE: f32 = 0.62 / 0.55;

/// Lengths of the component axes drawn by Model Info > Components > Show component
/// axes, relative to the definition's size (draft).
const COMPONENT_AXES: f64 = 0.3;

fn rgba(c: [u8; 3]) -> [f32; 4] {
    [
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
        1.0,
    ]
}

/// How View > Hidden Geometry draws hidden entities (drafts: SketchUp shows hidden
/// edges dashed and hidden faces as a faded mesh; dash lengths unverified). Their
/// color is the style's Modeling Settings > Hidden geometry.
const HIDDEN_DASH: f32 = 4.0;
const HIDDEN_MESH_DASH: f32 = 2.0;
const HIDDEN_FACE_ALPHA: f32 = 0.35;

/// Half-length of drawn guide lines, in inches.
const GUIDE_EXTENT: f64 = 1e5;

/// SketchUp's `UnitsOptions` codes (Ruby `Length::Inches` ... `Length::Meter`,
/// `Length::Decimal` ... `Length::Fractional`; the scaffold's Architectural inches
/// read back as `LengthFormat` 1 / `LengthUnit` 0).
const UNIT_CODES: [(mt_infer::units::LengthUnit, i32); 5] = {
    use mt_infer::units::LengthUnit::*;
    [
        (Inch, 0),
        (Foot, 1),
        (Millimeter, 2),
        (Centimeter, 3),
        (Meter, 4),
    ]
};
const FORMAT_CODES: [(mt_infer::units::LengthFormat, i32); 4] = {
    use mt_infer::units::LengthFormat::*;
    [
        (Decimal, 0),
        (Architectural, 1),
        (Engineering, 2),
        (Fractional, 3),
    ]
};

/// Model Info > Units as the file's `UnitsOptions` entries.
pub fn units_to_options(
    unit: mt_infer::units::LengthUnit,
    o: &mt_infer::units::UnitsOptions,
) -> Vec<mt_kernel::export::NamedOption<'static>> {
    use mt_skp::OptionValue::{Bool, Double, Int};
    let unit_code = UNIT_CODES.iter().find(|x| x.0 == unit).map_or(0, |c| c.1);
    let format_code = FORMAT_CODES
        .iter()
        .find(|x| x.0 == o.format)
        .map_or(0, |c| c.1);
    [
        ("LengthUnit", Int(unit_code)),
        ("LengthFormat", Int(format_code)),
        ("LengthPrecision", Int(i32::from(o.precision))),
        ("LengthSnapEnabled", Bool(o.length_snapping)),
        ("LengthSnapLength", Double(o.snap_length)),
        ("AnglePrecision", Int(i32::from(o.angle_precision))),
        ("AngleSnapEnabled", Bool(o.angle_snapping)),
        ("SnapAngle", Double(o.snap_angle)),
        ("AreaUnit", Int(o.area_unit.code())),
        ("AreaPrecision", Int(i32::from(o.area_precision))),
        ("VolumeUnit", Int(o.volume_unit.code())),
        ("VolumePrecision", Int(i32::from(o.volume_precision))),
        ("ForceInchDisplay", Bool(o.force_inch_display)),
        ("SuppressUnitsDisplay", Bool(o.suppress_units)),
    ]
    .into_iter()
    .map(|(k, v)| ("UnitsOptions", k, v))
    .collect()
}

/// Model Info > Units from a file's `UnitsOptions`; entries missing or out of range
/// keep `unit` / `fallback`'s values.
pub fn units_from_options(
    o: &std::collections::BTreeMap<String, mt_skp::OptionValue>,
    unit: mt_infer::units::LengthUnit,
    fallback: mt_infer::units::UnitsOptions,
) -> (mt_infer::units::LengthUnit, mt_infer::units::UnitsOptions) {
    let int = |k: &str| o.get(k).and_then(|v| v.as_int());
    let flag = |k: &str| o.get(k).and_then(|v| v.as_bool());
    let num = |k: &str| o.get(k).and_then(|v| v.as_f64()).filter(|v| v.is_finite());
    let small = |k: &str| {
        int(k)
            .and_then(|v| u8::try_from(v).ok())
            .filter(|&v| v <= 10)
    };
    let unit = int("LengthUnit")
        .and_then(|c| UNIT_CODES.iter().find(|x| x.1 == c))
        .map_or(unit, |x| x.0);
    let mut out = fallback;
    if let Some(f) = int("LengthFormat").and_then(|c| FORMAT_CODES.iter().find(|x| x.1 == c)) {
        out.format = f.0;
    }
    out.precision = small("LengthPrecision").unwrap_or(out.precision);
    out.length_snapping = flag("LengthSnapEnabled").unwrap_or(out.length_snapping);
    out.snap_length = num("LengthSnapLength")
        .filter(|&v| v > 0.0)
        .unwrap_or(out.snap_length);
    out.angle_precision = small("AnglePrecision").unwrap_or(out.angle_precision);
    out.angle_snapping = flag("AngleSnapEnabled").unwrap_or(out.angle_snapping);
    out.snap_angle = num("SnapAngle")
        .filter(|&v| v > 0.0)
        .unwrap_or(out.snap_angle);
    // Area and volume (SketchUp 2020+; codes unverified): without them, the squares
    // and cubes of the length unit.
    use mt_infer::units::{AreaUnit, VolumeUnit};
    out.area_unit = int("AreaUnit")
        .and_then(AreaUnit::from_code)
        .unwrap_or(AreaUnit::of_length(unit));
    out.volume_unit = int("VolumeUnit")
        .and_then(VolumeUnit::from_code)
        .unwrap_or(VolumeUnit::of_length(unit));
    out.area_precision = small("AreaPrecision").unwrap_or(out.area_precision);
    out.volume_precision = small("VolumePrecision").unwrap_or(out.volume_precision);
    out.force_inch_display = flag("ForceInchDisplay").unwrap_or(out.force_inch_display);
    out.suppress_units = flag("SuppressUnitsDisplay").unwrap_or(out.suppress_units);
    (unit, out)
}

/// A camera as the file's saved view: a parallel view keeps its visible height, a
/// perspective one its field of view.
fn view_camera(c: &Camera) -> mt_skp::ViewCamera {
    mt_skp::ViewCamera {
        eye: c.eye.to_array(),
        target: c.target.to_array(),
        up: c.up().to_array(),
        fov: c.fov_deg,
        parallel: !c.perspective,
        ortho_height: c.view_height(),
    }
}

impl Document {
    pub fn open(path: &Path) -> Result<Self, mt_skp::Error> {
        let src = mt_skp::read(path)?;
        let units = src
            .units
            .as_deref()
            .and_then(mt_infer::units::LengthUnit::from_name)
            .unwrap_or(mt_infer::units::LengthUnit::Inch);
        // VFF files give only the unit: whole millimeters as in SketchUp's metric
        // templates (unverified), two decimals otherwise. Legacy files carry the
        // whole of Model Info > Units.
        let fallback = mt_infer::units::UnitsOptions {
            precision: if units == mt_infer::units::LengthUnit::Millimeter {
                0
            } else {
                2
            },
            ..Default::default()
        };
        let (units, unit_options) = src
            .named_options
            .get("UnitsOptions")
            .map_or((units, fallback), |o| {
                units_from_options(o, units, fallback)
            });
        Ok(Self {
            path: path.to_owned(),
            model: import::from_skp(&src),
            units,
            saved_camera: src.camera,
            unit_options,
            undo: Vec::new(),
            shared: Default::default(),
            redo: Vec::new(),
            context: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
            has_file: true,
            last_hidden: None,
            undo_names: Vec::new(),
            redo_names: Vec::new(),
            operation: String::new(),
            view: None,
            dims_hidden: Vec::new(),
            tess: Default::default(),
            pieces: Default::default(),
        })
    }

    /// The model as `.skp` bytes, with its units and the view it was opened with.
    #[cfg(test)]
    pub fn to_skp(&self) -> Result<Vec<u8>, mt_skp::WriteError> {
        let options = units_to_options(self.units, &self.unit_options);
        mt_kernel::export::to_skp_view(&self.model, &options, self.saved_camera.as_ref())
    }

    /// [`to_skp`](Self::to_skp) saving `camera` as the view the file opens in, as
    /// SketchUp saves the current view. Two-point perspective is saved as plain
    /// perspective (its place in the file is unknown).
    pub fn to_skp_with_camera(&self, camera: &Camera) -> Result<Vec<u8>, mt_skp::WriteError> {
        let options = units_to_options(self.units, &self.unit_options);
        let view = view_camera(camera);
        mt_kernel::export::to_skp_view(&self.model, &options, Some(&view))
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
            shared: Default::default(),
            redo: Vec::new(),
            context: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
            has_file: false,
            last_hidden: None,
            undo_names: Vec::new(),
            redo_names: Vec::new(),
            operation: String::new(),
            view: None,
            dims_hidden: Vec::new(),
            tess: Default::default(),
            pieces: Default::default(),
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

    /// Marks the model as having unsaved changes (a recovered auto-save).
    pub fn mark_unsaved(&mut self) {
        self.saved_state = u64::MAX;
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

    /// The active context's entities for a change that is not an undo step and does
    /// not mark the model modified: a live preview the caller reverts, or records
    /// afterwards with [`Document::edit`] (Texture Position).
    pub fn active_mut_untracked(&mut self) -> &mut Entities {
        let ctx = self.active_ctx();
        self.shared.changed(ctx);
        match Self::path_definitions(&self.model, &self.context).and_then(|d| d.last().copied()) {
            Some(d) => &mut self.model.definitions[d].entities,
            None => &mut self.model.root,
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
        // Neither are images (SketchUp offers Explode instead).
        let def = self.active().instances[index].definition;
        if self.model.definitions.get(def).is_some_and(|d| d.is_image) {
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

    /// The current state as an undo step (copies only the entities collections that
    /// changed since the last step, see `undo`).
    pub(crate) fn snapshot(&mut self) -> Snapshot {
        (
            self.shared.take(&mut self.model),
            self.context.clone(),
            self.state,
            (self.units, self.unit_options),
        )
    }

    /// The entities collection being edited.
    fn active_ctx(&self) -> crate::undo::Ctx {
        match Self::path_definitions(&self.model, &self.context).and_then(|d| d.last().copied()) {
            Some(d) => crate::undo::Ctx::Def(d),
            None => crate::undo::Ctx::Root,
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
        let before = self.snapshot();
        let def =
            Self::path_definitions(&self.model, &self.context).and_then(|d| d.last().copied());
        let mut ents = match def {
            Some(d) => std::mem::take(&mut self.model.definitions[d].entities),
            None => std::mem::take(&mut self.model.root),
        };
        let changed = f(&mut self.model, &mut ents);
        // Even an edit reporting no change may have touched the collection.
        self.shared.changed(match def {
            Some(d) => crate::undo::Ctx::Def(d),
            None => crate::undo::Ctx::Root,
        });
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
        let before = self.snapshot();
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
        self.undo_names.push(self.operation.clone());
        // Old steps are freed on another thread (a big model's take a while).
        if self.undo.len() > UNDO_LIMIT {
            let step = self.undo.remove(0);
            let size = step.0.size();
            crate::undo::discard(step, size);
            self.undo_names.remove(0);
        }
        let redo = std::mem::take(&mut self.redo);
        let size = redo.iter().map(|s| s.0.size()).sum();
        crate::undo::discard(redo, size);
        self.redo_names.clear();
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
        let before = self.snapshot();
        self.units = units;
        self.unit_options = options;
        self.record(before);
        true
    }

    /// The name of the step Edit > Undo would undo (empty when unnamed).
    pub fn undo_name(&self) -> Option<&str> {
        self.undo
            .last()
            .map(|_| self.undo_names.last().map_or("", String::as_str))
    }

    /// The name of the step Edit > Redo would redo.
    pub fn redo_name(&self) -> Option<&str> {
        self.redo
            .last()
            .map(|_| self.redo_names.last().map_or("", String::as_str))
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
        let name = self.undo_names.pop().unwrap_or_default();
        self.redo_names.push(name);
        self.fix_context();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let now = self.restore(snapshot);
        self.undo.push(now);
        let name = self.redo_names.pop().unwrap_or_default();
        self.undo_names.push(name);
        self.fix_context();
        true
    }

    /// Makes `snapshot` current; returns the state it replaced.
    fn restore(&mut self, (step, context, state, units): Snapshot) -> Snapshot {
        let old_units = (self.units, self.unit_options);
        (self.units, self.unit_options) = units;
        (
            self.shared.restore(&mut self.model, step),
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

    /// Remembers `camera` as the saved view (after a save).
    pub fn set_saved_camera(&mut self, camera: &Camera) {
        self.saved_camera = Some(view_camera(camera));
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
    /// and soft edges are left out. Groups and components placed more than once are
    /// drawn per instance (see `instancing`).
    pub fn scene(&self) -> Scene {
        self.scene_with(true)
    }

    /// [`Document::scene`], with or without instancing (without: every instance
    /// flattened into the scene's vertices).
    pub(crate) fn scene_with(&self, instancing: bool) -> Scene {
        self.build_scene(instancing, None)
    }

    /// [`Document::scene`], keeping what did not change from `prev`, the scene this
    /// function last returned (see `scene_cache`). The result is the scene
    /// [`Document::scene`] makes; its `changes` say which of its parts are `prev`'s.
    pub fn scene_from(&self, prev: Option<Scene>) -> Scene {
        let mut cache = (self.pieces.lock()).unwrap_or_else(std::sync::PoisonError::into_inner);
        let settings = self.scene_settings(true);
        let pieces = crate::scene_cache::Pieces::begin(&mut cache, prev, &settings);
        let (mut scene, pieces) = self.build_scene_with(true, Some(pieces));
        if let Some(p) = pieces {
            p.end(&mut cache, &mut scene, settings);
        }
        scene
    }

    /// Everything besides the geometry a scene build reads.
    fn scene_settings(&self, instancing: bool) -> crate::scene_cache::Settings {
        let m = &self.model;
        crate::scene_cache::Settings {
            materials: m.materials.clone(),
            layers: m.layers.clone(),
            folders: m.tag_folders.clone(),
            colors: [m.front_color, m.back_color],
            style: m.style.clone(),
            axes: m.axes,
            color_by_tag: m.color_by_tag,
            edit_fade: m.edit_fade,
            context: self.context.clone(),
            path: Self::path_definitions(m, &self.context),
            view: self.view,
            instancing,
        }
    }

    fn build_scene(&self, instancing: bool, pieces: Option<crate::scene_cache::Pieces>) -> Scene {
        self.build_scene_with(instancing, pieces).0
    }

    fn build_scene_with(
        &self,
        instancing: bool,
        pieces: Option<crate::scene_cache::Pieces>,
    ) -> (Scene, Option<crate::scene_cache::Pieces>) {
        let mut scene = Scene {
            build: BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ..Default::default()
        };
        let mut out = SceneBuild::default();
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
            out.texture_of.push(layer);
        }
        let root = &self.model.root;
        let mut tess = (self.tess.lock()).unwrap_or_else(std::sync::PoisonError::into_inner);
        tess.begin();
        let mut walk = Walk {
            tess: Some(&mut tess),
            instancing: instancing.then(|| crate::instancing::Builder::new(&self.model)),
            pieces,
            ..Default::default()
        };
        self.collect(
            root,
            Some(crate::undo::Ctx::Root),
            DMat4::IDENTITY,
            Inherited::default(),
            Level::Context(&self.context),
            &mut walk,
            &mut out,
        );
        let mut pieces = walk.pieces.take();
        drop(walk);
        out.context.tail = (out.edge_owner.len(), out.tri_owner.len());
        out.context.known = true;
        match &mut pieces {
            Some(p) => {
                out.finish(p.prev(), &mut scene);
                let prev = p.prev();
                scene.edge_seed = (prev.edge_index.take()).map(|e| e.seed(prev.build));
            }
            None => out.finish(&mut Scene::default(), &mut scene),
        }
        tess.end();
        drop(tess);
        let f = |p: DVec3| p.as_vec3().to_array();
        self.section_planes(&mut scene);
        crate::annot::collect(self, &mut scene);
        let rest_hidden = !self.context.is_empty() && self.model.style.hide_rest_of_model;
        let guides = if self.model.style.guides && !rest_hidden {
            &root.guides[..]
        } else {
            &[]
        };
        let guide_color = crate::style_view::rgba(self.model.style.modeling_colors.guides);
        for g in guides {
            match g.dir {
                Some(d) => scene.guides.push(Segment::line(
                    f(g.point - d * GUIDE_EXTENT),
                    f(g.point + d * GUIDE_EXTENT),
                    guide_color,
                    [1.0, 3.0],
                )),
                None => {
                    // A small cross.
                    for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                        scene.guides.push(Segment::line(
                            f(g.point - axis * 0.5),
                            f(g.point + axis * 0.5),
                            guide_color,
                            [1.0, 0.0],
                        ));
                    }
                }
            }
        }
        (scene, pieces)
    }

    /// Faces and edges of entities that are not in the model (the clipboard while
    /// Paste places it), in their own coordinates, drawn with the model's materials,
    /// tags and style.
    pub fn ghost_scene(&self, ents: &Entities) -> Scene {
        let mut out = SceneBuild::default();
        // The same texture numbering as `scene` (the viewport holds those textures).
        let mut next = 0;
        out.texture_of = self
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
            None,
            DMat4::IDENTITY,
            Inherited::default(),
            // Owners are not used: the ghost is not pickable.
            Level::Inside(Owner::Instance(usize::MAX)),
            &mut Walk::default(),
            &mut out,
        );
        let mut scene = Scene::default();
        out.finish(&mut Scene::default(), &mut scene);
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
        if !self.model.style.section_planes || ents.section_planes.is_empty() {
            return;
        }
        // Plane size from the model's extent (SketchUp sizes it to the model; exact
        // rule unverified).
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        scene.for_each_tri(|_, t| {
            for p in t {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        });
        let half = if lo.is_finite() {
            ((hi - lo).length() * 0.35).max(12.0)
        } else {
            24.0
        };
        let f = |p: DVec3| p.as_vec3().to_array();
        for (i, sp) in ents.section_planes.iter().enumerate() {
            // A section plane is an object: View > Hidden Objects shows it.
            if sp.hidden && !self.model.style.hidden_objects {
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
            // Styles > Modeling colors.
            let mc = &self.model.style.modeling_colors;
            let color = crate::style_view::rgba(if active {
                mc.section_active
            } else {
                mc.section_inactive
            });
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
                scene.push_edge(
                    Segment::line(f(a), f(b), color, [EDGE_WIDTH, 0.0]),
                    Some(Owner::Section(i)),
                );
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

    /// The tag and the folders it is in are visible.
    fn layer_visible(&self, layer: Option<usize>) -> bool {
        layer.is_none_or(|l| self.model.layer_shown(l))
    }

    /// Color by Tag: the tag's color for an entity on `layer` (or its container's).
    fn tag_color(&self, layer: Option<usize>) -> Option<[f32; 4]> {
        if !self.model.color_by_tag {
            return None;
        }
        let l = layer.unwrap_or(0);
        self.model.layers.get(l).map(|x| rgba(x.color))
    }

    /// Dash length of edges on `layer` (Tags panel Dashes; 0 = solid).
    fn tag_dash(&self, layer: Option<usize>) -> f32 {
        layer
            .and_then(|l| self.model.layers.get(l))
            .map_or(0.0, |x| x.dashes.length())
    }

    /// Draws `ents`, collection `ctx` of the model (`None`: entities outside it).
    #[allow(clippy::too_many_arguments)]
    fn collect(
        &self,
        ents: &Entities,
        ctx: Option<crate::undo::Ctx>,
        xf: DMat4,
        inherited: Inherited,
        level: Level,
        walk: &mut Walk,
        out: &mut SceneBuild,
    ) {
        // This collection's own faces and edges as a placement of an instanced mesh.
        let place = walk.next.take();
        let show_hidden = self.model.style.hidden_geometry;
        let hidden_color = rgba(self.model.style.modeling_extras.hidden_geometry);
        // View > Hidden Objects: hidden groups and components (drawn as hidden).
        let show_objects = self.model.style.hidden_objects;
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
        let container = inherited.container;
        let outer_layer = inherited.layer;
        let similar = inherited.similar;
        let flat = inherited.flat;
        let inherited = inherited.material;
        // Outside the open context: faded, not selectable.
        let faded = matches!(level, Level::Outside | Level::Context([_, ..]));
        let fades = &self.model.edit_fade;
        let fade_amount = if similar { fades.similar } else { fades.rest };
        let fade = |c: [f32; 4]| fade_by(c, fade_amount);
        // The open component (Model Info > Components fades its other instances).
        let edit_def = editing
            .then(|| Self::path_definitions(&self.model, &self.context))
            .flatten()
            .and_then(|d| d.last().copied());
        let owner_of = |own: Owner| match level {
            Level::Context([]) => Some(own),
            Level::Inside(o) => Some(o),
            _ => None,
        };
        // Styles > Edge > Color (All same / By material / By axis).
        let edge_color = |dir: DVec3, faces: &[FaceId]| {
            let material = faces
                .first()
                .and_then(|&f| ents.faces[f].attrs.material.or(inherited))
                .map(|m| self.color(Some(m), self.model.front_color));
            let c = crate::style_view::edge_color(&self.model, dir, material);
            if faded {
                fade_by(c, fade_amount * EDGE_FADE)
            } else {
                c
            }
        };
        let world = |p: DVec3| {
            let q = xf.transform_point3(p);
            [q.x as f32, q.y as f32, q.z as f32]
        };
        let normal_xf = DMat3::from_mat4(xf).inverse().transpose();
        let world_normal = |f: FaceId| (normal_xf * ents.faces[f].normal).normalize_or_zero();
        let f32x3 = |n: DVec3| [n.x as f32, n.y as f32, n.z as f32];

        // The mesh this placement draws (`placed`), or the one it makes (`build`).
        use crate::instancing::{Build, Made};
        let (mut build, placed) = match (place, &walk.instancing) {
            (Some(key), Some(b)) => match b.made.get(&key) {
                None => (Some(Build::default()), None),
                Some(Made::Mesh(m)) => (None, Some(*m)),
                Some(Made::Flat) => (None, None),
            },
            _ => (None, None),
        };
        let placement_owner = match level {
            Level::Inside(o) => Some(o),
            _ => None,
        };
        let local = |p: DVec3| [p.x as f32, p.y as f32, p.z as f32];
        let local_normal = |f: FaceId| f32x3(ents.faces[f].normal.normalize_or_zero());
        // Where this visit's own drawing starts.
        let start = crate::scene_cache::ends(out);
        let edges_start = out.edge_owner.len();
        // Draws edge `id` (into `build` too when it makes a mesh).
        let style = &self.model.style;
        let draw_edge =
            |out: &mut SceneBuild, build: &mut Option<Build>, id: EdgeId, e: &mt_kernel::Edge| {
                // Edges off: the renderer draws only the profiles among model edges, so
                // edges are left out only when profiles are off too. View > Hidden
                // Geometry shows hidden edges (and soft ones) dashed.
                let hidden = ghost || e.attrs.hidden || e.attrs.soft;
                if rest_here
                    || (!style.edges && !style.profiles)
                    || ((e.attrs.hidden || e.attrs.soft) && !show_hidden)
                    || (ghost && !show_objects)
                    || !self.layer_visible(e.attrs.layer)
                {
                    return;
                }
                // Color by Tag and the tag's dashes (own tag, else the container's).
                let tag = e.attrs.layer.filter(|&l| l > 0).or(outer_layer);
                let color = match self.tag_color(tag) {
                    Some(c) if faded => fade_by(c, fade_amount * EDGE_FADE),
                    Some(c) => c,
                    None => edge_color(
                        xf.transform_vector3(ents.pos(e.end) - ents.pos(e.start)),
                        &e.faces,
                    ),
                };
                let mut seg = Segment::line(
                    world(ents.pos(e.start)),
                    world(ents.pos(e.end)),
                    if hidden { hidden_color } else { color },
                    [
                        EDGE_WIDTH,
                        if hidden {
                            HIDDEN_DASH
                        } else {
                            self.tag_dash(tag)
                        },
                    ],
                );
                if let Some(c) = container {
                    set_container(&mut out.edge_container, out.edge_owner.len(), c);
                }
                if let Some(b) = build {
                    let mut l = seg;
                    (l.a, l.b) = (local(ents.pos(e.start)), local(ents.pos(e.end)));
                    if !hidden {
                        l.flags = Segment::MODEL_EDGE;
                        match e.faces[..] {
                            [] => {}
                            [f] => {
                                l.n1 = local_normal(f);
                                l.profile = Segment::BORDER;
                            }
                            [f, g, ..] => {
                                l.n1 = local_normal(f);
                                l.n2 = local_normal(g);
                                l.profile = Segment::SILHOUETTE;
                            }
                        }
                    }
                    b.edges.push(l);
                }
                // The faces whose normals the world segment takes (none when hidden).
                let normals: &[FaceId] = if hidden {
                    &[]
                } else {
                    &e.faces[..e.faces.len().min(2)]
                };
                if hidden {
                    out.hidden_edges.insert(out.edge_owner.len());
                } else {
                    seg.flags = Segment::MODEL_EDGE;
                    match *normals {
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
                }
                if let Some(b) = build {
                    let n = |k: usize| {
                        normals
                            .get(k)
                            .map_or(DVec3::ZERO, |&f| ents.faces[f].normal)
                    };
                    b.local_edges.push(crate::instancing::LocalEdge {
                        a: ents.pos(e.start),
                        b: ents.pos(e.end),
                        seg,
                        normals: [n(0), n(1)],
                        count: normals.len() as u8,
                        hidden,
                    });
                }
                out.push_edge(seg, owner_of(Owner::Edge(id)));
            };

        // "Cut opening" components glued here cut holes into the faces they sit on.
        let openings = self.cut_openings(ents);
        // Another placement of a mesh that gives its edges for any placement.
        let lazy_edges =
            placed.filter(|&m| out.instancing.meshes[m as usize].local_edges.is_some());
        // The visit as the next build may reuse it (`scene_cache`).
        use crate::scene_cache::{At, Deps, LevelKey, Map, Piece, Pieces, Role, Whole};
        let role = match (&build, lazy_edges, place) {
            (Some(_), _, Some(k)) => Some(Role::Build(k)),
            (None, Some(_), Some(k)) => Some(Role::Placed(k)),
            (None, None, _) if placed.is_none() => Some(Role::Flat),
            _ => None,
        };
        let vkey = match (&walk.pieces, ctx, role) {
            (Some(_), Some(ctx), Some(role)) => Some(crate::scene_cache::VisitKey {
                ctx,
                // A mesh and its placements do not depend on where they are (but
                // see `Whole::valid`).
                xf: match role {
                    Role::Flat => crate::scene_cache::bits(&xf),
                    _ => [0; 16],
                },
                material: inherited,
                hidden: ghost,
                container,
                layer: outer_layer,
                similar,
                flat,
                level: match level {
                    Level::Context(rest) => LevelKey::Context(rest.len()),
                    Level::Inside(o) => LevelKey::Inside(o),
                    Level::Outside => LevelKey::Outside,
                },
                role,
                uids: [ents.vertices.uid(), ents.edges.uid(), ents.faces.uid()],
                openings: openings.clone(),
            }),
            _ => None,
        };
        let (mut rec, mut old) = match (&mut walk.pieces, vkey) {
            (Some(p), Some(k)) => {
                let (rec, old) = p.take(k);
                (Some(rec), old)
            }
            _ => (None, None),
        };
        // A visit that made a mesh, unchanged: the mesh and what it drew are kept.
        let mut whole_kept = false;
        if let (Some(rec), Some(Role::Build(k)), Some(ctx)) = (&mut rec, role, ctx)
            && let Some(w) = Pieces::valid_whole(&mut old, ents, &xf)
            && let Some(p) = &mut walk.pieces
        {
            let w = p.copy_whole(out, w, container, &xf);
            if let Some(builder) = &mut walk.instancing {
                let made = w.mesh.map_or(Made::Flat, Made::Mesh);
                builder.made.insert(k, made);
            }
            if let Some(m) = w.mesh {
                (out.instancing).place(
                    m,
                    xf,
                    placement_owner,
                    w.at.tris.clone(),
                    w.at.edges.clone(),
                    w.lazy,
                );
            }
            if let Some(cache) = &mut walk.tess {
                cache.keep(ctx);
            }
            build = None;
            whole_kept = true;
            rec.whole = Some(w);
        }
        // Another placement of a mesh kept from the last build, unchanged: what it drew
        // is kept.
        let mut placed_kept = false;
        if let (Some(rec), Some(Role::Placed(_)), Some(m)) = (&mut rec, role, lazy_edges)
            && let Some(p) = &mut walk.pieces
            && let Some(w) = p.valid_placed(&mut old, ents, m)
        {
            let w = p.copy_whole(out, w, container, &xf);
            let (tris, edges) = (w.at.tris.clone(), w.at.edges.clone());
            (out.instancing).place(m, xf, placement_owner, tris, edges, true);
            placed_kept = true;
            rec.whole = Some(w);
        }
        if let Some(m) = lazy_edges.filter(|_| !placed_kept) {
            // Only their owners, containers and hidden marks (see `instancing`).
            let mesh = &out.instancing.meshes[m as usize];
            let n = mesh.local_edges.as_ref().map_or(0, Vec::len);
            if let Some(c) = container {
                set_containers(&mut out.edge_container, edges_start..edges_start + n, c);
            }
            for &k in &mesh.hidden_edges {
                out.hidden_edges.insert(edges_start + k as usize);
            }
            out.edge_owner
                .extend(std::iter::repeat_n(placement_owner, n));
        }
        // Flat visits are drawn (and kept) per chunk of their edges and faces.
        let chunked = !whole_kept && role == Some(Role::Flat);
        // Where the open context's entities are drawn, by chunk (`Scene::context`).
        let is_context = matches!(level, Level::Context([]));
        // Edges may be drawn as hidden geometry (`Scene::hidden_edges`).
        let any_hidden = show_hidden || (ghost && show_objects);
        if chunked {
            for c in 0..ents.edges.chunk_count() {
                let start = crate::scene_cache::ends(out);
                match (&mut rec, &mut walk.pieces) {
                    (Some(rec), Some(p)) => {
                        if let Some(piece) = Pieces::valid_piece(&mut old, ents, Map::Edges, c) {
                            rec.edges.push(Some(p.copy_piece(out, piece, container)));
                        } else {
                            let mut deps = Deps::default();
                            for (id, e) in ents.edges.iter_chunk(c) {
                                deps.edge(e);
                                draw_edge(out, &mut build, id, e);
                            }
                            let at = At::since(start, out);
                            let piece = Piece::new(ents, Map::Edges, c, deps, at, out, any_hidden);
                            rec.edges.push(Some(piece));
                        }
                    }
                    _ => {
                        for (id, e) in ents.edges.iter_chunk(c) {
                            draw_edge(out, &mut build, id, e);
                        }
                    }
                }
                if is_context {
                    (out.context.edge_chunks).push(start[0]..out.edge_owner.len());
                }
            }
        } else if !whole_kept {
            for (id, e) in &ents.edges {
                if lazy_edges.is_some() {
                    // Given by the mesh (above).
                    break;
                }
                draw_edge(out, &mut build, id, e);
            }
        }

        let edges = edges_start..out.edge_owner.len();
        let tris_start = out.tri_owner.len();
        if let Some(m) = placed.filter(|_| !placed_kept) {
            // Another placement of a mesh: its triangles come from the mesh; only
            // their owners, materials and containers are kept.
            let mesh = &out.instancing.meshes[m as usize];
            let n = mesh.tris.len();
            if let Some(c) = container {
                set_containers(&mut out.tri_container, tris_start..tris_start + n, c);
            }
            out.tri_material
                .extend(mesh.tris.iter().map(|t| t.materials));
            out.tri_owner
                .extend(std::iter::repeat_n(placement_owner, n));
            let lazy = mesh.local_edges.is_some();
            out.instancing.place(
                m,
                xf,
                placement_owner,
                tris_start..out.tri_owner.len(),
                edges.clone(),
                lazy,
            );
        }
        // A collection reached again (another instance of its definition) reuses the
        // triangles of its faces (they depend only on the collection).
        let key = std::ptr::from_ref(ents) as usize;
        let visit = walk.visits.entry(key).or_insert(0);
        *visit += 1;
        let reused = *visit > 1;
        // Triangles kept from the last scene build (openings depend on more than the
        // face: not kept).
        let mut kept = match (&mut walk.tess, ctx) {
            (Some(cache), Some(ctx))
                if !whole_kept
                    && openings.is_empty()
                    && ents.faces.len() >= crate::tess_cache::KEEP_MIN_FACES =>
            {
                Some(cache.visit(ctx))
            }
            _ => None,
        };
        // Draws face `f` (into `build` too when it makes a mesh).
        let draw_face = |out: &mut SceneBuild,
                         build: &mut Option<Build>,
                         kept: &mut Option<crate::tess_cache::Visit>,
                         triangles: &mut FxMap<(usize, FaceId), Vec<[DVec3; 3]>>,
                         f: FaceId,
                         face: &mt_kernel::Face| {
            let hidden = ghost || face.attrs.hidden;
            if rest_here
                || (face.attrs.hidden && !show_hidden)
                || (ghost && !show_objects)
                || !self.layer_visible(face.attrs.layer)
            {
                return;
            }
            let front_material = face.attrs.material.or(inherited);
            let back_material = face.attrs.back_material.or(inherited);
            let mut front = self.color(front_material, self.model.front_color);
            let mut back = self.color(back_material, self.model.back_color);
            let by_tag = self.tag_color(face.attrs.layer.filter(|&l| l > 0).or(outer_layer));
            if let Some(c) = by_tag {
                front = c;
                back = c;
            }
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
                let i = m.filter(|_| !faded && !hidden && by_tag.is_none())?;
                let layer = *out.texture_of.get(i)?;
                let map = texture::face_mapping(face, is_back, self.model.materials.get(i)?)?;
                // Shift by whole repeats so the numbers stay small in f32.
                let first = ents.loop_points(&face.outer).first().copied()?;
                let shift = texture::uv_at(&map, first).floor();
                // Projective (distorted) mappings: scaled so q is 1 at the first vertex.
                let q0 = map.uvq(first.to_array())[2];
                let q0 = if q0.abs() < 1e-12 { 1.0 } else { q0 };
                Some((layer, map, shift, q0))
            };
            let (front_tex, back_tex) = (side(front_material, false), side(back_material, true));
            // Homogeneous coordinates (u q, v q) and q; the shader divides per pixel.
            type Side = Option<(u32, mt_skp::uv::UvMapping, glam::DVec2, f64)>;
            let uv = |t: &Side, p: DVec3| {
                t.as_ref().map_or(([0.0; 2], 1.0), |(_, map, shift, q0)| {
                    let [a, b, q] = map.uvq(p.to_array()).map(|x| x / q0);
                    let shifted = [a - shift.x * q, b - shift.y * q];
                    (shifted.map(|x| x as f32), q as f32)
                })
            };
            let layer = |t: &Side| t.as_ref().map_or(FaceVertex::NO_TEXTURE, |t| t.0);
            let texture = [layer(&front_tex), layer(&back_tex)];
            let n = world_normal(f);
            let tessellate = || {
                (!openings.is_empty())
                    .then(|| mt_kernel::behavior::cut_face(ents, f, &openings))
                    .flatten()
                    .unwrap_or_else(|| tessellate::triangulate(ents, f))
            };
            let own;
            let tris: &[[DVec3; 3]] = match kept.as_mut().and_then(|v| v.triangles(ents, f)) {
                Some(tris) => tris,
                None if reused => triangles.entry((key, f)).or_insert_with(tessellate),
                None => {
                    own = tessellate();
                    &own
                }
            };
            for &local in tris {
                let lt = crate::instancing::LocalTri {
                    local,
                    normal: face.normal,
                    materials: [front_material, back_material],
                    verts: local.map(|p| {
                        let ([fu, fv], fq) = uv(&front_tex, p);
                        let ([bu, bv], bq) = uv(&back_tex, p);
                        FaceVertex {
                            uv: [fu, fv, bu, bv],
                            q: [fq, bq],
                            texture,
                            ..FaceVertex::new([0.0; 3], [0.0; 3], front, back)
                        }
                    }),
                };
                // A mirroring transform flips the winding; `place` keeps it
                // counter-clockwise around the front normal.
                let (verts, tri) = lt.place(&xf, n);
                match build {
                    Some(b) => b.tris.push(lt),
                    None => {
                        for v in verts {
                            out.vertices.push(v);
                        }
                    }
                }
                if hidden {
                    // A hidden face shows as a mesh of its triangles.
                    for k in 0..3 {
                        let seg = Segment::line(
                            f32x3(tri[k]),
                            f32x3(tri[(k + 1) % 3]),
                            hidden_color,
                            [EDGE_WIDTH, HIDDEN_MESH_DASH],
                        );
                        out.hidden_edges.insert(out.edge_owner.len());
                        out.push_edge(seg, owner_of(Owner::Face(f)));
                    }
                }
                if let Some(c) = container {
                    set_container(&mut out.tri_container, out.tri_owner.len(), c);
                }
                out.push_tri(tri, owner_of(Owner::Face(f)));
                out.tri_material.push([front_material, back_material]);
            }
        };
        if chunked {
            for c in 0..ents.faces.chunk_count() {
                let start = crate::scene_cache::ends(out);
                match (&mut rec, &mut walk.pieces) {
                    (Some(rec), Some(p)) => {
                        if let Some(piece) = Pieces::valid_piece(&mut old, ents, Map::Faces, c) {
                            rec.faces.push(Some(p.copy_piece(out, piece, container)));
                            if let Some(k) = &mut kept {
                                k.keep_chunk(c);
                            }
                        } else {
                            let mut deps = Deps::default();
                            for (f, face) in ents.faces.iter_chunk(c) {
                                deps.face(ents, face);
                                draw_face(out, &mut build, &mut kept, &mut walk.triangles, f, face);
                            }
                            let at = At::since(start, out);
                            let piece = Piece::new(ents, Map::Faces, c, deps, at, out, any_hidden);
                            rec.faces.push(Some(piece));
                        }
                    }
                    _ => {
                        for (f, face) in ents.faces.iter_chunk(c) {
                            draw_face(out, &mut build, &mut kept, &mut walk.triangles, f, face);
                        }
                    }
                }
                if is_context {
                    let edges = start[0]..out.edge_owner.len();
                    (out.context.face_chunks).push((edges, start[2]..out.tri_owner.len()));
                }
            }
        } else if !whole_kept && placed.is_none() {
            for (f, face) in &ents.faces {
                draw_face(out, &mut build, &mut kept, &mut walk.triangles, f, face);
            }
        }
        if let (Some(visit), Some(cache)) = (kept, &mut walk.tess) {
            cache.finish(visit);
        }
        if let (Some(b), Some(key), Some(builder)) = (build, place, &mut walk.instancing) {
            let tris = tris_start..out.tri_owner.len();
            let meshes = out.instancing.meshes.len();
            let contained = edges.len();
            b.finish(builder, key, xf, placement_owner, tris, edges.clone(), out);
            if let Some(rec) = &mut rec {
                let mesh = (out.instancing.meshes.len() > meshes).then_some(meshes as u32);
                let lazy =
                    mesh.is_some_and(|m| out.instancing.meshes[m as usize].local_edges.is_some());
                let at = At::since(start, out);
                let w = Whole::new(ents, at, contained, out, mesh, lazy, &xf, any_hidden);
                rec.whole = Some(w);
            }
        }
        if let (Some(rec), Some(Role::Placed(_)), Some(m)) = (&mut rec, role, lazy_edges)
            && !placed_kept
        {
            let at = At::since(start, out);
            let w = Whole::placement(at, edges.len(), out, m, &xf, any_hidden);
            rec.whole = Some(w);
        }
        if let (Some(rec), Some(p)) = (rec, &mut walk.pieces) {
            p.new.push(rec);
        }

        for (index, i) in ents.instances.iter().enumerate() {
            if (i.hidden && !show_objects) || !self.layer_visible(i.layer) {
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
            if !walk.active.insert(i.definition) {
                continue;
            }
            let child = match level {
                Level::Context([]) => Level::Inside(Owner::Instance(index)),
                Level::Context([first, rest @ ..]) if *first == index => Level::Context(rest),
                Level::Inside(o) => Level::Inside(o),
                _ => Level::Outside,
            };
            let kind = if def.is_group {
                mt_infer::Container::Group
            } else {
                mt_infer::Container::Component
            };
            let parent = Inherited {
                material: i.material.or(inherited),
                hidden: ghost || i.hidden,
                container: container.or((!matches!(child, Level::Context(_))).then_some(kind)),
                layer: i.layer.filter(|&l| l > 0).or(outer_layer),
                similar: similar || (edit_def == Some(i.definition) && !on_path),
                flat,
            };
            let (ix, sun_turn) =
                crate::component_behavior::placed(self.view.as_ref(), def, xf * i.transform);
            // "Shadows face sun" components are turned for their shadows from the
            // flattened vertices.
            let parent = Inherited {
                flat: flat || sun_turn.is_some(),
                ..parent
            };
            // Model Info > Components > Show component axes.
            if self.model.edit_fade.show_axes && !def.is_group && !def.is_image {
                let size = def
                    .entities
                    .vertices
                    .values()
                    .map(|v| v.pos.abs().max_element())
                    .fold(0.0f64, f64::max);
                let len = (size * COMPONENT_AXES).max(1.0);
                let o = ix.transform_point3(DVec3::ZERO);
                for (axis, c) in [
                    (DVec3::X, [0.85, 0.1, 0.1, 1.0]),
                    (DVec3::Y, [0.1, 0.65, 0.1, 1.0]),
                    (DVec3::Z, [0.1, 0.2, 0.9, 1.0]),
                ] {
                    let tip = ix.transform_point3(axis * len);
                    out.guides
                        .push(Segment::line(f32x3(o), f32x3(tip), c, [EDGE_WIDTH, 0.0]));
                }
            }
            let start = out.vertices.len();
            let ctx = Some(crate::undo::Ctx::Def(i.definition));
            // Placed more than once: drawn per instance (see `instancing`), except on
            // the open context's path and when hidden.
            let reach = (walk.instancing.as_ref())
                .and_then(|b| b.reach.get(&i.definition).copied())
                .unwrap_or(0);
            walk.next = (reach >= 2
                && !parent.flat
                && !parent.hidden
                && matches!(child, Level::Inside(_) | Level::Outside))
            .then_some(crate::instancing::Key {
                definition: i.definition,
                material: parent.material,
                layer: parent.layer,
                similar: parent.similar,
                faded: matches!(child, Level::Outside),
            });
            let drawn = (out.edge_owner.len(), out.tri_owner.len());
            self.collect(&def.entities, ctx, ix, parent, child, walk, out);
            if matches!(level, Level::Context([])) {
                let edges = drawn.0..out.edge_owner.len();
                (out.context.instances).push((index, edges, drawn.1..out.tri_owner.len()));
            }
            if let Some(turn) = sun_turn {
                out.sun_facing.push((start..out.vertices.len(), turn));
            }
            walk.active.remove(&i.definition);
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
}

/// Zoom Extents: a perspective camera looking at everything `scene` draws.
pub fn extents_camera_of(scene: &Scene, fov_deg: f64) -> Option<Camera> {
    let (min, max) = crate::camera_cmd::scene_bounds(scene)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/openskp")
            .join(name)
    }

    #[test]
    fn undo_steps_keep_their_names() {
        let mut doc = Document::untitled();
        doc.operation = "Line".into();
        assert!(doc.edit(|root| !mt_kernel::add_line(root, DVec3::ZERO, DVec3::X).is_empty()));
        doc.operation = "Move".into();
        assert!(doc.edit(|root| !mt_kernel::add_line(root, DVec3::Y, DVec3::Z).is_empty()));
        assert_eq!(doc.undo_name(), Some("Move"));
        assert_eq!(doc.redo_name(), None);
        doc.undo();
        assert_eq!(doc.undo_name(), Some("Line"));
        assert_eq!(doc.redo_name(), Some("Move"));
        doc.redo();
        assert_eq!(doc.undo_name(), Some("Move"));
        doc.undo();
        doc.undo();
        assert_eq!(doc.undo_name(), None);
        assert_eq!(doc.redo_name(), Some("Line"));
    }

    thread_local! {
        /// The scene the last `dump` built from the one before (`scene_cache`).
        static KEPT: std::cell::RefCell<Option<Scene>> = const { std::cell::RefCell::new(None) };
    }

    /// Everything undo restores, as text (clones drop the spatial indexes, which are
    /// lookup caches, not model data).
    /// Also checks that the scene, built with the triangles kept from earlier builds,
    /// is the one a new document draws.
    fn dump(doc: &Document) -> String {
        let mut fresh = Document::untitled();
        fresh.model = doc.model.clone();
        fresh.context = doc.context.clone();
        assert_eq!(
            crate::bench::checksum(&doc.scene()),
            crate::bench::checksum(&fresh.scene()),
            "kept triangles"
        );
        // And the scene built from the previous dump's keeps only what is unchanged.
        let prev = KEPT.with(|k| k.borrow_mut().take());
        let kept = crate::scene_cache::tests::step(doc, prev, "kept pieces");
        KEPT.with(|k| *k.borrow_mut() = Some(kept));
        format!(
            "{:?}",
            (
                doc.model.clone(),
                &doc.context,
                doc.units,
                doc.unit_options,
                doc.state
            )
        )
    }

    /// A square of size 10 at height `z`.
    fn square(ents: &mut Entities, z: f64) {
        let p = [
            DVec3::new(0.0, 0.0, z),
            DVec3::new(10.0, 0.0, z),
            DVec3::new(10.0, 10.0, z),
            DVec3::new(0.0, 10.0, z),
        ];
        for i in 0..4 {
            mt_kernel::add_line(ents, p[i], p[(i + 1) % 4]);
        }
    }

    /// Undo steps share unchanged entities collections, so each kind of edit (and
    /// changes made without an undo step in between) is checked to come back exactly:
    /// undo restores the state before each step and redo the state after it.
    #[test]
    fn undo_and_redo_restore_the_exact_model_after_many_edit_kinds() {
        type Action = (&'static str, fn(&mut Document));
        fn group_all(doc: &mut Document) {
            doc.edit_model(|model, ents| {
                let sel = mt_kernel::group::Selection {
                    edges: ents.edges.keys().collect(),
                    faces: ents.faces.keys().collect(),
                    instances: (0..ents.instances.len()).collect(),
                };
                mt_kernel::group::make_group(&mut model.definitions, ents, &sel, "G".into(), true)
                    .is_some()
            });
        }
        let actions: [Action; 24] = [
            ("square", |d| {
                d.edit(|e| {
                    square(e, 0.0);
                    true
                });
            }),
            ("group it", group_all),
            ("second square", |d| {
                d.edit(|e| {
                    square(e, 20.0);
                    true
                });
            }),
            ("untracked style, open group", |d| {
                d.model.style.hidden_geometry = true;
                assert!(d.open_instance(0));
            }),
            ("draw in group", |d| {
                d.edit(|e| !mt_kernel::add_line(e, DVec3::ZERO, DVec3::Z * 5.0).is_empty());
            }),
            ("push/pull in group", |d| {
                let f = d.active().faces.keys().next().unwrap();
                d.edit(|e| mt_kernel::push_pull::push_pull(e, f, 7.0, false).is_some());
            }),
            ("hide in group", |d| {
                let f = d.active().faces.keys().next().unwrap();
                d.hide(&[Owner::Face(f)]);
            }),
            ("close, untracked root change", |d| {
                d.close_instance();
                let e = d.model.root.edges.keys().next().unwrap();
                d.model.root.edges[e].attrs.soft = true;
            }),
            ("erase", |d| {
                let e = d.model.root.edges.keys().nth(1).unwrap();
                d.erase(&[Owner::Edge(e)], false, false);
            }),
            ("material (whole model)", |d| {
                d.edit_whole(|m| {
                    m.materials.push(mt_kernel::Material {
                        name: "Red".into(),
                        color: [200, 0, 0],
                        opacity: 1.0,
                        texture: None,
                    });
                    true
                });
            }),
            ("units", |d| {
                d.set_units(mt_infer::units::LengthUnit::Millimeter, Default::default());
            }),
            ("untracked preview", |d| {
                let ents = d.active_mut_untracked();
                let e = ents.edges.keys().next().unwrap();
                ents.edges[e].attrs.smooth = true;
            }),
            ("second instance", |d| {
                d.edit(|e| {
                    let mut i = e.instances[0].clone();
                    i.transform = DMat4::from_translation(DVec3::X * 30.0);
                    e.instances.push(i);
                    true
                });
                assert!(d.open_instance(1));
            }),
            ("edit the shared definition", |d| {
                d.edit(|e| !mt_kernel::add_line(e, DVec3::Y, DVec3::Y * 9.0).is_empty());
            }),
            ("close, untracked definition change", |d| {
                d.close_instance();
                let def = d.model.root.instances[0].definition;
                let ents = &mut d.model.definitions[def].entities;
                let e = ents.edges.keys().next().unwrap();
                ents.edges[e].attrs.hidden = true;
            }),
            ("move", |d| {
                let all = crate::context_extra::all_owners(d.active());
                d.edit(|e| {
                    crate::modify::apply(e, &all, DMat4::from_translation(DVec3::Z), false);
                    true
                });
            }),
            ("lock", |d| {
                d.lock(&[Owner::Instance(1)], true);
            }),
            ("group everything", group_all),
            ("explode", |d| {
                d.edit_model(|m, e| mt_kernel::group::explode(&m.definitions, e, 0));
            }),
            ("delete", |d| {
                let e = d.active().edges.keys().next().unwrap();
                d.delete(&[Owner::Edge(e), Owner::Instance(0)]);
            }),
            // Enough entities for several storage chunks (`mt_kernel::slot_store`), then
            // changes to a few of them: steps share the chunks they did not touch.
            ("many lines", |d| {
                d.edit(|e| {
                    for i in 0..700 {
                        let x = i as f64 * 3.0;
                        mt_kernel::add_line(
                            e,
                            DVec3::new(x, 100.0, 0.0),
                            DVec3::new(x, 110.0, 1.0),
                        );
                    }
                    true
                });
            }),
            ("untracked change in a shared chunk", |d| {
                let ents = d.active_mut_untracked();
                let e = ents.edges.keys().nth(400).unwrap();
                ents.edges[e].attrs.soft = true;
                let v = ents.vertices.keys().nth(900).unwrap();
                ents.vertices[v].pos.z += 0.5;
            }),
            ("erase from the middle", |d| {
                let edges: Vec<Owner> = (d.active().edges.keys())
                    .skip(300)
                    .step_by(7)
                    .take(40)
                    .map(Owner::Edge)
                    .collect();
                d.erase(&edges, false, false);
            }),
            ("draw into freed slots", |d| {
                d.edit(|e| {
                    let a = DVec3::new(0.0, -50.0, 0.0);
                    !mt_kernel::add_line(e, a, a + DVec3::X * 9.0).is_empty()
                });
            }),
        ];
        let mut doc = Document::untitled();
        // The state before each undo step.
        let mut before = Vec::new();
        for (name, action) in actions {
            let steps = doc.undo.len();
            let dump_before = dump(&doc);
            action(&mut doc);
            if doc.undo.len() > steps {
                before.push((name, dump_before));
            }
        }
        let names: Vec<_> = before.iter().map(|b| b.0).collect();
        assert_eq!(
            before.len(),
            19,
            "every edit kind is one undo step: {names:?}"
        );
        let mut after = Vec::new();
        for (name, state) in before.iter().rev() {
            after.push(dump(&doc));
            assert!(doc.undo());
            assert_eq!(dump(&doc), *state, "undo {name}");
        }
        for ((name, _), state) in before.iter().zip(after.iter().rev()) {
            assert!(doc.redo());
            assert_eq!(dump(&doc), *state, "redo {name}");
        }
        // Undo part way, edit, and undo back again.
        for _ in 0..5 {
            doc.undo();
        }
        let back = dump(&doc);
        doc.edit(|e| {
            square(e, 50.0);
            true
        });
        let edited = dump(&doc);
        assert!(doc.undo());
        assert_eq!(dump(&doc), back);
        assert!(doc.redo());
        assert_eq!(dump(&doc), edited);
        while doc.undo() {}
        assert_eq!(dump(&doc), before[0].1, "back to the empty model");
    }

    /// A hexagon of radius 4 around `c` (a face large enough for its triangles to be
    /// kept between scene builds).
    fn hexagon(ents: &mut Entities, c: DVec3) {
        let p = |k: usize| {
            let a = k as f64 * std::f64::consts::TAU / 6.0;
            c + DVec3::new(a.cos(), a.sin(), 0.0) * 4.0
        };
        for k in 0..6 {
            mt_kernel::add_line(ents, p(k), p(k + 1));
        }
    }

    /// Triangles kept from earlier scene builds (`tess_cache`) are only reused for
    /// unchanged faces: after each kind of edit, undo and redo the scene is the one a
    /// new document draws (`dump` compares them).
    #[test]
    fn kept_triangles_follow_every_edit() {
        let mut doc = Document::untitled();
        let at = |i: usize| DVec3::new((i % 5) as f64 * 10.0, (i / 5) as f64 * 10.0, 0.0);
        doc.edit(|e| {
            (0..20).for_each(|i| hexagon(e, at(i)));
            true
        });
        dump(&doc);
        let faces: Vec<FaceId> = doc.model.root.faces.keys().collect();
        assert_eq!(faces.len(), 20);
        // Move some faces (their edges and neighbors stay).
        let some: Vec<Owner> = faces[..5].iter().map(|&f| Owner::Face(f)).collect();
        doc.edit(|e| {
            crate::modify::apply(e, &some, DMat4::from_translation(DVec3::Z * 3.0), false);
            true
        });
        dump(&doc);
        // Push/pull one, erase an edge of another, group the rest of a row.
        doc.edit(|e| mt_kernel::push_pull::push_pull(e, faces[7], 5.0, false).is_some());
        dump(&doc);
        let edge = doc.model.root.faces[faces[12]].outer.coedges[0].edge;
        doc.erase(&[Owner::Edge(edge)], false, false);
        dump(&doc);
        doc.edit_model(|model, ents| {
            let sel = mt_kernel::group::Selection {
                edges: vec![],
                faces: faces[15..].to_vec(),
                instances: vec![],
            };
            mt_kernel::group::make_group(&mut model.definitions, ents, &sel, "G".into(), true)
                .is_some()
        });
        dump(&doc);
        // A change without an undo step.
        let v = doc.model.root.vertices.keys().next().unwrap();
        doc.model.root.vertices[v].pos.z += 1.0;
        dump(&doc);
        while doc.undo() {
            dump(&doc);
        }
        while doc.redo() {
            dump(&doc);
        }
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
        let visible_tris = doc.scene().triangles().len();
        assert!(doc.hide(&[Owner::Face(face), Owner::Instance(0)]));
        assert!(doc.scene().triangles().is_empty());
        assert!(
            !doc.hide(&[Owner::Face(face)]),
            "already hidden: no undo step"
        );
        // Hidden Geometry shows the hidden face, Hidden Objects the hidden group;
        // each alone shows only its own kind.
        doc.model.style.hidden_geometry = true;
        let geometry_only = doc.scene();
        assert!(geometry_only.tri_owner.contains(&Some(Owner::Face(face))));
        assert!(!geometry_only.tri_owner.contains(&Some(Owner::Instance(0))));
        doc.model.style.hidden_geometry = false;
        doc.model.style.hidden_objects = true;
        let objects_only = doc.scene();
        assert!(!objects_only.tri_owner.contains(&Some(Owner::Face(face))));
        assert!(objects_only.tri_owner.contains(&Some(Owner::Instance(0))));
        doc.model.style.hidden_geometry = true;
        let scene = doc.scene();
        doc.model.style.hidden_objects = false;
        assert_eq!(scene.triangles().len(), visible_tris);
        assert!(scene.tri_owner.contains(&Some(Owner::Instance(0))));
        assert!(scene.tri_owner.contains(&Some(Owner::Face(face))));
        assert!(!scene.hidden_edges.is_empty());
        assert!(
            scene
                .hidden_edges
                .iter()
                .all(|&i| scene.edges()[i].style[1] > 0.0),
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
        let tris = |doc: &Document| doc.scene().triangles().len();
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
        assert!(!scene.edges().is_empty());
        assert!(!scene.indices.is_empty());
        assert_eq!(scene.indices.len() % 3, 0);
    }

    /// The current view is saved and reopened (perspective and parallel).
    #[test]
    fn saved_view_survives_save_and_reopen() {
        let dir = std::env::temp_dir().join(format!("mt-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("view.skp");
        let mut doc = Document::untitled();
        mt_kernel::add_line(&mut doc.model.root, DVec3::ZERO, DVec3::X);
        for perspective in [true, false] {
            let cam = Camera {
                eye: DVec3::new(-120.0, -300.0, 80.0),
                target: DVec3::new(10.0, 5.0, 2.0),
                fov_deg: 40.0,
                perspective,
                two_point: false,
            };
            std::fs::write(&path, doc.to_skp_with_camera(&cam).unwrap()).unwrap();
            let back = Document::open(&path).unwrap().saved_camera().unwrap();
            assert!((back.eye - cam.eye).length() < 1e-9);
            assert!((back.target - cam.target).length() < 1e-9);
            assert_eq!(back.perspective, perspective);
            assert!((back.fov_deg - 40.0).abs() < 1e-9, "{}", back.fov_deg);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn instances_are_placed_in_world_space() {
        let doc = Document::open(&corpus("Untitled.skp")).unwrap();
        let root_only = doc.model.root.edges.len();
        assert!(doc.scene().edges().len() > root_only);
    }

    /// Model Info > Units survive save and reopen through the file's UnitsOptions.
    #[test]
    fn units_survive_save_and_reopen() {
        use mt_infer::units::{LengthFormat, LengthUnit, UnitsOptions};
        let dir = std::env::temp_dir().join(format!("mt-units-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("units.skp");
        for empty in [true, false] {
            let mut doc = Document::untitled();
            if !empty {
                mt_kernel::add_line(&mut doc.model.root, DVec3::ZERO, DVec3::X);
            }
            doc.units = LengthUnit::Centimeter;
            doc.unit_options = UnitsOptions {
                format: LengthFormat::Decimal,
                precision: 3,
                length_snapping: false,
                snap_length: 0.5,
                angle_precision: 2,
                angle_snapping: false,
                snap_angle: 7.5,
                area_unit: mt_infer::units::AreaUnit::SquareMeters,
                area_precision: 1,
                volume_unit: mt_infer::units::VolumeUnit::Liters,
                volume_precision: 3,
                force_inch_display: false,
                suppress_units: true,
            };
            std::fs::write(&path, doc.to_skp().unwrap()).unwrap();
            let back = Document::open(&path).unwrap();
            assert_eq!(back.units, doc.units);
            assert_eq!(back.unit_options, doc.unit_options);
        }
        // A SketchUp-saved v17 file: its own units (Decimal, two places here).
        let doc = Document::open(&corpus("capilla_quiroz_v17.skp")).unwrap();
        assert_eq!(doc.unit_options.format, LengthFormat::Decimal);
        assert_eq!(doc.unit_options.precision, 2);
        let _ = std::fs::remove_dir_all(&dir);
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

    /// The GUI's sequence on a blank model: Rectangle, Push/Pull it into a box, Move
    /// or Rotate part of it, or a copy (the tools' clicks inside `Document::edit`, as
    /// the app runs them), then undo and redo: each step comes back exactly. (What
    /// the viewport draws after the undo: `live_view`'s
    /// `a_finished_moves_stretch_is_not_drawn`.)
    #[test]
    fn undo_after_moving_part_of_a_pushed_box() {
        use crate::modify::{Kind, ModifyTool, PushPull, Transform};
        fn at(p: DVec3) -> mt_infer::Inference {
            mt_infer::Inference {
                point: p,
                kind: mt_infer::Kind::Free,
                edge: None,
                guide: None,
                locked: false,
                normal: None,
                segment: None,
                container: None,
            }
        }
        fn modify(
            doc: &mut Document,
            tool: &mut dyn ModifyTool,
            picked: Option<Owner>,
            selection: &[Owner],
            p: DVec3,
        ) -> bool {
            doc.edit(|ents| {
                let mut ctx = crate::modify::Ctx {
                    ents,
                    picked,
                    selection,
                    ctrl: false,
                    eye: DVec3::new(30.0, -40.0, 50.0),
                };
                tool.click(&mut ctx, &at(p))
            })
        }
        fn face_toward(e: &Entities, n: DVec3) -> FaceId {
            (e.faces.iter().find(|(_, f)| f.normal.dot(n) > 0.9))
                .map(|(k, _)| k)
                .unwrap()
        }
        fn edge_where(e: &Entities, f: impl Fn(DVec3, DVec3) -> bool) -> EdgeId {
            (e.edges.keys())
                .find(|&k| f(e.pos(e.edges[k].start), e.pos(e.edges[k].end)))
                .unwrap()
        }
        // What is moved, from the box: the entity under the cursor or a selection.
        type Pick = fn(&Entities) -> (Option<Owner>, Vec<Owner>);
        let picks: [(&str, Pick); 6] = [
            ("top edge", |e| {
                let k = edge_where(e, |a, b| a.z > 4.0 && b.z > 4.0);
                (Some(Owner::Edge(k)), vec![])
            }),
            ("vertical edge", |e| {
                let k = edge_where(e, |a, b| (a - b).z.abs() > 4.0);
                (Some(Owner::Edge(k)), vec![])
            }),
            ("top face", |e| {
                (Some(Owner::Face(face_toward(e, DVec3::Z))), vec![])
            }),
            ("side face", |e| {
                (Some(Owner::Face(face_toward(e, DVec3::X))), vec![])
            }),
            ("selected top face and an edge", |e| {
                let f = face_toward(e, DVec3::Z);
                let k = edge_where(e, |a, b| a.z < 1.0 && b.z < 1.0);
                (None, vec![Owner::Face(f), Owner::Edge(k)])
            }),
            ("everything selected", |e| {
                (None, crate::context_extra::all_owners(e))
            }),
        ];
        // `dump`, with negative zeros as zeros: a translation recomputes the face
        // normals (-0.0 becomes 0.0), and undo keeps storage chunks that compare
        // equal.
        fn exact(doc: &Document) -> String {
            dump(doc).replace("-0.0,", "0.0,").replace("-0.0)", "0.0)")
        }
        for (kind, copy, (name, pick)) in [Kind::Move, Kind::Rotate]
            .into_iter()
            .flat_map(|k| [false, true].map(|c| (k, c)))
            .flat_map(|(k, c)| picks.map(|p| (k, c, p)))
        {
            {
                let name = format!("{kind:?} {name}, copy {copy}");
                let mut doc = Document::untitled();
                let mut states = vec![exact(&doc)];
                let mut rect = crate::tools::for_command("tool.rectangle").unwrap();
                assert!(!doc.edit(|e| rect.click(e, &at(DVec3::ZERO))));
                assert!(doc.edit(|e| rect.click(e, &at(DVec3::new(10.0, 8.0, 0.0)))));
                states.push(exact(&doc));
                let f = doc.active().faces.keys().next().unwrap();
                let mut pp = PushPull::default();
                let p = DVec3::new(5.0, 4.0, 0.0);
                assert!(!modify(&mut doc, &mut pp, Some(Owner::Face(f)), &[], p));
                // Pulled up (a ground face points down: the distance is along it).
                let n = doc.active().faces[f].normal;
                let up = p + n * 5.0 * n.z.signum();
                assert!(modify(&mut doc, &mut pp, Some(Owner::Face(f)), &[], up));
                states.push(exact(&doc));
                let (picked, sel) = pick(doc.active());
                let mut tool = Transform::new(kind);
                if copy {
                    tool.toggle_ctrl();
                }
                // Move: base point, destination. Rotate: center, start, end (a
                // quarter turn about the vertical through the corner).
                let c = DVec3::new(10.0, 8.0, 5.0);
                let clicks = match kind {
                    Kind::Move => vec![c, c + DVec3::new(3.0, 2.0, 1.0)],
                    Kind::Rotate => vec![c, c + DVec3::X, c + DVec3::Y],
                };
                let (last, first) = clicks.split_last().unwrap();
                for &p in first {
                    assert!(!modify(&mut doc, &mut tool, picked, &sel, p), "{name}");
                }
                assert!(modify(&mut doc, &mut tool, picked, &sel, *last), "{name}");
                states.push(exact(&doc));
                assert_ne!(states[3], states[2], "{name}");
                assert_eq!(doc.undo.len(), 3, "{name}: three undo steps");
                for (i, s) in states.iter().enumerate().rev().skip(1) {
                    assert!(doc.undo());
                    assert_eq!(exact(&doc), *s, "{name}: undo to state {i}");
                }
                for (i, s) in states.iter().enumerate().skip(1) {
                    assert!(doc.redo());
                    assert_eq!(exact(&doc), *s, "{name}: redo to state {i}");
                }
            }
        }
    }
}
