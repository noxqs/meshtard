//! Material textures and per-face texture positioning.

use std::sync::Arc;

use glam::{DVec2, DVec3};
use mt_skp::uv::UvMapping;

use crate::entities::Face;
use crate::model::Material;

/// A material's texture image.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    /// File name as stored (SketchUp keeps the source path).
    pub filename: String,
    /// Image file bytes as stored (PNG, JPEG or BMP), shared by model copies.
    pub data: Arc<[u8]>,
    /// Size of one tile in inches (0 when the file has none; treated as 1).
    pub width: f64,
    pub height: f64,
    /// SketchUp colorizes the image towards the material color. The material color
    /// is the image's average color unless the user recolored it; files often set
    /// this flag either way.
    pub colorize: bool,
}

/// Texture positioning of a face's sides (SketchUp's pinned/positioned textures): the
/// 3x3 row-major texture-to-plane matrix as stored in the file, `None` for the default
/// projection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaceUv {
    pub front: Option<[f64; 9]>,
    pub back: Option<[f64; 9]>,
}

impl FaceUv {
    /// Boxed when at least one side is positioned, else `None` (the cheap default).
    pub fn boxed(front: Option<[f64; 9]>, back: Option<[f64; 9]>) -> Option<Box<Self>> {
        (front.is_some() || back.is_some()).then(|| Box::new(Self { front, back }))
    }
}

/// The texture mapping of one side of `face` painted with `material`, or `None` when
/// the material has no texture. Evaluate it at the face's vertices in the face's own
/// (definition) coordinates; [`uv_at`] does that.
pub fn face_mapping(face: &Face, back: bool, material: &Material) -> Option<UvMapping> {
    let t = material.texture.as_ref()?;
    let uv = face.attrs.uv.as_deref();
    let transform = uv.and_then(|u| {
        if back {
            u.back.as_ref()
        } else {
            u.front.as_ref()
        }
    });
    Some(UvMapping::new(
        face.normal.to_array(),
        transform,
        [t.width, t.height],
    ))
}

/// UV (in texture repeats) of point `p` under `mapping`.
pub fn uv_at(mapping: &UvMapping, p: DVec3) -> DVec2 {
    DVec2::from_array(mapping.uv(p.to_array()))
}
