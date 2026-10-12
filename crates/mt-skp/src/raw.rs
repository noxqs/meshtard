//! Intermediate parse result shared by the VFF and legacy readers, before
//! `model::build_model` resolves ids to names. Port of OpenSKP `internal.hpp`.

use std::collections::BTreeMap;

use crate::model::{
    ArcCurve, AttrDicts, ConstructionLine, ConstructionPoint, Dimension, EntityId, SectionPlane,
    StyleItem, StyleWatermark, TextEntity, Vec3, ViewCamera,
};

/// A node of the VFF `model.dat` TLV tree. Container tags have `children`;
/// everything else keeps its raw `payload`.
#[derive(Debug, Clone, Default)]
pub struct TlvNode {
    pub tag: crate::bytes::Tag,
    pub children: Vec<TlvNode>,
    pub payload: Vec<u8>,
}

/// One loop edge reference: edge id and +1 (same direction) / -1 (reversed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoEdge {
    pub edge_id: EntityId,
    pub orientation: i64,
}

#[derive(Debug, Clone)]
pub struct RawFace {
    /// `loops[0]` is the outer boundary, the rest are holes.
    pub loops: Vec<Vec<CoEdge>>,
    pub normal: Vec3,
    pub material_id: Option<EntityId>,
    pub back_material_id: Option<EntityId>,
    pub uv_transform: Option<[f64; 9]>,
    pub uv_transform_back: Option<[f64; 9]>,
    pub uv_projected: bool,
    pub uv_projected_back: bool,
    pub hidden: bool,
    /// Layer entity id as text until `build_model` resolves it to a name.
    pub layer: String,
}

impl Default for RawFace {
    fn default() -> Self {
        Self {
            loops: Vec::new(),
            normal: [0.0, 0.0, 1.0],
            material_id: None,
            back_material_id: None,
            uv_transform: None,
            uv_transform_back: None,
            uv_projected: false,
            uv_projected_back: false,
            hidden: false,
            layer: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RawInstance {
    pub id: Option<EntityId>,
    pub ref_guid: String,
    pub name: String,
    pub ref_idx: Option<EntityId>,
    pub material_id: Option<EntityId>,
    /// 13 doubles (see `model::Instance::matrix`), or empty for identity.
    pub matrix: Vec<f64>,
    /// Layer entity id as text until `build_model` resolves it to a name.
    pub layer: String,
    /// Stringified view of the `dynamic_attributes` dictionary.
    pub properties: BTreeMap<String, String>,
    pub attribute_dicts: AttrDicts,
    pub hidden: bool,
    /// A group instance (legacy files only so far).
    pub is_group: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GeometryBuilder {
    pub vertices: BTreeMap<EntityId, Vec3>,
    /// Edge id to (start vertex, end vertex).
    pub edges: BTreeMap<EntityId, (Option<EntityId>, Option<EntityId>)>,
    /// Display flag byte: +1 hidden, +8 soft, +16 smooth.
    pub edge_flags: BTreeMap<EntityId, i32>,
    /// Edge id to the id of the curve (arc, circle, polyline) it belongs to.
    pub edge_curves: BTreeMap<EntityId, EntityId>,
    /// Edge id to its own layer id (as text, resolved to the name in `model.rs`).
    pub edge_layers: BTreeMap<EntityId, String>,
    /// Curve id to its analytic arc, for `CArcCurve` curves.
    pub arc_curves: BTreeMap<EntityId, ArcCurve>,
    pub faces: BTreeMap<EntityId, RawFace>,
    pub instances: Vec<RawInstance>,
    pub section_planes: Vec<SectionPlane>,
    pub texts: Vec<TextEntity>,
    pub dimensions: Vec<Dimension>,
    pub construction_lines: Vec<ConstructionLine>,
    pub construction_points: Vec<ConstructionPoint>,
}

#[derive(Debug, Clone, Default)]
pub struct RawDefinition {
    pub guid: String,
    pub name: String,
    pub always_faces_camera: bool,
    pub shadows_face_sun: bool,
    pub is_image: bool,
    pub builder: GeometryBuilder,
}

#[derive(Debug, Clone, Default)]
pub struct RawTexture {
    pub filename: String,
    pub x_scale: f64,
    pub y_scale: f64,
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct RawMaterial {
    pub name: String,
    pub r: i32,
    pub g: i32,
    pub b: i32,
    pub a: i32,
    pub transparency: f64,
    pub colorized: bool,
    pub colorize_type: i32,
    pub texture: Option<RawTexture>,
}

impl Default for RawMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            r: 128,
            g: 128,
            b: 128,
            a: 255,
            transparency: 1.0,
            colorized: false,
            colorize_type: 0,
            texture: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RawStyle {
    pub name: String,
    pub description: String,
    pub front_color: Option<[u8; 3]>,
    pub back_color: Option<[u8; 3]>,
    pub items: BTreeMap<i32, StyleItem>,
    pub watermarks: Vec<StyleWatermark>,
    pub folder: String,
    pub active: bool,
    pub working_copy: bool,
    pub modified: bool,
}

#[derive(Debug, Clone)]
pub struct RawPage {
    pub name: String,
    pub eye: Option<Vec3>,
    pub target: Option<Vec3>,
    pub up: Option<Vec3>,
    pub fov: f64,
    pub parallel: bool,
    pub ortho_height: f64,
    pub hidden_layer_ids: Vec<EntityId>,
    pub selected: bool,
}

impl Default for RawPage {
    fn default() -> Self {
        Self {
            name: String::new(),
            eye: None,
            target: None,
            up: None,
            fov: 35.0,
            parallel: false,
            ortho_height: 0.0,
            hidden_layer_ids: Vec::new(),
            selected: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RawDimension {
    pub a: Vec3,
    pub b: Vec3,
    pub offset: f64,
    pub plane_x: Option<Vec3>,
    pub normal: Option<Vec3>,
    pub offset_direction: Option<Vec3>,
    pub hidden: bool,
    pub text: String,
    /// Layer id (resolved to the name by `build_model`), empty for Layer0.
    pub layer: String,
}

/// Index into `RawParsed::material_store`.
pub type MaterialRef = usize;

#[derive(Debug, Clone)]
pub struct RawParsed {
    pub version: String,
    pub units: Option<String>,
    pub layer_colors: BTreeMap<String, [u8; 3]>,
    /// Layer names in first-seen file order (`layer_colors` is sorted by name).
    pub layer_order: Vec<String>,
    pub layer_hidden: BTreeMap<String, bool>,
    pub layer_attribute_dictionaries: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>,
    pub layer_id_to_name: BTreeMap<EntityId, String>,
    pub pages: Vec<RawPage>,
    pub camera: Option<ViewCamera>,
    pub dimensions: Vec<RawDimension>,
    pub material_id_to_name: BTreeMap<EntityId, String>,
    /// Owns every material; the two maps below index into it (OpenSKP shares
    /// `shared_ptr`s between them).
    pub material_store: Vec<RawMaterial>,
    pub materials: BTreeMap<String, MaterialRef>,
    pub materials_by_folder: BTreeMap<String, MaterialRef>,
    pub styles: Vec<RawStyle>,
    pub definitions: BTreeMap<EntityId, RawDefinition>,
    pub root: RawDefinition,
    pub model_attribute_dicts: AttrDicts,
    pub named_options: BTreeMap<String, BTreeMap<String, crate::model::OptionValue>>,
}

impl Default for RawParsed {
    fn default() -> Self {
        Self {
            version: "unknown".into(),
            units: None,
            layer_colors: BTreeMap::new(),
            layer_order: Vec::new(),
            layer_hidden: BTreeMap::new(),
            layer_attribute_dictionaries: BTreeMap::new(),
            layer_id_to_name: BTreeMap::new(),
            pages: Vec::new(),
            camera: None,
            dimensions: Vec::new(),
            material_id_to_name: BTreeMap::new(),
            material_store: Vec::new(),
            materials: BTreeMap::new(),
            materials_by_folder: BTreeMap::new(),
            styles: Vec::new(),
            definitions: BTreeMap::new(),
            root: RawDefinition {
                guid: "ROOT".into(),
                name: "ROOT_MODEL".into(),
                ..Default::default()
            },
            model_attribute_dicts: AttrDicts::new(),
            named_options: BTreeMap::new(),
        }
    }
}

impl RawParsed {
    /// Adds a material under `name` (replacing an earlier one with the same name, as
    /// OpenSKP's map assignment does) and returns its index.
    pub fn insert_material(&mut self, m: RawMaterial) -> MaterialRef {
        let i = self.material_store.len();
        self.materials.insert(m.name.clone(), i);
        self.material_store.push(m);
        i
    }
}
