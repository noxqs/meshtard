//! The parsed `.skp` model. Port of OpenSKP `model.hpp` / `model.cpp`.
//!
//! Coordinates are raw SketchUp model space: Z up, inches.

use std::collections::BTreeMap;

use crate::raw::{CoEdge, RawDefinition, RawParsed};

pub type EntityId = i64;
pub type Vec3 = [f64; 3];

/// One attribute-dictionary value with the type SketchUp stored.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Null,
    String(String),
    Integer(i64),
    Float(f64),
    Vec(Vec3),
    Array(Vec<AttrValue>),
    Boolean(bool),
    /// SketchUp `Length`, in inches.
    Length(f64),
}

impl std::fmt::Display for AttrValue {
    /// Same text as OpenSKP's `ParsedAttribute::to_string`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => Ok(()),
            Self::String(s) => f.write_str(s),
            Self::Integer(n) => write!(f, "{n}"),
            Self::Boolean(b) => write!(f, "{}", i32::from(*b)),
            Self::Float(x) | Self::Length(x) => write!(f, "{x}"),
            Self::Vec(v) => write!(f, "{},{},{}", v[0], v[1], v[2]),
            Self::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                Ok(())
            }
        }
    }
}

pub type AttrDict = BTreeMap<String, AttrValue>;
pub type AttrDicts = BTreeMap<String, AttrDict>;

pub fn stringify_attr_dict(d: &AttrDict) -> BTreeMap<String, String> {
    d.iter().map(|(k, v)| (k.clone(), v.to_string())).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub id: EntityId,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: EntityId,
    pub v1_id: EntityId,
    pub v2_id: EntityId,
    pub soft: bool,
    pub smooth: bool,
    pub hidden: bool,
    /// The curve (arc, circle, polyline) this edge is a segment of. Legacy files only
    /// so far.
    pub curve_id: Option<EntityId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub id: EntityId,
    /// `loops[0]` is the outer boundary, the rest are holes.
    pub loops: Vec<Vec<CoEdge>>,
    pub normal: Option<Vec3>,
    pub material_id: Option<EntityId>,
    pub back_material_id: Option<EntityId>,
    /// Front 3x3 row-major UV transform.
    pub uv_transform: Option<[f64; 9]>,
    pub uv_transform_back: Option<[f64; 9]>,
    /// Texture is projected: UVs are in the projection plane's frame.
    pub uv_projected: bool,
    pub uv_projected_back: bool,
    pub hidden: bool,
    /// Layer (tag) name; empty for Layer0 in VFF files and for all legacy faces.
    pub layer: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub color: [u8; 3],
    pub hidden: bool,
    /// Legacy files only.
    pub attribute_dictionaries: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub filename: String,
    pub width: f64,
    pub height: f64,
    /// Image file bytes (PNG/JPG/BMP).
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    pub color: [u8; 4],
    /// 0 = fully transparent, 1 = opaque.
    pub transparency: f64,
    pub id: Option<EntityId>,
    pub texture: Option<Texture>,
    pub colorized: bool,
    pub colorize_type: i32,
}

/// One `<sty:item>` of a style.xml.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleItem {
    /// 1 bool, 3/4 int32, 5 ABGR color (older files), 6 float, 7 double, 13 nested XML.
    pub kind: i32,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleWatermark {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub image_path: String,
    pub file_name: String,
    pub image: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    pub name: String,
    pub description: String,
    pub front_color: Option<[u8; 3]>,
    pub back_color: Option<[u8; 3]>,
    /// Every style.xml item by SketchUp's (undocumented) id.
    pub items: BTreeMap<i32, StyleItem>,
    pub watermarks: Vec<StyleWatermark>,
    /// Folder in the ZIP: `styles/<folder>/style.xml`.
    pub folder: String,
    /// The model's current style.
    pub active: bool,
    /// Working copy of the current style (folder `<folder>_1`).
    pub working_copy: bool,
    /// Current style edited without updating it.
    pub modified: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub name: String,
    pub ref_idx: Option<EntityId>,
    pub guid: String,
    /// 13 doubles: row-major 3x3 rotation/scale (0..9), translation (9..12), scale divisor (12).
    /// Empty for identity.
    pub matrix: Vec<f64>,
    /// Explicit layer override name, or empty.
    pub layer: String,
    /// Stringified `dynamic_attributes` dictionary.
    pub properties: BTreeMap<String, String>,
    pub attribute_dictionaries: AttrDicts,
    pub material_id: Option<EntityId>,
    pub hidden: bool,
    /// Persistent id (VFF only).
    pub id: Option<EntityId>,
    /// A group instance (legacy `CGroup`; VFF not read yet).
    pub is_group: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionPlane {
    pub plane: [f64; 4],
    pub name: String,
    pub label: String,
    pub hidden: bool,
}

impl Default for SectionPlane {
    fn default() -> Self {
        Self {
            plane: [0.0, 0.0, 1.0, 0.0],
            name: String::new(),
            label: String::new(),
            hidden: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextEntity {
    pub text: String,
    pub hidden: bool,
    /// The point the leader points at, in the owning definition's frame (world for
    /// the root). `None` for screen texts and for texts anchored to an entity other
    /// than a vertex (e.g. a face). Legacy files only.
    pub point: Option<Vec3>,
    /// Where the label floats (the leader joins it to `point`). `None` for screen
    /// texts. Legacy files only.
    pub label_point: Option<Vec3>,
    /// Screen texts: the label's position as fractions of the view's width and
    /// height (origin unverified, assumed top left). Legacy files only.
    pub screen_position: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dimension {
    /// Displayed text; empty when the measured value is shown.
    pub text: String,
    pub hidden: bool,
    pub a: Option<Vec3>,
    pub b: Option<Vec3>,
    pub offset: f64,
    pub plane_x: Option<Vec3>,
    pub normal: Option<Vec3>,
    /// Unit direction in which a positive `offset` moves the dimension line, when
    /// the reader derives it (legacy files: plane normal x (b - a)). VFF files give
    /// `plane_x` and `normal` instead.
    pub offset_direction: Option<Vec3>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstructionLine {
    pub point: Vec3,
    pub direction: Vec3,
    /// `None` when unbounded in that direction.
    pub start: Option<Vec3>,
    pub end: Option<Vec3>,
}

impl Default for ConstructionLine {
    fn default() -> Self {
        Self {
            point: [0.0; 3],
            direction: [1.0, 0.0, 0.0],
            start: None,
            end: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConstructionPoint {
    pub position: Vec3,
}

/// A saved scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub name: String,
    pub eye: Option<Vec3>,
    pub target: Option<Vec3>,
    pub up: Option<Vec3>,
    pub fov: f64,
    pub parallel: bool,
    pub ortho_height: f64,
    pub hidden_layers: Vec<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewCamera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov: f64,
    pub parallel: bool,
    pub ortho_height: f64,
}

impl Default for ViewCamera {
    fn default() -> Self {
        Self {
            eye: [0.0, 0.0, 1.0],
            target: [0.0; 3],
            up: [0.0, 0.0, 1.0],
            fov: 35.0,
            parallel: false,
            ortho_height: 0.0,
        }
    }
}

/// Component definition, group, or the model root.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Definition {
    pub id: EntityId,
    pub guid: String,
    pub name: String,
    pub vertices: BTreeMap<EntityId, Vertex>,
    pub edges: BTreeMap<EntityId, Edge>,
    pub faces: BTreeMap<EntityId, Face>,
    pub instances: Vec<Instance>,
    pub section_planes: Vec<SectionPlane>,
    pub texts: Vec<TextEntity>,
    pub dimensions: Vec<Dimension>,
    pub construction_lines: Vec<ConstructionLine>,
    pub construction_points: Vec<ConstructionPoint>,
    pub always_faces_camera: bool,
    pub shadows_face_sun: bool,
    pub is_image: bool,
}

/// A typed model option value (SketchUp's options manager, style items).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OptionValue {
    Bool(bool),
    Int(i32),
    Double(f64),
}

impl OptionValue {
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_int(self) -> Option<i32> {
        match self {
            Self::Int(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f64(self) -> Option<f64> {
        match self {
            Self::Double(v) => Some(v),
            Self::Int(v) => Some(f64::from(v)),
            Self::Bool(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// e.g. `{25.0.575}`.
    pub version: String,
    /// Unit system from `meta/meta.dat` (VFF only), e.g. `Millimeter`.
    pub units: Option<String>,
    pub definitions: BTreeMap<EntityId, Definition>,
    pub root: Definition,
    pub layers: Vec<Layer>,
    pub pages: Vec<Page>,
    /// The view the model was saved with (VFF only).
    pub camera: Option<ViewCamera>,
    /// Model-level dimensions with world-space endpoints (VFF), or the root's
    /// dimensions whose endpoints are known (legacy).
    pub dimensions: Vec<Dimension>,
    pub materials: Vec<Material>,
    pub styles: Vec<Style>,
    /// Dictionaries on the model itself (VFF only).
    pub attribute_dictionaries: AttrDicts,
    /// The options manager by section and key (legacy files only), e.g.
    /// `UnitsOptions` / `LengthUnit`. Names are those of SketchUp's Ruby
    /// `OptionsManager`.
    pub named_options: BTreeMap<String, BTreeMap<String, OptionValue>>,
    material_indices: BTreeMap<EntityId, usize>,
}

impl Model {
    pub fn material_by_id(&self, id: EntityId) -> Option<&Material> {
        self.material_indices.get(&id).map(|&i| &self.materials[i])
    }

    /// Every material entity id and the material it names.
    pub fn materials_by_id(&self) -> impl Iterator<Item = (EntityId, &Material)> {
        self.material_indices
            .iter()
            .map(|(&id, &i)| (id, &self.materials[i]))
    }
}

fn definition(id: EntityId, r: RawDefinition) -> Definition {
    let b = r.builder;
    let vertices = b
        .vertices
        .into_iter()
        .map(|(id, p)| {
            let v = Vertex {
                id,
                x: p[0],
                y: p[1],
                z: p[2],
            };
            (id, v)
        })
        .collect();
    let edges = b
        .edges
        .into_iter()
        .map(|(id, (v1, v2))| {
            let flags = b.edge_flags.get(&id).copied().unwrap_or(0);
            let e = Edge {
                id,
                v1_id: v1.unwrap_or(0),
                v2_id: v2.unwrap_or(0),
                soft: flags & 8 != 0,
                smooth: flags & 16 != 0,
                hidden: flags & 1 != 0,
                curve_id: b.edge_curves.get(&id).copied(),
            };
            (id, e)
        })
        .collect();
    let faces = b
        .faces
        .into_iter()
        .map(|(id, f)| {
            let face = Face {
                id,
                loops: f.loops,
                normal: Some(f.normal),
                material_id: f.material_id,
                back_material_id: f.back_material_id,
                uv_transform: f.uv_transform,
                uv_transform_back: f.uv_transform_back,
                uv_projected: f.uv_projected,
                uv_projected_back: f.uv_projected_back,
                hidden: f.hidden,
                layer: f.layer,
            };
            (id, face)
        })
        .collect();
    let instances = b
        .instances
        .into_iter()
        .map(|i| Instance {
            name: i.name,
            ref_idx: i.ref_idx,
            guid: i.ref_guid,
            matrix: i.matrix,
            layer: i.layer,
            properties: i.properties,
            attribute_dictionaries: i.attribute_dicts,
            material_id: i.material_id,
            hidden: i.hidden,
            id: i.id,
            is_group: i.is_group,
        })
        .collect();
    Definition {
        id,
        guid: r.guid,
        name: r.name,
        vertices,
        edges,
        faces,
        instances,
        section_planes: b.section_planes,
        texts: b.texts,
        dimensions: b.dimensions,
        construction_lines: b.construction_lines,
        construction_points: b.construction_points,
        always_faces_camera: r.always_faces_camera,
        shadows_face_sun: r.shadows_face_sun,
        is_image: r.is_image,
    }
}

/// Resolves ids to names and builds the public model.
pub(crate) fn build_model(mut p: RawParsed) -> Model {
    let layer_names = p.layer_id_to_name.clone();
    let resolve = |layer: &mut String| {
        if let Ok(id) = layer.parse::<EntityId>()
            && let Some(name) = layer_names.get(&id)
        {
            *layer = name.clone();
        }
    };
    let resolve_def = |d: &mut RawDefinition| {
        for i in &mut d.builder.instances {
            resolve(&mut i.layer);
        }
        for f in d.builder.faces.values_mut() {
            resolve(&mut f.layer);
        }
    };
    for d in p.definitions.values_mut() {
        resolve_def(d);
    }
    resolve_def(&mut p.root);

    let definitions = std::mem::take(&mut p.definitions)
        .into_iter()
        .map(|(id, d)| (id, definition(id, d)))
        .collect();
    let root = definition(0, std::mem::take(&mut p.root));

    let layers = p
        .layer_order
        .iter()
        .filter_map(|name| {
            let color = *p.layer_colors.get(name)?;
            Some(Layer {
                name: name.clone(),
                color,
                hidden: p.layer_hidden.get(name).copied().unwrap_or(false),
                attribute_dictionaries: p
                    .layer_attribute_dictionaries
                    .get(name)
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .collect();

    let pages = p
        .pages
        .iter()
        .map(|pg| Page {
            name: pg.name.clone(),
            eye: pg.eye,
            target: pg.target,
            up: pg.up,
            fov: pg.fov,
            parallel: pg.parallel,
            ortho_height: pg.ortho_height,
            hidden_layers: pg
                .hidden_layer_ids
                .iter()
                .filter_map(|id| p.layer_id_to_name.get(id).cloned())
                .collect(),
            selected: pg.selected,
        })
        .collect();

    let dimensions = p
        .dimensions
        .iter()
        .map(|d| Dimension {
            text: d.text.clone(),
            hidden: d.hidden,
            a: Some(d.a),
            b: Some(d.b),
            offset: d.offset,
            plane_x: d.plane_x,
            normal: d.normal,
            offset_direction: d.offset_direction,
        })
        .collect();

    // Materials in name order (OpenSKP iterates its name-keyed map).
    let mut store_to_index = BTreeMap::new();
    let mut materials: Vec<Material> = Vec::new();
    for &store in p.materials.values() {
        let r = &mut p.material_store[store];
        store_to_index.insert(store, materials.len());
        materials.push(Material {
            name: r.name.clone(),
            color: [r.r as u8, r.g as u8, r.b as u8, r.a as u8],
            transparency: r.transparency,
            id: None,
            texture: r.texture.take().map(|t| Texture {
                filename: t.filename,
                width: t.x_scale,
                height: t.y_scale,
                data: t.data,
            }),
            colorized: r.colorized,
            colorize_type: r.colorize_type,
        });
    }
    let mut material_indices = BTreeMap::new();
    for (&id, name) in &p.material_id_to_name {
        let store = p
            .materials
            .get(name)
            .or_else(|| p.materials_by_folder.get(name));
        if let Some(i) = store.and_then(|s| store_to_index.get(s)).copied() {
            materials[i].id.get_or_insert(id);
            material_indices.insert(id, i);
        }
    }

    let styles = p
        .styles
        .into_iter()
        .map(|s| Style {
            name: s.name,
            description: s.description,
            front_color: s.front_color,
            back_color: s.back_color,
            items: s.items,
            watermarks: s.watermarks,
            folder: s.folder,
            active: s.active,
            working_copy: s.working_copy,
            modified: s.modified,
        })
        .collect();

    Model {
        version: p.version,
        units: p.units,
        definitions,
        root,
        layers,
        pages,
        camera: p.camera,
        dimensions,
        materials,
        styles,
        attribute_dictionaries: p.model_attribute_dicts,
        named_options: p.named_options,
        material_indices,
    }
}
