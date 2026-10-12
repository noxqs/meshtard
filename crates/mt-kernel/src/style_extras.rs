//! The Styles panel settings beyond the View menu toggles (FEATURES.md item 65): edge
//! color, endpoints and jitter; face transparency quality and ambient occlusion;
//! background, sky and ground; watermarks; modeling colors. Part of
//! [`crate::StyleSettings`].
//!
//! Saved in `.skp` files through the style items whose ids are documented (OpenSKP's
//! `docs/API_DESIGN.md`, confirmed there against SketchUp 2026) and present in the
//! writer's scaffold: see [`crate::export::style_items`]. Ambient occlusion color and
//! watermarks are model-only (their items are not in the scaffold). Defaults are drafts
//! of SketchUp's default style (unverified).

use std::sync::Arc;

/// Edge Settings > Color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeColorMode {
    /// Every edge in [`EdgeLook::color`].
    #[default]
    AllSame,
    /// Edges in the color of the material of a face they bound (Meshtard edges carry
    /// no material of their own; SketchUp uses the edge's material).
    ByMaterial,
    /// Edges parallel to the drawing axes in the axis colors, the others in
    /// [`EdgeLook::color`].
    ByAxis,
}

impl EdgeColorMode {
    pub const ALL: [Self; 3] = [Self::AllSame, Self::ByMaterial, Self::ByAxis];

    pub fn label(self) -> &'static str {
        match self {
            Self::AllSame => "All same",
            Self::ByMaterial => "By material",
            Self::ByAxis => "By axis",
        }
    }

    /// `EdgeColorMode` style item value (Ruby `RenderingOptions`: 0 object color,
    /// 1 foreground color, 2 direction color; from memory, unverified).
    pub fn item(self) -> i32 {
        match self {
            Self::ByMaterial => 0,
            Self::AllSame => 1,
            Self::ByAxis => 2,
        }
    }

    pub fn from_item(v: i64) -> Option<Self> {
        Some(match v {
            0 => Self::ByMaterial,
            1 => Self::AllSame,
            2 => Self::ByAxis,
            _ => return None,
        })
    }
}

/// Edge Settings beyond the View > Edge Style toggles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeLook {
    pub color_mode: EdgeColorMode,
    /// The "All same" color (sRGB).
    pub color: [u8; 3],
    /// Endpoints: edge ends drawn thicker over this many pixels.
    pub endpoints: bool,
    pub endpoint_length: f32,
    /// Jitter: edges drawn as two slightly offset strokes, like a sketch.
    pub jitter: bool,
}

impl Default for EdgeLook {
    fn default() -> Self {
        Self {
            color_mode: EdgeColorMode::AllSame,
            color: [0, 0, 0],
            endpoints: false,
            // SketchUp's default endpoint length (LineEndWidth) is unverified.
            endpoint_length: 7.0,
            jitter: false,
        }
    }
}

/// Face Settings > Transparency quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransparencyQuality {
    /// Translucent faces drawn in model order (overlaps may look wrong).
    Faster,
    /// Translucent faces sorted back to front every frame.
    #[default]
    Nicer,
}

/// Face Settings beyond the face style.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FaceLook {
    /// "Enable transparency" off draws translucent materials opaque. Stored
    /// inverted so the default (on) is `false`.
    pub transparency_off: bool,
    pub quality: TransparencyQuality,
}

/// Face Settings > Ambient Occlusion (SketchUp 2025): faces darken where they meet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbientOcclusion {
    pub enabled: bool,
    /// 0..1.
    pub intensity: f32,
    /// How far occluders reach, in inches.
    pub distance: f32,
    /// Tint of the occlusion (black when off). Model-only.
    pub use_color: bool,
    pub color: [u8; 3],
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        // Off; intensity and distance are guesses (unverified).
        Self {
            enabled: false,
            intensity: 0.5,
            distance: 12.0,
            use_color: false,
            color: [0, 0, 0],
        }
    }
}

/// Background Settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Background {
    pub color: [u8; 3],
    pub sky: bool,
    /// Sky color at the zenith.
    pub sky_color: [u8; 3],
    /// Sky color at the horizon (`HorizonColor`; not in SketchUp's panel).
    pub horizon_color: [u8; 3],
    /// How high the horizon color reaches into the sky: the sky color is
    /// `mix(horizon, sky, elevation^blend)` (0.5 by default; larger keeps more horizon).
    pub horizon_blend: f32,
    pub ground: bool,
    pub ground_color: [u8; 3],
    /// 0 opaque (hides what is below the ground) .. 1 invisible.
    pub ground_transparency: f32,
    /// Show ground from below.
    pub ground_from_below: bool,
}

impl Default for Background {
    fn default() -> Self {
        // `spec/default_model.toml`'s draft colors.
        Self {
            color: [255, 255, 255],
            sky: true,
            sky_color: [163, 199, 235],
            horizon_color: [237, 242, 247],
            horizon_blend: 0.5,
            ground: true,
            ground_color: [212, 209, 199],
            ground_transparency: 0.0,
            ground_from_below: false,
        }
    }
}

/// Where a watermark image goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WatermarkMode {
    /// Stretched over the viewport, optionally keeping its aspect ratio (fit inside).
    Stretched { lock_aspect: bool },
    /// Repeated over the viewport at `scale` times its pixel size.
    Tiled { scale: f32 },
    /// One copy at `scale` times its pixel size, at `anchor`: 0..1 across and down the
    /// viewport (0, 0 top left; SketchUp offers the nine corners, edges and center).
    Positioned { anchor: [f32; 2], scale: f32 },
}

/// A watermark: an image drawn over (overlay) or under (background) the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    pub name: String,
    /// PNG or JPEG bytes.
    pub image: Arc<[u8]>,
    pub overlay: bool,
    /// The panel's Blend: 0 invisible .. 1 opaque.
    pub opacity: f32,
    pub mode: WatermarkMode,
    /// Create Mask: the image's brightness becomes the opacity of the style's
    /// background color (white opaque, black clear; from memory of SketchUp's help,
    /// unverified).
    pub mask: bool,
}

impl Watermark {
    /// Moves watermark `i` of `list` (bottom first) one place up (drawn later, over
    /// the next one) or down, past the next watermark of its own kind (overlays and
    /// backgrounds are ordered among themselves). Returns the new index, `None` when it
    /// cannot move.
    pub fn reorder(list: &mut [Watermark], i: usize, up: bool) -> Option<usize> {
        let overlay = list.get(i)?.overlay;
        let j = if up {
            (i + 1..list.len()).find(|&j| list[j].overlay == overlay)?
        } else {
            (0..i).rev().find(|&j| list[j].overlay == overlay)?
        };
        list.swap(i, j);
        Some(j)
    }
}

/// Modeling Settings colors (sRGB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelingColors {
    pub selected: [u8; 3],
    pub locked: [u8; 3],
    pub guides: [u8; 3],
    pub section_active: [u8; 3],
    pub section_inactive: [u8; 3],
    pub section_cut: [u8; 3],
}

impl Default for ModelingColors {
    fn default() -> Self {
        // Drafts (SketchUp's exact colors unverified).
        Self {
            selected: [0, 0, 255],
            locked: [255, 0, 0],
            guides: [64, 64, 64],
            section_active: [242, 102, 13],
            section_inactive: [217, 140, 51],
            section_cut: [0, 0, 0],
        }
    }
}

/// Inference marker colors of Modeling Settings (the point inferences; the axis,
/// parallel and tangent colors are Preferences > Accessibility's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InferenceColors {
    pub endpoint: [u8; 3],
    pub midpoint: [u8; 3],
    pub on_edge: [u8; 3],
    pub on_face: [u8; 3],
}

impl Default for InferenceColors {
    fn default() -> Self {
        // The inference engine's colors (`mt_infer::Kind::color`).
        Self {
            endpoint: [0, 200, 0],
            midpoint: [0, 200, 220],
            on_edge: [230, 0, 0],
            on_face: [0, 0, 230],
        }
    }
}

/// Modeling Settings colors beyond [`ModelingColors`]: hidden geometry and the
/// inference markers. Model-only (their style items are not known); SketchUp's
/// panel and defaults for these are unverified.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelingExtras {
    /// Hidden edges and the mesh of hidden faces under View > Hidden Geometry.
    pub hidden_geometry: [u8; 3],
    pub inference: InferenceColors,
}

impl Default for ModelingExtras {
    fn default() -> Self {
        Self {
            hidden_geometry: [115, 115, 115],
            inference: InferenceColors::default(),
        }
    }
}

/// A style item color: signed int32 ABGR (R in the low byte), as SketchUp stores them.
pub fn color_item(c: [u8; 3]) -> i32 {
    i32::from_le_bytes([c[0], c[1], c[2], 0xff])
}

/// The color of a style item value written by [`color_item`].
pub fn item_color(v: i64) -> [u8; 3] {
    let [r, g, b, _] = (v as i32).to_le_bytes();
    [r, g, b]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wm(name: &str) -> Watermark {
        Watermark {
            name: name.into(),
            image: Arc::from(&b""[..]),
            overlay: false,
            opacity: 1.0,
            mode: WatermarkMode::Stretched { lock_aspect: true },
            mask: false,
        }
    }

    #[test]
    fn reorder_moves_one_place_and_stops_at_the_ends() {
        let mut l = vec![wm("a"), wm("b"), wm("c")];
        assert_eq!(Watermark::reorder(&mut l, 0, true), Some(1));
        assert_eq!(l[1].name, "a");
        assert_eq!(Watermark::reorder(&mut l, 2, true), None);
        assert_eq!(Watermark::reorder(&mut l, 0, false), None);
        assert_eq!(Watermark::reorder(&mut l, 5, false), None);
        assert_eq!(Watermark::reorder(&mut l, 2, false), Some(1));
        let names: Vec<_> = l.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["b", "c", "a"]);
        // An overlay between two backgrounds is skipped over.
        l[1].overlay = true;
        assert_eq!(Watermark::reorder(&mut l, 0, true), Some(2));
        let names: Vec<_> = l.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["a", "c", "b"]);
        assert_eq!(Watermark::reorder(&mut l, 1, true), None);
    }
}
