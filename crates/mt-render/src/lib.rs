//! Camera and wgpu viewport renderer.

pub mod camera;
pub mod effects;
pub mod environment;
pub mod mesh;
pub mod section;
pub mod sun;
pub mod texture;
pub mod viewport;

pub use camera::{Camera, Ray, StandardView, ease_in_out};
pub use effects::{AmbientOcclusion, Watermark, WatermarkPlacement};
pub use environment::{EnvImage, EnvView, ProceduralSky};
pub use mesh::FaceVertex;
pub use section::{SectionCut, section_fill, section_lines};
pub use sun::{rotate_north, sun_direction};
pub use texture::{TextureImage, TextureSource};
pub use viewport::{
    BACK_EDGE_DASH, FaceStyle, Fog, InstanceMesh, Light, Segment, ShadingStyle, Shadows, Style,
    Transparency, Viewport, XRAY_OPACITY, depth_cue_width, extend_segment, fog_amount, read_back,
};
