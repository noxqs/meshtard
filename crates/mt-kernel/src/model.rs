//! A whole model: root geometry, component definitions, materials and tags.

use slotmap::SlotMap;

use glam::DVec3;

use crate::entities::{DefinitionId, Entities, LayerId};

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    /// sRGB color.
    pub color: [u8; 3],
    /// 1 = opaque.
    pub opacity: f64,
    /// `None` for a plain color. The color above is what Shaded shows (SketchUp stores
    /// the texture's average color there).
    pub texture: Option<crate::texture::Texture>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Definition {
    pub name: String,
    pub guid: String,
    pub entities: Entities,
    /// Components panel description.
    pub description: String,
    /// A group's definition (one instance, edited without affecting others, listed
    /// apart from components). Imported from the `Group#n` names SketchUp gives group
    /// definitions (unverified that every group follows this).
    pub is_group: bool,
}

/// A saved view: SketchUp's Scenes (pages in the Ruby API).
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub name: String,
    pub eye: DVec3,
    pub target: DVec3,
    pub fov_deg: f64,
    pub perspective: bool,
    /// Tags hidden in this scene.
    pub hidden_layers: Vec<LayerId>,
}

/// SketchUp's face styles (View > Face Style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FaceStyle {
    Wireframe,
    HiddenLine,
    #[default]
    Shaded,
    ShadedWithTextures,
    Monochrome,
}

/// The style settings the viewport uses. Defaults are drafts of SketchUp's default
/// style (unverified).
#[derive(Debug, Clone, PartialEq)]
pub struct StyleSettings {
    pub edges: bool,
    pub profiles: bool,
    pub profile_width: f32,
    pub section_cut_width: f32,
    pub face_style: FaceStyle,
    pub xray: bool,
    /// View > Section Planes: draw section plane objects.
    pub section_planes: bool,
    /// View > Section Cuts: the active section cuts the model.
    pub section_cuts: bool,
}

impl Default for StyleSettings {
    fn default() -> Self {
        Self {
            edges: true,
            profiles: true,
            profile_width: 2.0,
            section_cut_width: 3.0,
            face_style: FaceStyle::Shaded,
            xray: false,
            section_planes: true,
            section_cuts: true,
        }
    }
}

/// The Shadows panel. Defaults are drafts (SketchUp's default location is Boulder,
/// Colorado, unverified; time 13:30 on November 8 is from memory).
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowSettings {
    pub enabled: bool,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub utc_offset: f64,
    pub latitude: f64,
    pub longitude: f64,
    /// 0..1
    pub light: f64,
    /// 0..1
    pub dark: f64,
    pub on_faces: bool,
    pub on_ground: bool,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            month: 11,
            day: 8,
            hour: 13,
            minute: 30,
            utc_offset: -7.0,
            latitude: 40.017,
            longitude: -105.283,
            light: 0.8,
            dark: 0.45,
            on_faces: true,
            on_ground: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Model {
    pub root: Entities,
    pub definitions: SlotMap<DefinitionId, Definition>,
    pub materials: Vec<Material>,
    /// Index 0 is `Layer0` (untagged).
    pub layers: Vec<Layer>,
    /// Style colors for faces without a material.
    pub front_color: [u8; 3],
    pub back_color: [u8; 3],
    pub pages: Vec<Page>,
    pub style: StyleSettings,
    pub shadows: ShadowSettings,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            root: Entities::new(),
            definitions: SlotMap::with_key(),
            materials: Vec::new(),
            layers: vec![Layer {
                name: "Layer0".into(),
                color: [255, 255, 255],
                visible: true,
            }],
            // SketchUp's default style (values from OpenSKP's fixture checks).
            front_color: [255, 255, 255],
            back_color: [164, 178, 187],
            pages: Vec::new(),
            style: StyleSettings::default(),
            shadows: ShadowSettings::default(),
        }
    }
}

impl Model {
    /// A definition name not used yet: `base`, else `base#2`, `base#3`, ...
    pub fn unique_definition_name(&self, base: &str) -> String {
        let taken = |n: &str| self.definitions.values().any(|d| d.name == n);
        if !taken(base) {
            return base.to_owned();
        }
        let stem =
            base.rsplit_once('#').map_or(
                base,
                |(s, n)| if n.parse::<u32>().is_ok() { s } else { base },
            );
        (2..)
            .map(|i| format!("{stem}#{i}"))
            .find(|n| !taken(n))
            .expect("unbounded")
    }
}
