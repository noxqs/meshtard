//! Camera and wgpu viewport renderer.

pub mod camera;
pub mod mesh;
pub mod section;
pub mod sun;
pub mod texture;
pub mod viewport;

pub use camera::{Camera, Ray, StandardView, ease_in_out};
pub use mesh::FaceVertex;
pub use section::{SectionCut, section_fill, section_lines};
pub use sun::sun_direction;
pub use texture::{TextureImage, TextureSource};
pub use viewport::{
    BACK_EDGE_DASH, FaceStyle, Fog, Light, Segment, ShadingStyle, Shadows, Style, Viewport,
    XRAY_OPACITY, depth_cue_width, extend_segment, fog_amount,
};
