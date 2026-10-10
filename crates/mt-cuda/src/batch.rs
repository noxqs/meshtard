//! CPU versions of the per-face and per-triangle batches.

use glam::{DVec2, DVec3};
use mt_kernel::geom::Plane;

/// Each point projected into the plane of its face: `points[i] = (face, p)` gives
/// `Plane::new(normals[face], _).project(p)`, the 2D coordinates triangulation works in.
pub fn project_points(normals: &[DVec3], points: &[(u32, DVec3)]) -> Vec<DVec2> {
    let bases: Vec<(DVec3, DVec3)> = normals
        .iter()
        .map(|&n| Plane::new(n, DVec3::ZERO).basis())
        .collect();
    points
        .iter()
        .map(|&(f, p)| {
            let (u, v) = bases[f as usize];
            DVec2::new(p.dot(u), p.dot(v))
        })
        .collect()
}

/// Unit normal of each triangle (counter-clockwise front), zero for degenerate ones.
pub fn triangle_normals(triangles: &[[DVec3; 3]]) -> Vec<DVec3> {
    triangles
        .iter()
        .map(|&[a, b, c]| (b - a).cross(c - a).normalize_or_zero())
        .collect()
}
