//! SketchUp `.skp` writer for the legacy (v17, SketchUp 2017) MFC `CArchive`
//! format.
//!
//! A Rust port of `openskp.create` from [OpenSKP](https://github.com/iamahsanmehmood/openskp)
//! (MIT, Copyright (c) 2024-2026 Ahsan Mehmood). Output is byte-identical to the
//! Python writer for the same calls (see `tests/write.rs`).
//!
//! The writer inverts the legacy reader's decoding (class-ref/back-ref protocol,
//! entity preambles, drawbase records) and adds the flag bytes real SketchUp
//! requires although readers ignore them. It supports faces (with holes,
//! front/back materials, layers, explicit texture positioning, custom attributes
//! and optional fan triangulation of warped polygons), circles, arcs and
//! polylines as true curve entities, solid and PNG/JPEG-textured materials,
//! layers, component definitions with instances (nestable inside other
//! definitions), groups, images, guides, section planes, leader texts and free
//! linear dimensions. Vertices and edges are shared wherever coordinates are
//! exactly equal. Coordinates are in inches. Files open in the standard "Iso"
//! view.
//!
//! # Ordering
//!
//! The file is assembled from sections that each start numbering their archive
//! slots where the previous one ended, so calls must come in section order:
//! materials, then layers, then component definitions and groups, then
//! root-level geometry and instances. An out-of-order call fails with
//! [`WriteError::Order`].
//!
//! # The blank scaffold
//!
//! Every legacy `.skp` carries a header, material-manager and style/font-manager
//! region OpenSKP has not fully reverse-engineered; it only understands enough to
//! preserve it byte-for-byte and renumber the few references inside it that shift
//! when geometry is inserted. New files are therefore built by splicing geometry
//! into a bundled minimal empty document (`src/scaffold/blank_v17.skp`).
//!
//! As OpenSKP discloses: that template's bytes came from Trimble's official
//! SketchUp SDK during OpenSKP's research phase (`SUModelCreate` plus a bare
//! `SUModelSaveToFileWithVersion` call, nothing else). Its content is SketchUp's
//! built-in empty-document boilerplate (default style, default "Layer0",
//! references to system fonts like Arial/Tahoma), the same bytes any new SketchUp
//! document contains, not anyone's creative work or user data. The entity
//! encoding, object-graph protocol, required flag bytes and tail-reference
//! renumbering are independently reverse-engineered. No SDK is called at any
//! point.
//!
//! # Determinism
//!
//! Definitions, instances, groups and images carry a random GUID, and
//! definitions a creation timestamp (`uuid.uuid4()` and `time.time()` in
//! OpenSKP). [`SkpBuilder::set_guid_source`] and [`SkpBuilder::set_clock`]
//! replace both; [`SkpBuilder::deterministic`] uses [`splitmix_guids`] and a
//! fixed timestamp. Everything else is a pure function of the calls made.

mod archive;
mod geom;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub use archive::EdgeFlags;
use archive::{
    ArcParams, ArchiveWriter, Body, CurveKind, FaceRecord, InstanceKind, InstanceRecord, shift_ref,
};
pub use geom::rotation_matrix;

/// A point or vector in inches.
pub type Point3 = [f64; 3];

/// The bundled blank v17 document (see the module docs).
const SCAFFOLD: &[u8] = include_bytes!("../scaffold/blank_v17.skp");

// Offsets and slot numbers inside SCAFFOLD, as OpenSKP derives them by walking
// it with the legacy reader (the material insertion point sits 9 bytes before
// the `CLayer` class record). They are specific to this exact file; the test
// suite re-derives them from the Python writer.
const MATERIAL_INSERT_POS: usize = 3400;
/// First slot a material would occupy.
const BASE_SLOT: u32 = 9;
const LAYER_COUNT_POS: usize = 3405;
const ORIG_LAYER_COUNT: u32 = 1;
const LAYER_INSERT_POS: usize = 3505;
const LAYER_WRITER_BASE: u32 = 11;
const DEF_COUNT_POS: usize = 3507;
const ORIG_DEF_COUNT: u32 = 0;
const ROOT_COUNT_POS: usize = 3511;
const ORIG_ROOT_COUNT: u32 = 0;
const TAIL_POS: usize = 3515;
const SCAFFOLD_NEXT_SLOT: u32 = 11;
/// Classes the scaffold declares after the material insertion point.
const SCAFFOLD_CLASS_SLOTS: &[(&str, u32)] = &[("CLayer", 9)];
/// u32 "last persistent id handed out" counter, before the material section.
const PID_COUNTER_POS: usize = 1987;

/// Tail-relative offsets of slot references that shift with every new slot.
/// Found by OpenSKP by diffing SDK-authored files; specific to this scaffold.
const TAIL_REF_POSITIONS: [usize; 6] = [409, 468, 477, 479, 1383, 1385];

/// Camera bytes SketchUp writes for the standard "Iso" view (parallel
/// projection looking at the origin from the (1, -1, 1) octant), copied
/// verbatim from SDK output.
const ISO_CAMERA_PREFIX_OFFSET: usize = 2993;
const ISO_CAMERA_PREFIX_PATCH: [u8; 98] = hex_bytes(concat!(
    "594000000000000059c000000000000059400000000000000000000000000000",
    "000000000000000000003f2c0c70bd20dabf3f2c0c70bd20da3f3f2c0c70bd20",
    "ea3f000000000000f03f0000000000408f40000000000000003e402adf272c80",
    "3457",
));
const ISO_CAMERA_TAIL_PATCH_A: (usize, [u8; 16]) =
    (509, hex_bytes("d0a869613c442d4799a4667d1adfa836"));
const ISO_CAMERA_TAIL_PATCH_B: (usize, [u8; 15]) =
    (1390, hex_bytes("4e53c84477029246bba95827bba7e2"));

/// The unbounded side of a construction line (SketchUp's own sentinel).
const CLINE_INFINITE: f64 = 1e30;

/// Where a [leader text](SkpBuilder::add_text)'s label floats by default,
/// relative to its anchor.
pub const DEFAULT_LEADER: Point3 = [15.0, 15.0, 15.0];

pub(crate) const fn hex_bytes<const N: usize>(s: &str) -> [u8; N] {
    const fn nib(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("bad hex digit"),
        }
    }
    let b = s.as_bytes();
    assert!(b.len() == 2 * N, "hex length mismatch");
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = (nib(b[2 * i]) << 4) | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}

#[derive(Debug)]
pub enum WriteError {
    Io(std::io::Error),
    /// A call came after a section it must precede (see the module docs).
    Order(String),
    /// Invalid geometry or arguments.
    Invalid(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Order(m) | Self::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

fn invalid(msg: impl Into<String>) -> WriteError {
    WriteError::Invalid(msg.into())
}

fn order(msg: impl Into<String>) -> WriteError {
    WriteError::Order(msg.into())
}

/// A material registered with [`SkpBuilder::add_material`] or
/// [`SkpBuilder::add_texture_material`]. Only valid on the builder that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialId {
    owner: u64,
    slot: u32,
}

/// A layer registered with [`SkpBuilder::add_layer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId {
    owner: u64,
    slot: u32,
}

/// A finished component definition (or group), returned by
/// [`DefinitionBuilder::finish`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefinitionId {
    owner: u64,
    slot: u32,
    name: String,
}

impl DefinitionId {
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A typed attribute value. The wrapper variants pick SketchUp's specific type
/// tags for values that share a wire shape with another type.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
    Null,
    Bool(bool),
    Int(i32),
    Double(f64),
    String(String),
    /// `Geom::Point3d`, a position in inches.
    Point3d(Point3),
    /// `Geom::Vector3d`, a direction.
    Vector3d(Point3),
    /// `Length`, a double in inches.
    Length(f64),
    /// A `time_t`-style unsigned 32-bit time.
    Timestamp(u32),
    Array(Vec<AttributeValue>),
}

/// A named attribute dictionary (SketchUp's `AttributeDictionary`, the
/// mechanism dynamic components use), entries in write order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttributeDict {
    pub name: String,
    pub entries: Vec<(String, AttributeValue)>,
}

impl AttributeDict {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    pub fn with(mut self, key: impl Into<String>, value: AttributeValue) -> Self {
        self.entries.push((key.into(), value));
        self
    }
}

/// One texture-positioning pin: a point on the face and the texture
/// coordinate (in tiles of the image) that lands on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvPin {
    pub point: Point3,
    pub uv: [f64; 2],
}

/// A planar face: an outer loop (not repeating the first point) and holes.
#[derive(Debug, Clone, Default)]
pub struct Face {
    pub outer: Vec<Point3>,
    /// Closed loops cut out of the face, on its plane, any winding.
    pub holes: Vec<Vec<Point3>>,
    pub material: Option<MaterialId>,
    pub back_material: Option<MaterialId>,
    pub layer: Option<LayerId>,
    pub hidden: bool,
    /// Flags for edges this face newly creates (shared edges keep theirs).
    pub edges: EdgeFlags,
    /// Explicit texture positioning instead of the default projection.
    pub front_uv: Option<[UvPin; 3]>,
    pub back_uv: Option<[UvPin; 3]>,
    pub attributes: Vec<AttributeDict>,
    /// Fan-triangulate from the first point when the outer loop is not
    /// planar, instead of failing (what SketchUp's UI does). Ignored with holes.
    pub auto_triangulate: bool,
}

impl Face {
    pub fn new(outer: impl IntoIterator<Item = Point3>) -> Self {
        Self {
            outer: outer.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn hole(mut self, hole: impl IntoIterator<Item = Point3>) -> Self {
        self.holes.push(hole.into_iter().collect());
        self
    }

    pub fn material(mut self, m: MaterialId) -> Self {
        self.material = Some(m);
        self
    }

    pub fn back_material(mut self, m: MaterialId) -> Self {
        self.back_material = Some(m);
        self
    }

    pub fn layer(mut self, l: LayerId) -> Self {
        self.layer = Some(l);
        self
    }

    pub fn auto_triangulate(mut self) -> Self {
        self.auto_triangulate = true;
        self
    }
}

/// A circular face that SketchUp edits as a true circle.
#[derive(Debug, Clone)]
pub struct Circle {
    pub center: Point3,
    /// Plane normal (any nonzero length), also the face's front normal.
    pub normal: Point3,
    pub radius: f64,
    /// Tessellation, 3 to 255 (SketchUp's default is 24).
    pub segments: u32,
    pub material: Option<MaterialId>,
    pub back_material: Option<MaterialId>,
    pub layer: Option<LayerId>,
    pub hidden: bool,
    /// As on [`Face`], but the pins are not scaled by the material's applied
    /// size (OpenSKP only scales them for plain faces).
    pub front_uv: Option<[UvPin; 3]>,
    pub back_uv: Option<[UvPin; 3]>,
    pub attributes: Vec<AttributeDict>,
}

impl Circle {
    pub fn new(center: Point3, normal: Point3, radius: f64) -> Self {
        Self {
            center,
            normal,
            radius,
            segments: 24,
            material: None,
            back_material: None,
            layer: None,
            hidden: false,
            front_uv: None,
            back_uv: None,
            attributes: Vec::new(),
        }
    }

    pub fn segments(mut self, n: u32) -> Self {
        self.segments = n;
        self
    }

    pub fn material(mut self, m: MaterialId) -> Self {
        self.material = Some(m);
        self
    }

    pub fn layer(mut self, l: LayerId) -> Self {
        self.layer = Some(l);
        self
    }
}

/// An open arc (edges only) that SketchUp edits as a true arc. Angles are in
/// radians from a fixed in-plane reference direction derived from the normal.
#[derive(Debug, Clone)]
pub struct Arc {
    pub center: Point3,
    pub normal: Point3,
    pub radius: f64,
    pub start_angle: f64,
    pub end_angle: f64,
    /// Tessellation, 3 to 255.
    pub segments: u32,
    pub edges: EdgeFlags,
}

impl Arc {
    pub fn new(
        center: Point3,
        normal: Point3,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Self {
        Self {
            center,
            normal,
            radius,
            start_angle,
            end_angle,
            segments: 24,
            edges: EdgeFlags::default(),
        }
    }

    pub fn segments(mut self, n: u32) -> Self {
        self.segments = n;
        self
    }
}

/// A freeform polyline curve (`CCurve`): straight edges grouped as one curve.
#[derive(Debug, Clone, Default)]
pub struct Polyline {
    pub points: Vec<Point3>,
    /// Also connect the last point back to the first.
    pub closed: bool,
    pub edges: EdgeFlags,
}

impl Polyline {
    pub fn new(points: impl IntoIterator<Item = Point3>) -> Self {
        Self {
            points: points.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn closed(mut self) -> Self {
        self.closed = true;
        self
    }
}

/// Where and how a component instance, group or image is placed.
#[derive(Debug, Clone)]
pub struct Placement {
    /// Instance name; `None` uses the definition's name. For a nested group
    /// instance an empty name does too.
    pub name: Option<String>,
    pub translation: Point3,
    /// Row-major 3x3 rotation/scale, applied before the translation.
    pub matrix: Option<[f64; 9]>,
    /// `(axis, angle in radians)`: a rotation instead of `matrix` (give at
    /// most one of the two).
    pub rotation: Option<(Point3, f64)>,
    /// Material painted on the instance itself (not used by images).
    pub material: Option<MaterialId>,
    pub layer: Option<LayerId>,
    /// Not used by images.
    pub attributes: Vec<AttributeDict>,
    pub hidden: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            name: None,
            translation: [0.0; 3],
            matrix: None,
            rotation: None,
            material: None,
            layer: None,
            attributes: Vec::new(),
            hidden: false,
        }
    }
}

impl Placement {
    pub fn at(translation: Point3) -> Self {
        Self {
            translation,
            ..Self::default()
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn rotated(mut self, axis: Point3, angle: f64) -> Self {
        self.rotation = Some((axis, angle));
        self
    }

    pub fn matrix(mut self, m: [f64; 9]) -> Self {
        self.matrix = Some(m);
        self
    }

    fn resolve_matrix(&self) -> Result<[f64; 9], WriteError> {
        match (self.matrix, self.rotation) {
            (Some(_), Some(_)) => Err(invalid(
                "give at most one of matrix/rotation - rotation is just a convenience for matrix",
            )),
            (Some(m), None) => Ok(m),
            (None, Some((axis, angle))) => rotation_matrix(axis, angle),
            (None, None) => Ok([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
        }
    }
}

/// A construction (guide) line.
#[derive(Debug, Clone, Copy)]
pub enum GuideLine {
    /// A bounded segment between two points.
    Segment { start: Point3, end: Point3 },
    /// An unbounded line through a point.
    Infinite { point: Point3, direction: Point3 },
}

/// A textured material for [`SkpBuilder::add_texture_material`].
#[derive(Debug, Clone)]
pub struct TextureMaterial<'a> {
    pub name: &'a str,
    /// PNG or JPEG file contents, embedded verbatim.
    pub image: &'a [u8],
    /// Stored as the texture's file name (SketchUp stores the source path).
    pub path: &'a str,
    /// Model-space size of one tile in inches (default 1.0 each).
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub opacity: Option<f64>,
}

impl<'a> TextureMaterial<'a> {
    pub fn new(name: &'a str, image: &'a [u8], path: &'a str) -> Self {
        Self {
            name,
            image,
            path,
            width: None,
            height: None,
            opacity: None,
        }
    }
}

/// A GUID generator: SplitMix64 seeded with `seed`, each GUID being two
/// consecutive outputs in little-endian order. Used by
/// [`SkpBuilder::deterministic`]; trivial to reproduce in other languages.
pub fn splitmix_guids(seed: u64) -> impl FnMut() -> [u8; 16] {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    move || {
        let mut g = [0u8; 16];
        g[..8].copy_from_slice(&next().to_le_bytes());
        g[8..].copy_from_slice(&next().to_le_bytes());
        g
    }
}

/// A random version-4 GUID (std's randomly keyed hasher as the entropy source).
fn random_guid() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut g = [0u8; 16];
    for half in g.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    g[6] = (g[6] & 0x0f) | 0x40;
    g[8] = (g[8] & 0x3f) | 0x80;
    g
}

fn now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
}

fn check_points(points: &[Point3]) -> Result<(), WriteError> {
    if points.iter().flatten().all(|c| c.is_finite()) {
        Ok(())
    } else {
        Err(invalid("coordinates must be finite"))
    }
}

fn check_string(s: &str) -> Result<(), WriteError> {
    if s.encode_utf16().count() >= 0xff {
        return Err(invalid(format!(
            "string too long to encode (255 char limit): {s:?}"
        )));
    }
    Ok(())
}

/// Rejects anything that would fail halfway through writing, so a failed call
/// leaves the file consistent.
fn check_dicts(dicts: &[AttributeDict]) -> Result<(), WriteError> {
    fn value(v: &AttributeValue) -> Result<(), WriteError> {
        match v {
            AttributeValue::String(s) => check_string(s),
            AttributeValue::Array(items) => items.iter().try_for_each(value),
            _ => Ok(()),
        }
    }
    for d in dicts {
        check_string(&d.name)?;
        for (k, v) in &d.entries {
            check_string(k)?;
            value(v)?;
        }
    }
    Ok(())
}

/// Scales texture pins from tiles to inches of texture space by the
/// material's applied size.
fn scale_pins(pins: Option<[UvPin; 3]>, size: Option<(f64, f64)>) -> Option<[UvPin; 3]> {
    match (pins, size) {
        (Some(p), Some((w, h))) if !(w == 1.0 && h == 1.0) => Some(p.map(|pin| UvPin {
            point: pin.point,
            uv: [pin.uv[0] * w, pin.uv[1] * h],
        })),
        _ => pins,
    }
}

/// The CDib format tag from the image's magic bytes: 4 PNG, 1 JPEG.
fn image_subtype(image: &[u8]) -> Result<u32, WriteError> {
    if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(4)
    } else if image.starts_with(b"\xff\xd8\xff") {
        Ok(1)
    } else {
        Err(invalid(
            "unrecognized image format - only PNG and JPEG textures are supported",
        ))
    }
}

/// Which entity list an operation writes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Root,
    Definition,
}

/// A face as the archive writer takes it, after handle resolution.
struct FaceArgs<'a> {
    points: &'a [Point3],
    holes: &'a [Vec<Point3>],
    material: u32,
    back_material: u32,
    layer: u32,
    hidden: bool,
    edges: EdgeFlags,
    front_uv: Option<[UvPin; 3]>,
    back_uv: Option<[UvPin; 3]>,
    attributes: &'a [AttributeDict],
    curve: CurveKind,
}

/// Builds a new legacy-format (v17) `.skp` file. See the module docs.
///
/// ```
/// use mt_skp::write::{Face, Placement, SkpBuilder};
///
/// let mut skp = SkpBuilder::new();
/// let red = skp.add_material("Red", [255, 0, 0, 255])?;
/// let mut chair = skp.add_component_definition("Chair", None)?;
/// chair.add_face(&Face::new([[0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 20.0, 0.0]]).material(red))?;
/// let chair = chair.finish()?;
/// skp.add_instance(&chair, &Placement::at([100.0, 0.0, 0.0]))?;
/// let bytes = skp.to_bytes()?;
/// assert!(mt_skp::parse(&bytes).is_ok());
/// # Ok::<(), mt_skp::write::WriteError>(())
/// ```
pub struct SkpBuilder {
    owner: u64,
    guid: Box<dyn FnMut() -> [u8; 16]>,
    clock: Box<dyn FnMut() -> u32>,

    material_writer: ArchiveWriter,
    materials_by_name: HashMap<String, MaterialId>,
    material_slots: HashSet<u32>,
    material_count: u32,
    /// Applied tile size of each textured material slot.
    applied_sizes: HashMap<u32, (f64, f64)>,

    layer_writer: Option<ArchiveWriter>,
    layer_writer_start: u32,
    layers_by_name: HashMap<String, LayerId>,
    layer_slots: HashSet<u32>,
    layer_count: u32,

    definition_writer: Option<ArchiveWriter>,
    definition_writer_start: u32,
    definition_count: u32,
    open_definition: Option<String>,
    /// Groups whose placement waits for the root section to start.
    pending_groups: Vec<(DefinitionId, Placement)>,

    geometry_writer: Option<ArchiveWriter>,
    body: Body,
    root_items: usize,
    font_slot: Option<u32>,
}

impl Default for SkpBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SkpBuilder {
    /// A new, empty document with random GUIDs and the current time.
    pub fn new() -> Self {
        static OWNERS: AtomicU64 = AtomicU64::new(1);
        let pid_start = u64::from(u32::from_le_bytes(
            SCAFFOLD[PID_COUNTER_POS..PID_COUNTER_POS + 4]
                .try_into()
                .expect("4 bytes"),
        )) + 1;
        Self {
            owner: OWNERS.fetch_add(1, Ordering::Relaxed),
            guid: Box::new(random_guid),
            clock: Box::new(now),
            material_writer: ArchiveWriter::new(BASE_SLOT, HashMap::new(), pid_start),
            materials_by_name: HashMap::new(),
            material_slots: HashSet::new(),
            material_count: 0,
            applied_sizes: HashMap::new(),
            layer_writer: None,
            layer_writer_start: 0,
            layers_by_name: HashMap::new(),
            layer_slots: HashSet::new(),
            layer_count: 0,
            definition_writer: None,
            definition_writer_start: 0,
            definition_count: 0,
            open_definition: None,
            pending_groups: Vec::new(),
            geometry_writer: None,
            body: Body::default(),
            root_items: 0,
            font_slot: None,
        }
    }

    /// A builder whose GUIDs come from [`splitmix_guids`]`(seed)` and whose
    /// definitions are all stamped `timestamp` (Unix seconds).
    pub fn deterministic(seed: u64, timestamp: u32) -> Self {
        let mut b = Self::new();
        b.set_guid_source(splitmix_guids(seed));
        b.set_clock(move || timestamp);
        b
    }

    /// Replaces the GUID generator (called once per definition, instance,
    /// group and image, in call order; groups when the root section starts).
    pub fn set_guid_source(&mut self, f: impl FnMut() -> [u8; 16] + 'static) {
        self.guid = Box::new(f);
    }

    /// Replaces the clock stamping definitions (Unix seconds).
    pub fn set_clock(&mut self, f: impl FnMut() -> u32 + 'static) {
        self.clock = Box::new(f);
    }

    /// Material handles by name.
    pub fn material(&self, name: &str) -> Option<MaterialId> {
        self.materials_by_name.get(name).copied()
    }

    /// Layer handles by name.
    pub fn layer(&self, name: &str) -> Option<LayerId> {
        self.layers_by_name.get(name).copied()
    }

    fn check_material_section(&self, what: &str) -> Result<(), WriteError> {
        if self.geometry_writer.is_some() {
            return Err(order(format!(
                "{what} must be called before any root geometry"
            )));
        }
        if self.layer_writer.is_some() {
            return Err(order(format!(
                "{what} must be called before any add_layer calls"
            )));
        }
        if self.definition_writer.is_some() {
            return Err(order(format!(
                "{what} must be called before any add_component_definition calls"
            )));
        }
        Ok(())
    }

    fn register_material(&mut self, name: &str, slot: u32) -> Result<MaterialId, WriteError> {
        if slot > 0xffff {
            return Err(invalid("too many materials"));
        }
        let id = MaterialId {
            owner: self.owner,
            slot,
        };
        self.materials_by_name.insert(name.to_owned(), id);
        self.material_slots.insert(slot);
        self.material_count += 1;
        Ok(id)
    }

    /// A solid-color material. Registering a name again returns the first
    /// handle. Must precede layers, definitions and geometry.
    pub fn add_material(&mut self, name: &str, rgba: [u8; 4]) -> Result<MaterialId, WriteError> {
        self.add_material_with_opacity(name, rgba, None)
    }

    /// [`add_material`](Self::add_material) with an explicit opacity (0..1).
    pub fn add_material_with_opacity(
        &mut self,
        name: &str,
        rgba: [u8; 4],
        opacity: Option<f64>,
    ) -> Result<MaterialId, WriteError> {
        self.check_material_section("add_material")?;
        if let Some(&id) = self.materials_by_name.get(name) {
            return Ok(id);
        }
        check_string(name)?;
        let slot = self.material_writer.material(name, rgba, opacity)?;
        self.register_material(name, slot)
    }

    /// A PNG or JPEG textured material. Same ordering rules as
    /// [`add_material`](Self::add_material).
    pub fn add_texture_material(
        &mut self,
        t: &TextureMaterial<'_>,
    ) -> Result<MaterialId, WriteError> {
        self.check_material_section("add_texture_material")?;
        if let Some(&id) = self.materials_by_name.get(t.name) {
            return Ok(id);
        }
        let subtype = image_subtype(t.image)?;
        check_string(t.name)?;
        check_string(t.path)?;
        let size = (t.width.unwrap_or(1.0), t.height.unwrap_or(1.0));
        let slot = self
            .material_writer
            .textured_material(t.name, t.image, t.path, subtype, size, t.opacity)?;
        self.applied_sizes.insert(slot, size);
        self.register_material(t.name, slot)
    }

    /// [`add_texture_material`](Self::add_texture_material) from an image
    /// file; the path is stored as given.
    pub fn add_texture_material_file(
        &mut self,
        name: &str,
        path: impl AsRef<Path>,
    ) -> Result<MaterialId, WriteError> {
        self.check_material_section("add_texture_material")?;
        if let Some(&id) = self.materials_by_name.get(name) {
            return Ok(id);
        }
        let image = std::fs::read(path.as_ref())?;
        let path = path.as_ref().to_string_lossy();
        self.add_texture_material(&TextureMaterial::new(name, &image, &path))
    }

    fn material_shift(&self) -> u32 {
        self.material_writer.next_slot - BASE_SLOT
    }

    fn material_shifted_class_slot(&self) -> HashMap<String, u32> {
        let shift = self.material_shift();
        SCAFFOLD_CLASS_SLOTS
            .iter()
            .map(|&(n, s)| (n.to_owned(), s + shift))
            .collect()
    }

    fn layer_shift(&self) -> u32 {
        self.layer_writer
            .as_ref()
            .map_or(0, |w| w.next_slot - self.layer_writer_start)
    }

    fn post_layer_class_slot(&self) -> HashMap<String, u32> {
        match &self.layer_writer {
            Some(w) => w.class_slot.clone(),
            None => self.material_shifted_class_slot(),
        }
    }

    fn definition_shift(&self) -> u32 {
        self.definition_writer
            .as_ref()
            .map_or(0, |w| w.next_slot - self.definition_writer_start)
    }

    fn post_definition_class_slot(&self) -> HashMap<String, u32> {
        match &self.definition_writer {
            Some(w) => w.class_slot.clone(),
            None => self.post_layer_class_slot(),
        }
    }

    /// The next free persistent id: sections continue one sequence.
    fn next_pid(&self) -> u64 {
        [
            &self.geometry_writer,
            &self.definition_writer,
            &self.layer_writer,
        ]
        .into_iter()
        .flatten()
        .next()
        .map_or(self.material_writer.next_pid, |w| w.next_pid)
    }

    /// A layer (SketchUp 2020+ "tag"). Registering a name again returns the
    /// first handle and ignores the other arguments. Must precede definitions
    /// and geometry.
    pub fn add_layer(
        &mut self,
        name: &str,
        color: Option<[u8; 4]>,
        visible: bool,
    ) -> Result<LayerId, WriteError> {
        if self.geometry_writer.is_some() {
            return Err(order("add_layer must be called before any root geometry"));
        }
        if self.definition_writer.is_some() {
            return Err(order(
                "add_layer must be called before any add_component_definition calls",
            ));
        }
        if let Some(&id) = self.layers_by_name.get(name) {
            return Ok(id);
        }
        check_string(name)?;
        check_string(&format!("Layer_{name}"))?;
        if self.layer_writer.is_none() {
            self.layer_writer_start = LAYER_WRITER_BASE + self.material_shift();
            self.layer_writer = Some(ArchiveWriter::new(
                self.layer_writer_start,
                self.material_shifted_class_slot(),
                self.material_writer.next_pid,
            ));
        }
        let w = self.layer_writer.as_mut().expect("just created");
        let slot = w.layer(name, true, !visible, color)?;
        if slot > 0xffff {
            return Err(invalid("too many layers"));
        }
        let id = LayerId {
            owner: self.owner,
            slot,
        };
        self.layers_by_name.insert(name.to_owned(), id);
        self.layer_slots.insert(slot);
        self.layer_count += 1;
        Ok(id)
    }

    fn material_slot(&self, m: Option<MaterialId>) -> Result<u32, WriteError> {
        match m {
            None => Ok(0),
            Some(m) if m.owner == self.owner && self.material_slots.contains(&m.slot) => Ok(m.slot),
            Some(_) => Err(invalid("material handle does not belong to this builder")),
        }
    }

    fn layer_slot(&self, l: Option<LayerId>) -> Result<u32, WriteError> {
        match l {
            None => Ok(0),
            Some(l) if l.owner == self.owner && self.layer_slots.contains(&l.slot) => Ok(l.slot),
            Some(_) => Err(invalid("layer handle does not belong to this builder")),
        }
    }

    fn check_definition(&self, d: &DefinitionId) -> Result<(), WriteError> {
        if d.owner != self.owner {
            return Err(invalid(format!(
                "component definition {:?} belongs to a different builder",
                d.name
            )));
        }
        Ok(())
    }

    fn start_definition(
        &mut self,
        name: String,
        description: String,
        caller: &str,
        attributes: &[AttributeDict],
        behavior: u8,
        group: Option<Placement>,
    ) -> Result<DefinitionBuilder<'_>, WriteError> {
        if self.geometry_writer.is_some() {
            return Err(order(format!(
                "{caller} must be called before any root geometry ({} already written); \
                 collect all definitions first, then place instances",
                self.root_items
            )));
        }
        if let Some(open) = &self.open_definition {
            return Err(order(format!(
                "component definition {open:?} is still open - finish it before starting another"
            )));
        }
        check_string(&name)?;
        check_string(&description)?;
        check_dicts(attributes)?;
        if self.definition_writer.is_none() {
            self.definition_writer_start =
                SCAFFOLD_NEXT_SLOT + self.material_shift() + self.layer_shift();
            self.definition_writer = Some(ArchiveWriter::new(
                self.definition_writer_start,
                self.post_layer_class_slot(),
                self.next_pid(),
            ));
        }
        let w = self.definition_writer.as_mut().expect("just created");
        let (slot, count_pos) = w.definition_header(attributes)?;
        self.definition_count += 1;
        self.open_definition = Some(name.clone());
        Ok(DefinitionBuilder {
            skp: self,
            slot,
            name,
            description,
            behavior,
            count_pos,
            body: Body::default(),
            group,
        })
    }

    /// Starts a reusable component definition; add its geometry, then
    /// [`finish`](DefinitionBuilder::finish) it and place it with
    /// [`add_instance`](Self::add_instance). Must precede root geometry.
    pub fn add_component_definition(
        &mut self,
        name: &str,
        description: Option<&str>,
    ) -> Result<DefinitionBuilder<'_>, WriteError> {
        self.add_component_definition_with(name, description, &[], false, false)
    }

    /// [`add_component_definition`](Self::add_component_definition) with
    /// attribute dictionaries and the face-me behaviors (`always_faces_camera`
    /// for 2D people and cut-out trees, `shadows_face_sun` to keep their
    /// shadows still).
    pub fn add_component_definition_with(
        &mut self,
        name: &str,
        description: Option<&str>,
        attributes: &[AttributeDict],
        always_faces_camera: bool,
        shadows_face_sun: bool,
    ) -> Result<DefinitionBuilder<'_>, WriteError> {
        let behavior = u8::from(always_faces_camera) | (u8::from(shadows_face_sun) << 1);
        self.start_definition(
            name.to_owned(),
            description.unwrap_or("").to_owned(),
            "add_component_definition",
            attributes,
            behavior,
            None,
        )
    }

    /// Starts a group, placed automatically by `placement` once finished.
    /// The definition is named after `placement.name` (default "Group").
    pub fn add_group(&mut self, placement: Placement) -> Result<DefinitionBuilder<'_>, WriteError> {
        self.material_slot(placement.material)?;
        self.layer_slot(placement.layer)?;
        placement.resolve_matrix()?;
        check_dicts(&placement.attributes)?;
        let name = placement
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Group".to_owned());
        self.start_definition(name, String::new(), "add_group", &[], 0, Some(placement))
    }

    /// Starts the root section. Later material, layer and definition calls
    /// fail; root geometry calls do this implicitly.
    pub fn begin_root(&mut self) -> Result<(), WriteError> {
        self.ensure_geometry_writer()
    }

    fn ensure_geometry_writer(&mut self) -> Result<(), WriteError> {
        if self.geometry_writer.is_some() {
            return Ok(());
        }
        if let Some(open) = &self.open_definition {
            return Err(order(format!(
                "component definition {open:?} is still open - finish it before adding root geometry"
            )));
        }
        let start = SCAFFOLD_NEXT_SLOT
            + self.material_shift()
            + self.layer_shift()
            + self.definition_shift();
        let mut w = ArchiveWriter::new(start, self.post_definition_class_slot(), self.next_pid());
        for (def, p) in std::mem::take(&mut self.pending_groups) {
            let rec = InstanceRecord {
                definition_slot: def.slot,
                name: &def.name,
                translation: p.translation,
                matrix: p.resolve_matrix()?,
                material: self.material_slot(p.material)?,
                layer: self.layer_slot(p.layer)?,
                attributes: &p.attributes,
                hidden: p.hidden,
                guid: (self.guid)(),
            };
            w.instance_like(InstanceKind::Group, &rec)?;
            self.body.new_entities += 1;
            self.root_items += 1;
        }
        self.geometry_writer = Some(w);
        Ok(())
    }

    fn writer(&mut self, s: Section) -> &mut ArchiveWriter {
        match s {
            Section::Root => self.geometry_writer.as_mut(),
            Section::Definition => self.definition_writer.as_mut(),
        }
        .expect("section writer exists")
    }

    /// Runs `f` on the root body and root writer.
    fn with_root<T>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut Body) -> Result<T, WriteError>,
    ) -> Result<T, WriteError> {
        let mut body = std::mem::take(&mut self.body);
        let r = f(self, &mut body);
        // Groups placed while the root body was out were counted on the
        // placeholder.
        let placed = std::mem::replace(&mut self.body, body).new_entities;
        self.body.new_entities += placed;
        r
    }

    /// Plane fit, texture fit and hole validation, then the face record.
    fn write_face(
        &mut self,
        s: Section,
        body: &mut Body,
        a: &FaceArgs<'_>,
    ) -> Result<(), WriteError> {
        let plane = geom::plane_from_polygon(a.points)?;
        let normal = [plane[0], plane[1], plane[2]];
        let front_matrix = a
            .front_uv
            .map(|p| geom::uv_matrix_for_face(&p, normal))
            .transpose()?;
        let back_matrix = a
            .back_uv
            .map(|p| geom::uv_matrix_for_face(&p, normal))
            .transpose()?;
        check_dicts(a.attributes)?;
        let tol = geom::tolerance(a.points);
        for hole in a.holes {
            if hole.len() < 3 {
                return Err(invalid("a hole needs at least 3 points"));
            }
            for &p in hole {
                let dist = geom::plane_distance(plane, p);
                if dist.abs() > tol {
                    return Err(invalid(format!(
                        "hole point {p:?} is {:.6e} units off the face's plane",
                        dist.abs()
                    )));
                }
            }
        }
        let rec = FaceRecord {
            material: a.material,
            back_material: a.back_material,
            layer: a.layer,
            hidden: a.hidden,
            edges: a.edges,
            front_matrix,
            back_matrix,
            attributes: a.attributes,
            curve: a.curve,
        };
        self.writer(s).face(a.points, a.holes, plane, body, &rec)
    }

    fn face_op(&mut self, s: Section, body: &mut Body, f: &Face) -> Result<(), WriteError> {
        let material = self.material_slot(f.material)?;
        let back_material = self.material_slot(f.back_material)?;
        let layer = self.layer_slot(f.layer)?;
        if f.outer.len() < 3 {
            return Err(invalid("a face needs at least 3 points"));
        }
        check_points(&f.outer)?;
        f.holes.iter().try_for_each(|h| check_points(h))?;
        if s == Section::Root {
            self.ensure_geometry_writer()?;
        }
        let front_uv = scale_pins(f.front_uv, self.applied_sizes.get(&material).copied());
        let back_uv = scale_pins(f.back_uv, self.applied_sizes.get(&back_material).copied());
        let args = FaceArgs {
            points: &f.outer,
            holes: &f.holes,
            material,
            back_material,
            layer,
            hidden: f.hidden,
            edges: f.edges,
            front_uv,
            back_uv,
            attributes: &f.attributes,
            curve: CurveKind::None,
        };
        if !f.holes.is_empty()
            || !f.auto_triangulate
            || f.outer.len() == 3
            || geom::is_coplanar(&f.outer)?
        {
            return self.write_face(s, body, &args);
        }
        if front_uv.is_some() || back_uv.is_some() {
            return Err(invalid(
                "auto_triangulate cannot be combined with front_uv/back_uv positioning",
            ));
        }
        for i in 1..f.outer.len() - 1 {
            let tri = [f.outer[0], f.outer[i], f.outer[i + 1]];
            self.write_face(
                s,
                body,
                &FaceArgs {
                    points: &tri,
                    ..args
                },
            )?;
        }
        Ok(())
    }

    fn circle_op(&mut self, s: Section, body: &mut Body, c: &Circle) -> Result<(), WriteError> {
        let material = self.material_slot(c.material)?;
        let back_material = self.material_slot(c.back_material)?;
        let layer = self.layer_slot(c.layer)?;
        if !(3..=255).contains(&c.segments) {
            return Err(invalid(format!(
                "segments must be between 3 and 255, got {}",
                c.segments
            )));
        }
        check_points(&[c.center, c.normal, [c.radius; 3]])?;
        let normal = geom::normalize3(c.normal)?;
        if s == Section::Root {
            self.ensure_geometry_writer()?;
        }
        let (u, w) = geom::circle_basis(normal)?;
        let curve = CurveKind::Arc(ArcParams {
            center: c.center,
            normal,
            xaxis: u.map(|x| c.radius * x),
            start_angle: 0.0,
            end_angle: 2.0 * std::f64::consts::PI,
            radius: c.radius,
            segments: c.segments,
        });
        let points = geom::circle_points(c.center, c.radius, c.segments, u, w);
        let args = FaceArgs {
            points: &points,
            holes: &[],
            material,
            back_material,
            layer,
            hidden: c.hidden,
            edges: EdgeFlags::default(),
            front_uv: c.front_uv,
            back_uv: c.back_uv,
            attributes: &c.attributes,
            curve,
        };
        self.write_face(s, body, &args)
    }

    fn arc_op(&mut self, s: Section, body: &mut Body, a: &Arc) -> Result<(), WriteError> {
        if !(3..=255).contains(&a.segments) {
            return Err(invalid(format!(
                "segments must be between 3 and 255, got {}",
                a.segments
            )));
        }
        if a.end_angle == a.start_angle {
            return Err(invalid(
                "start_angle and end_angle must differ - use a circle for a full circle",
            ));
        }
        check_points(&[a.center, a.normal, [a.radius, a.start_angle, a.end_angle]])?;
        let normal = geom::normalize3(a.normal)?;
        if s == Section::Root {
            self.ensure_geometry_writer()?;
        }
        let (u, w) = geom::circle_basis(normal)?;
        let curve = CurveKind::Arc(ArcParams {
            center: a.center,
            normal,
            xaxis: u.map(|x| a.radius * x),
            start_angle: a.start_angle,
            end_angle: a.end_angle,
            radius: a.radius,
            segments: a.segments,
        });
        let points = geom::arc_points(
            a.center,
            a.radius,
            a.segments,
            u,
            w,
            a.start_angle,
            a.end_angle,
        );
        self.writer(s).edges(&points, body, false, a.edges, curve);
        Ok(())
    }

    fn polyline_op(&mut self, s: Section, body: &mut Body, p: &Polyline) -> Result<(), WriteError> {
        if p.points.len() < 2 {
            return Err(invalid("a polyline needs at least 2 points"));
        }
        check_points(&p.points)?;
        if s == Section::Root {
            self.ensure_geometry_writer()?;
        }
        let n = p.points.len() as u32;
        let pairs = if p.closed { n } else { n - 1 };
        self.writer(s).edges(
            &p.points,
            body,
            p.closed,
            p.edges,
            CurveKind::Polyline(pairs),
        );
        Ok(())
    }

    fn instance_op(
        &mut self,
        s: Section,
        body: &mut Body,
        kind: InstanceKind,
        def: &DefinitionId,
        p: &Placement,
    ) -> Result<(), WriteError> {
        let material = self.material_slot(p.material)?;
        let layer = self.layer_slot(p.layer)?;
        self.check_definition(def)?;
        let matrix = p.resolve_matrix()?;
        check_points(&[p.translation])?;
        check_dicts(&p.attributes)?;
        if s == Section::Root {
            self.ensure_geometry_writer()?;
        }
        let name = match (kind, p.name.as_deref()) {
            (InstanceKind::Group, Some("")) | (_, None) => def.name.as_str(),
            (_, Some(n)) => n,
        };
        check_string(name)?;
        let rec = InstanceRecord {
            definition_slot: def.slot,
            name,
            translation: p.translation,
            matrix,
            material,
            layer,
            attributes: &p.attributes,
            hidden: p.hidden,
            guid: (self.guid)(),
        };
        self.writer(s).instance_like(kind, &rec)?;
        body.new_entities += 1;
        Ok(())
    }

    fn text_op(
        &mut self,
        s: Section,
        body: &mut Body,
        text: &str,
        point: Point3,
        leader: Point3,
    ) -> Result<(), WriteError> {
        check_string(text)?;
        check_points(&[point, leader])?;
        if s == Section::Root {
            self.ensure_geometry_writer()?;
        }
        let label = [0, 1, 2].map(|i| point[i] + leader[i]);
        let mut font = self.font_slot;
        self.writer(s).text(text, point, label, &mut font)?;
        self.font_slot = font;
        body.new_entities += 1;
        Ok(())
    }

    fn construction_point_op(
        &mut self,
        s: Section,
        body: &mut Body,
        p: Point3,
    ) -> Result<(), WriteError> {
        check_points(&[p])?;
        if s == Section::Root {
            self.ensure_geometry_writer()?;
        }
        self.writer(s).construction_point(p);
        body.new_entities += 1;
        Ok(())
    }

    /// Root-level face. Vertices and edges are shared with earlier root faces
    /// wherever coordinates are exactly equal.
    pub fn add_face(&mut self, face: &Face) -> Result<(), WriteError> {
        self.with_root(|s, b| s.face_op(Section::Root, b, face))?;
        self.root_items += 1;
        Ok(())
    }

    /// Root-level circular face.
    pub fn add_circle(&mut self, circle: &Circle) -> Result<(), WriteError> {
        self.with_root(|s, b| s.circle_op(Section::Root, b, circle))?;
        self.root_items += 1;
        Ok(())
    }

    /// Root-level open arc.
    pub fn add_arc(&mut self, arc: &Arc) -> Result<(), WriteError> {
        self.with_root(|s, b| s.arc_op(Section::Root, b, arc))?;
        self.root_items += 1;
        Ok(())
    }

    /// Root-level polyline curve.
    pub fn add_polyline(&mut self, polyline: &Polyline) -> Result<(), WriteError> {
        self.with_root(|s, b| s.polyline_op(Section::Root, b, polyline))?;
        self.root_items += 1;
        Ok(())
    }

    /// Places a component instance at the root.
    pub fn add_instance(
        &mut self,
        def: &DefinitionId,
        placement: &Placement,
    ) -> Result<(), WriteError> {
        self.with_root(|s, b| {
            s.instance_op(Section::Root, b, InstanceKind::Component, def, placement)
        })?;
        self.root_items += 1;
        Ok(())
    }

    /// Places an image entity (SketchUp's File > Import > Image): a textured
    /// `width` x `height` quad in the XY plane, positioned by `placement`
    /// (its name, material and attributes are not used). Registers a texture
    /// material and a definition, so it is subject to both their ordering
    /// rules and starts the root section.
    ///
    /// OpenSKP could not calibrate the `CImage` schema against a real
    /// SketchUp file; check the output in SketchUp before relying on it.
    pub fn add_image(
        &mut self,
        image: &[u8],
        path: &str,
        width: f64,
        height: f64,
        placement: &Placement,
    ) -> Result<(), WriteError> {
        let layer = self.layer_slot(placement.layer)?;
        let name = format!("__openskp_image_{}", self.material_count);
        let mat = self.add_texture_material(&TextureMaterial::new(&name, image, path))?;
        let def_name = format!("Image{}", self.definition_count);
        let mut def = self.add_component_definition(&def_name, None)?;
        def.add_face(&Face {
            front_uv: Some([
                UvPin {
                    point: [0.0, 0.0, 0.0],
                    uv: [0.0, 0.0],
                },
                UvPin {
                    point: [width, 0.0, 0.0],
                    uv: [1.0, 0.0],
                },
                UvPin {
                    point: [0.0, height, 0.0],
                    uv: [0.0, 1.0],
                },
            ]),
            ..Face::new([
                [0.0, 0.0, 0.0],
                [width, 0.0, 0.0],
                [width, height, 0.0],
                [0.0, height, 0.0],
            ])
            .material(mat)
        })?;
        let def = def.finish()?;
        let matrix = placement.resolve_matrix()?;
        check_points(&[placement.translation])?;
        self.ensure_geometry_writer()?;
        let rec = InstanceRecord {
            definition_slot: def.slot,
            name: "",
            translation: placement.translation,
            matrix,
            material: 0,
            layer,
            attributes: &[],
            hidden: placement.hidden,
            guid: (self.guid)(),
        };
        self.writer(Section::Root)
            .instance_like(InstanceKind::Image, &rec)?;
        self.body.new_entities += 1;
        self.root_items += 1;
        Ok(())
    }

    /// A free linear dimension between two points; `offset` is the signed
    /// distance of the dimension line from the measured segment.
    pub fn add_dimension(&mut self, p1: Point3, p2: Point3, offset: f64) -> Result<(), WriteError> {
        check_points(&[p1, p2, [offset; 3]])?;
        self.ensure_geometry_writer()?;
        if p1 == p2 {
            return Err(invalid("dimension endpoints coincide"));
        }
        let mut font = self.font_slot;
        self.writer(Section::Root)
            .dimension(p1, p2, offset, &mut font)?;
        self.font_slot = font;
        self.body.new_entities += 1;
        self.root_items += 1;
        Ok(())
    }

    /// A leader text anchored at `point` with its label at `point + leader`
    /// (see [`DEFAULT_LEADER`]).
    pub fn add_text(
        &mut self,
        text: &str,
        point: Point3,
        leader: Point3,
    ) -> Result<(), WriteError> {
        self.with_root(|s, b| s.text_op(Section::Root, b, text, point, leader))?;
        self.root_items += 1;
        Ok(())
    }

    /// A construction (guide) line.
    pub fn add_construction_line(&mut self, line: GuideLine) -> Result<(), WriteError> {
        let (point, dir, start, end) = match line {
            GuideLine::Segment { start, end } => {
                check_points(&[start, end])?;
                let d = [0, 1, 2].map(|i| end[i] - start[i]);
                let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if length == 0.0 {
                    return Err(invalid("construction line endpoints coincide"));
                }
                (start, d.map(|c| c / length), 0.0, length)
            }
            GuideLine::Infinite { point, direction } => {
                check_points(&[point, direction])?;
                let d = direction;
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len == 0.0 {
                    return Err(invalid("construction line direction must be nonzero"));
                }
                (point, d.map(|c| c / len), -CLINE_INFINITE, CLINE_INFINITE)
            }
        };
        self.ensure_geometry_writer()?;
        self.writer(Section::Root)
            .construction_line(point, dir, start, end);
        self.body.new_entities += 1;
        self.root_items += 1;
        Ok(())
    }

    /// A construction (guide) point.
    pub fn add_construction_point(&mut self, position: Point3) -> Result<(), WriteError> {
        self.with_root(|s, b| s.construction_point_op(Section::Root, b, position))?;
        self.root_items += 1;
        Ok(())
    }

    /// A section plane through `point` with `normal` (any nonzero length).
    pub fn add_section_plane(&mut self, point: Point3, normal: Point3) -> Result<(), WriteError> {
        check_points(&[point, normal])?;
        let n = normal;
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len == 0.0 {
            return Err(invalid("section plane normal must be nonzero"));
        }
        let (a, b, c) = (n[0] / len, n[1] / len, n[2] / len);
        let d = -(a * point[0] + b * point[1] + c * point[2]);
        self.ensure_geometry_writer()?;
        self.writer(Section::Root).section_plane([a, b, c, d]);
        self.body.new_entities += 1;
        self.root_items += 1;
        Ok(())
    }

    /// The finished file. Places any pending groups first.
    pub fn to_bytes(&mut self) -> Result<Vec<u8>, WriteError> {
        if !self.pending_groups.is_empty() {
            self.ensure_geometry_writer()?;
        }
        let Some(geometry) = &self.geometry_writer else {
            return Err(invalid("no geometry added"));
        };
        if self.root_items == 0 {
            return Err(invalid("no geometry added"));
        }
        let material_shift = self.material_shift();
        let layer_shift = self.layer_shift();
        let definition_shift = self.definition_shift();
        let geometry_start = SCAFFOLD_NEXT_SLOT + material_shift + layer_shift + definition_shift;
        let geometry_shift = geometry.next_slot - geometry_start;
        let d = SCAFFOLD;
        let mut out =
            Vec::with_capacity(d.len() + self.material_writer.buf.len() + geometry.buf.len());

        // The 4 bytes before the material insertion point are the (reserved)
        // material count, overwritten in place.
        let mut prefix = d[..MATERIAL_INSERT_POS - 4].to_vec();
        let last_pid = (self.next_pid() - 1) as u32;
        prefix[PID_COUNTER_POS..PID_COUNTER_POS + 4].copy_from_slice(&last_pid.to_le_bytes());
        prefix[ISO_CAMERA_PREFIX_OFFSET..ISO_CAMERA_PREFIX_OFFSET + ISO_CAMERA_PREFIX_PATCH.len()]
            .copy_from_slice(&ISO_CAMERA_PREFIX_PATCH);
        out.extend_from_slice(&prefix);
        out.extend_from_slice(&self.material_count.to_le_bytes());
        out.extend_from_slice(&self.material_writer.buf);

        // Layer0 and the layer count.
        let mut middle = d[MATERIAL_INSERT_POS..LAYER_INSERT_POS].to_vec();
        let rel = LAYER_COUNT_POS - MATERIAL_INSERT_POS;
        middle[rel..rel + 4].copy_from_slice(&(ORIG_LAYER_COUNT + self.layer_count).to_le_bytes());
        out.extend_from_slice(&middle);
        if let Some(w) = &self.layer_writer {
            out.extend_from_slice(&w.buf);
        }

        // The active-layer anchor: a back-ref to Layer0, which moves only with
        // materials.
        let mut anchor = d[LAYER_INSERT_POS..DEF_COUNT_POS].to_vec();
        if material_shift != 0 {
            shift_ref(&mut anchor, 0, material_shift);
        }
        out.extend_from_slice(&anchor);

        out.extend_from_slice(&(ORIG_DEF_COUNT + self.definition_count).to_le_bytes());
        if let Some(w) = &self.definition_writer {
            out.extend_from_slice(&w.buf);
        }
        out.extend_from_slice(&d[DEF_COUNT_POS + 4..ROOT_COUNT_POS]);
        out.extend_from_slice(&(ORIG_ROOT_COUNT + self.body.new_entities).to_le_bytes());
        out.extend_from_slice(&d[ROOT_COUNT_POS + 4..TAIL_POS]);
        out.extend_from_slice(&geometry.buf);

        // Tail: shift references and patch the camera, in ascending offset
        // order so a widened reference moves everything after it.
        let mut tail = d[TAIL_POS..].to_vec();
        let total_shift = material_shift + layer_shift + definition_shift + geometry_shift;
        let mut actions: Vec<(usize, Option<&[u8]>)> =
            TAIL_REF_POSITIONS.iter().map(|&p| (p, None)).collect();
        actions.push((ISO_CAMERA_TAIL_PATCH_A.0, Some(&ISO_CAMERA_TAIL_PATCH_A.1)));
        actions.push((ISO_CAMERA_TAIL_PATCH_B.0, Some(&ISO_CAMERA_TAIL_PATCH_B.1)));
        actions.sort_by_key(|a| a.0);
        let mut growth = 0;
        for (pos, patch) in actions {
            let here = pos + growth;
            match patch {
                None => growth += shift_ref(&mut tail, here, total_shift),
                Some(p) => tail[here..here + p.len()].copy_from_slice(p),
            }
        }
        out.extend_from_slice(&tail);
        Ok(out)
    }

    /// Writes the finished file to `path`.
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), WriteError> {
        let bytes = self.to_bytes()?;
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

/// An open component definition or group; vertices and edges are shared only
/// within it. Finish it with [`finish`](Self::finish). Dropping it unfinished
/// leaves the builder unable to start another definition or root geometry.
pub struct DefinitionBuilder<'a> {
    skp: &'a mut SkpBuilder,
    slot: u32,
    name: String,
    description: String,
    behavior: u8,
    count_pos: usize,
    body: Body,
    group: Option<Placement>,
}

impl DefinitionBuilder<'_> {
    pub fn add_face(&mut self, face: &Face) -> Result<(), WriteError> {
        self.skp.face_op(Section::Definition, &mut self.body, face)
    }

    pub fn add_circle(&mut self, circle: &Circle) -> Result<(), WriteError> {
        self.skp
            .circle_op(Section::Definition, &mut self.body, circle)
    }

    pub fn add_arc(&mut self, arc: &Arc) -> Result<(), WriteError> {
        self.skp.arc_op(Section::Definition, &mut self.body, arc)
    }

    pub fn add_polyline(&mut self, polyline: &Polyline) -> Result<(), WriteError> {
        self.skp
            .polyline_op(Section::Definition, &mut self.body, polyline)
    }

    /// Nests an instance of an already finished definition.
    pub fn add_instance(
        &mut self,
        def: &DefinitionId,
        placement: &Placement,
    ) -> Result<(), WriteError> {
        self.skp.instance_op(
            Section::Definition,
            &mut self.body,
            InstanceKind::Component,
            def,
            placement,
        )
    }

    /// Nests an already finished definition as a group. (A group cannot be
    /// declared inline inside another definition in this format.)
    pub fn add_group_instance(
        &mut self,
        def: &DefinitionId,
        placement: &Placement,
    ) -> Result<(), WriteError> {
        self.skp.instance_op(
            Section::Definition,
            &mut self.body,
            InstanceKind::Group,
            def,
            placement,
        )
    }

    pub fn add_text(
        &mut self,
        text: &str,
        point: Point3,
        leader: Point3,
    ) -> Result<(), WriteError> {
        self.skp
            .text_op(Section::Definition, &mut self.body, text, point, leader)
    }

    pub fn add_construction_point(&mut self, position: Point3) -> Result<(), WriteError> {
        self.skp
            .construction_point_op(Section::Definition, &mut self.body, position)
    }

    /// Closes the definition (it must contain something). A group is queued
    /// for placement at the root.
    pub fn finish(self) -> Result<DefinitionId, WriteError> {
        if self.body.new_entities == 0 {
            return Err(invalid(format!(
                "component definition {:?} has no geometry",
                self.name
            )));
        }
        let guid = (self.skp.guid)();
        let timestamp = (self.skp.clock)();
        let w = self.skp.writer(Section::Definition);
        w.buf[self.count_pos..self.count_pos + 4]
            .copy_from_slice(&self.body.new_entities.to_le_bytes());
        w.definition_tail(
            &self.name,
            &self.description,
            self.behavior,
            guid,
            timestamp,
        )?;
        self.skp.open_definition = None;
        let id = DefinitionId {
            owner: self.skp.owner,
            slot: self.slot,
            name: self.name,
        };
        if let Some(p) = self.group {
            self.skp.pending_groups.push((id.clone(), p));
        }
        Ok(id)
    }
}
