//! Camera and wgpu viewport renderer.

pub mod camera;
pub mod mesh;
pub mod viewport;

pub use camera::{Camera, Ray};
pub use mesh::FaceVertex;
pub use viewport::{Light, Segment, ShadingStyle, Style, Viewport};
