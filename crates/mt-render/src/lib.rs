//! Camera and wgpu viewport renderer.

pub mod camera;
pub mod mesh;
pub mod section;
pub mod sun;
pub mod viewport;

pub use camera::{Camera, Ray};
pub use mesh::FaceVertex;
pub use section::{SectionCut, section_lines};
pub use sun::sun_direction;
pub use viewport::{
    FaceStyle, Light, Segment, ShadingStyle, Shadows, Style, Viewport, XRAY_OPACITY,
};
