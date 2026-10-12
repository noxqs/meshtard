//! Model data behind the tray panels and Model Info pages of FEATURES.md 60 and 61:
//! tag folders and dashes, In Model styles, scene properties, component editing
//! fades, dimension and text styles, geo-location.
//!
//! None of it is read from or written to `.skp` files yet (`mt-skp` does not expose
//! these records): it lives in the model while it is open and is part of undo.
//! Defaults are drafts of SketchUp's (unverified).

use crate::entities::{EdgeId, FaceId};
use crate::model::{Model, ShadowSettings, StyleSettings};

/// A tag's dash pattern (Tags panel, Dashes column). The renderer draws plain dashes
/// of [`Dashes::length`]; SketchUp's patterns and scale are unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dashes {
    #[default]
    Solid,
    Dash,
    ShortDash,
    LongDash,
    Dot,
    DashDot,
}

impl Dashes {
    pub const ALL: [Self; 6] = [
        Self::Solid,
        Self::Dash,
        Self::ShortDash,
        Self::LongDash,
        Self::Dot,
        Self::DashDot,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Default",
            Self::Dash => "Dash",
            Self::ShortDash => "Short Dash",
            Self::LongDash => "Long Dash",
            Self::Dot => "Dot",
            Self::DashDot => "Dash Dot",
        }
    }

    /// Dash length in logical points for the renderer's segment style (0 = solid).
    /// Dash Dot is drawn as an even dash (the renderer has one dash length).
    pub fn length(self) -> f32 {
        match self {
            Self::Solid => 0.0,
            Self::Dash => 6.0,
            Self::ShortDash => 3.0,
            Self::LongDash => 12.0,
            Self::Dot => 1.5,
            Self::DashDot => 5.0,
        }
    }
}

/// A tag folder (Tags panel). Folders nest; a hidden folder hides every tag in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFolder {
    pub name: String,
    /// Index of the folder it is in (`None`: top level).
    pub parent: Option<usize>,
    pub visible: bool,
}

/// An "In Model" style of the Styles panel: a name and the settings it applies.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedStyle {
    pub name: String,
    pub description: String,
    pub settings: StyleSettings,
}

/// What a scene saves and restores (Scenes panel, "Properties to save").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneProps {
    pub camera: bool,
    pub hidden_geometry: bool,
    pub visible_tags: bool,
    pub section_planes: bool,
    pub style_fog: bool,
    pub shadows: bool,
    pub axes: bool,
}

impl Default for SceneProps {
    fn default() -> Self {
        Self {
            camera: true,
            hidden_geometry: true,
            visible_tags: true,
            section_planes: true,
            style_fog: true,
            shadows: true,
            axes: true,
        }
    }
}

impl SceneProps {
    /// Names and fields in the panel's order.
    pub fn fields_mut(&mut self) -> [(&'static str, &mut bool); 7] {
        [
            ("Camera Location", &mut self.camera),
            ("Hidden Geometry", &mut self.hidden_geometry),
            ("Visible Tags", &mut self.visible_tags),
            ("Active Section Planes", &mut self.section_planes),
            ("Style and Fog", &mut self.style_fog),
            ("Shadow Settings", &mut self.shadows),
            ("Axes Location", &mut self.axes),
        ]
    }
}

/// The model state a scene saved besides its camera and hidden tags: applied on
/// activation for the properties it saves.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneState {
    /// Style and fog (and the Axes toggle, applied with "Axes Location").
    pub style: StyleSettings,
    /// The In Model style the scene uses, if any.
    pub style_index: Option<usize>,
    pub shadows: ShadowSettings,
    /// Root entities hidden when saved (the root's other entities are shown).
    pub hidden_faces: Vec<FaceId>,
    pub hidden_edges: Vec<EdgeId>,
    pub hidden_instances: Vec<usize>,
    /// The root's active section plane.
    pub active_section: Option<usize>,
    /// The drawing axes (Axes Location).
    pub axes: glam::DMat4,
}

/// Model Info > Components: how the rest of the model shows while a group or
/// component is open. Fades are 0 (not faded) to 1 (white); the hide flags are
/// View > Component Edit's (`StyleSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditFade {
    /// Other instances of the open component.
    pub similar: f32,
    /// Everything else outside the open context.
    pub rest: f32,
    /// Show component axes: each component instance's axes at its origin.
    pub show_axes: bool,
}

impl Default for EditFade {
    fn default() -> Self {
        Self {
            similar: 0.55,
            rest: 0.55,
            show_axes: false,
        }
    }
}

/// How a dimension line or leader ends (Model Info > Dimensions and Text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Endpoint {
    None,
    Slash,
    Dot,
    #[default]
    ClosedArrow,
    OpenArrow,
}

impl Endpoint {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Slash,
        Self::Dot,
        Self::ClosedArrow,
        Self::OpenArrow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Slash => "Slash",
            Self::Dot => "Dot",
            Self::ClosedArrow => "Closed Arrow",
            Self::OpenArrow => "Open Arrow",
        }
    }
}

/// Where a dimension's text sits (Model Info > Dimensions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DimensionAlign {
    /// Facing the screen, centered on the dimension line.
    #[default]
    Screen,
    /// Along the dimension line, above it.
    Above,
    /// Along the dimension line, centered on it.
    Centered,
    /// Along the dimension line, past its end.
    Outside,
}

impl DimensionAlign {
    pub const ALL: [Self; 4] = [Self::Screen, Self::Above, Self::Centered, Self::Outside];

    pub fn label(self) -> &'static str {
        match self {
            Self::Screen => "Align to screen",
            Self::Above => "Align to dimension line: Above",
            Self::Centered => "Align to dimension line: Centered",
            Self::Outside => "Align to dimension line: Outside",
        }
    }
}

/// A dimension's text and end style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DimensionStyle {
    /// Points.
    pub font_size: f32,
    pub align: DimensionAlign,
    pub endpoint: Endpoint,
}

impl Default for DimensionStyle {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            align: DimensionAlign::Screen,
            endpoint: Endpoint::ClosedArrow,
        }
    }
}

/// A leader text's leader (Model Info > Text, Leader Lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeaderKind {
    /// Drawn in the model (the label follows the model, "View Based" in SketchUp is
    /// the label staying screen-aligned; both are drawn the same here).
    #[default]
    ViewBased,
    Pushpin,
    Hidden,
}

impl LeaderKind {
    pub const ALL: [Self; 3] = [Self::ViewBased, Self::Pushpin, Self::Hidden];

    pub fn label(self) -> &'static str {
        match self {
            Self::ViewBased => "View Based",
            Self::Pushpin => "Pushpin",
            Self::Hidden => "Hidden",
        }
    }
}

/// A text's font size, color and leader style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Points.
    pub font_size: f32,
    /// sRGB.
    pub color: [u8; 3],
    pub leader: LeaderKind,
    pub endpoint: Endpoint,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            color: [0, 0, 0],
            leader: LeaderKind::ViewBased,
            endpoint: Endpoint::Dot,
        }
    }
}

/// Model Info > Geo-location: whether a location was set, and the angle of north
/// from the green axis (degrees, clockwise seen from above). Latitude and longitude
/// are the Shadows settings'.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GeoLocation {
    pub located: bool,
    pub north_angle: f64,
}

impl Model {
    /// Whether folder `i` and every folder above it are visible.
    pub fn folder_shown(&self, i: usize) -> bool {
        let mut at = Some(i);
        // Bounded walk: a broken parent chain cannot loop forever.
        for _ in 0..=self.tag_folders.len() {
            let Some(k) = at else { return true };
            let Some(f) = self.tag_folders.get(k) else {
                return true;
            };
            if !f.visible {
                return false;
            }
            at = f.parent;
        }
        true
    }

    /// Whether tag `i` shows: the tag and the folders it is in are visible. Unknown
    /// tags show.
    pub fn layer_shown(&self, i: usize) -> bool {
        self.layers
            .get(i)
            .is_none_or(|l| l.visible && l.folder.is_none_or(|f| self.folder_shown(f)))
    }

    /// Removes tag folder `i`: its tags and folders move to its parent.
    pub fn remove_tag_folder(&mut self, i: usize) -> bool {
        let Some(parent) = self.tag_folders.get(i).map(|f| f.parent) else {
            return false;
        };
        self.tag_folders.remove(i);
        let fix = |x: Option<usize>| match x {
            Some(k) if k == i => parent.map(|p| if p > i { p - 1 } else { p }),
            Some(k) if k > i => Some(k - 1),
            other => other,
        };
        for f in &mut self.tag_folders {
            f.parent = fix(f.parent);
        }
        for l in &mut self.layers {
            l.folder = fix(l.folder);
        }
        true
    }

    /// Whether folder `a` is `b` or inside it (moving `b` into `a` would loop).
    pub fn folder_within(&self, a: usize, b: usize) -> bool {
        let mut at = Some(a);
        for _ in 0..=self.tag_folders.len() {
            match at {
                Some(k) if k == b => return true,
                Some(k) => at = self.tag_folders.get(k).and_then(|f| f.parent),
                None => return false,
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Layer;

    #[test]
    fn folders_hide_their_tags_and_nested_folders() {
        let mut m = Model {
            tag_folders: vec![
                TagFolder {
                    name: "Outer".into(),
                    parent: None,
                    visible: true,
                },
                TagFolder {
                    name: "Inner".into(),
                    parent: Some(0),
                    visible: true,
                },
            ],
            ..Model::default()
        };
        m.layers.push(Layer {
            name: "A".into(),
            folder: Some(1),
            ..Layer::default()
        });
        assert!(m.layer_shown(1));
        m.tag_folders[0].visible = false;
        assert!(!m.layer_shown(1), "a hidden outer folder hides the tag");
        assert!(m.layer_shown(0) && m.layer_shown(99));
        assert!(m.folder_within(1, 0) && !m.folder_within(0, 1));
        // Removing the inner folder moves its tag to the outer one.
        assert!(m.remove_tag_folder(1));
        assert_eq!(m.layers[1].folder, Some(0));
        assert!(m.remove_tag_folder(0));
        assert_eq!(m.layers[1].folder, None);
        assert!(m.layer_shown(1));
    }
}

/// Environments panel (FEATURES.md 66): the model's environment and how it is used.
/// Model-only like the rest of this module (not in .skp files); the images live in
/// the app's environment library, `source` names one.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentSettings {
    /// Library key of the environment (a bundled name or a file path); `None` = none.
    pub source: Option<String>,
    /// Use as Skydome.
    pub skydome: bool,
    /// Use for Reflections.
    pub reflections: bool,
    /// Rotation about the blue axis, degrees.
    pub rotation_deg: f64,
    /// Exposure multipliers (1 = as recorded).
    pub skydome_exposure: f64,
    pub reflection_exposure: f64,
    /// Set Sun Location: the sun's panorama point (u, v in 0..1); when set it
    /// replaces the Shadows panel's sun.
    pub sun_uv: Option<[f64; 2]>,
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            source: None,
            skydome: true,
            reflections: true,
            rotation_deg: 0.0,
            skydome_exposure: 1.0,
            reflection_exposure: 1.0,
            sun_uv: None,
        }
    }
}

/// Model Info > Rendering. Model-only (not in .skp files yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderingOptions {
    /// Use Anti-Aliased Textures: textures sampled smoothly with mipmaps (on) or as
    /// nearest texels (off). On by default (unverified).
    pub antialiased_textures: bool,
}

impl Default for RenderingOptions {
    fn default() -> Self {
        Self {
            antialiased_textures: true,
        }
    }
}
