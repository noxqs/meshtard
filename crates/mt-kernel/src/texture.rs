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

/// One side's stored `uv_transform` after moving its face by `xf`: the texture stays on
/// the same points of the face. `old_normal`/`new_normal`: the face's unit normal
/// before and after; `on_face`: a point of the face before the move.
pub fn transform_uv(
    m: &[f64; 9],
    old_normal: DVec3,
    new_normal: DVec3,
    on_face: DVec3,
    xf: glam::DMat4,
) -> [f64; 9] {
    let basis = |n: DVec3| {
        let (x, y) = mt_skp::uv::face_uv_basis(n.to_array());
        (DVec3::from_array(x), DVec3::from_array(y))
    };
    let (xr, yr) = basis(old_normal);
    let (nxr, nyr) = basis(new_normal);
    // Texture point (u, v) to plane coordinates: [px py q] = [u v 1] * m.
    let plane = |u: f64, v: f64| {
        let q = u * m[2] + v * m[5] + m[8];
        let q = if q.abs() < 1e-12 { 1.0 } else { q };
        DVec2::new(u * m[0] + v * m[3] + m[6], u * m[1] + v * m[4] + m[7]) / q
    };
    let lift = |p: DVec2| on_face + xr * (p.x - on_face.dot(xr)) + yr * (p.y - on_face.dot(yr));
    let moved = |u, v| {
        let p = xf.transform_point3(lift(plane(u, v)));
        DVec2::new(p.dot(nxr), p.dot(nyr))
    };
    let (o, a, b) = (moved(0.0, 0.0), moved(1.0, 0.0), moved(0.0, 1.0));
    let (a, b) = (a - o, b - o);
    [a.x, a.y, 0.0, b.x, b.y, 0.0, o.x, o.y, 1.0]
}

/// The texture-to-plane matrix that puts one tile of a `tile`-sized texture on the
/// rectangle `origin + s * u + t * v` (`s`, `t` in 0..1) of a face with unit normal
/// `normal` (an image entity's face, a texture fit to a face).
pub fn fit_uv(normal: DVec3, origin: DVec3, u: DVec3, v: DVec3, tile: [f64; 2]) -> [f64; 9] {
    let (xr, yr) = mt_skp::uv::face_uv_basis(normal.to_array());
    let (xr, yr) = (DVec3::from_array(xr), DVec3::from_array(yr));
    let size = |s: f64| if s > 1e-9 { s } else { 1.0 };
    let (tw, th) = (size(tile[0]), size(tile[1]));
    [
        u.dot(xr) / tw,
        u.dot(yr) / tw,
        0.0,
        v.dot(xr) / th,
        v.dot(yr) / th,
        0.0,
        origin.dot(xr),
        origin.dot(yr),
        1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DMat4;

    #[test]
    fn fit_and_move_keep_texture_on_the_rectangle() {
        let tile = [40.0, 20.0];
        let n = DVec3::Z;
        let m = fit_uv(n, DVec3::ZERO, DVec3::X * 40.0, DVec3::Y * 20.0, tile);
        let map = UvMapping::new(n.to_array(), Some(&m), tile);
        assert!(uv_at(&map, DVec3::new(40.0, 20.0, 0.0)).abs_diff_eq(DVec2::ONE, 1e-12));
        // Stand the rectangle up on a wall and move it.
        let xf = DMat4::from_translation(DVec3::new(5.0, 7.0, 3.0))
            * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
        let n2 = xf.transform_vector3(n).normalize();
        let m2 = transform_uv(&m, n, n2, DVec3::ZERO, xf);
        let map2 = UvMapping::new(n2.to_array(), Some(&m2), tile);
        for (p, uv) in [
            (DVec3::ZERO, DVec2::ZERO),
            (DVec3::new(40.0, 0.0, 0.0), DVec2::X),
            (DVec3::new(40.0, 20.0, 0.0), DVec2::ONE),
        ] {
            let got = uv_at(&map2, xf.transform_point3(p));
            assert!(got.abs_diff_eq(uv, 1e-9), "{p} -> {got}");
        }
    }
}
