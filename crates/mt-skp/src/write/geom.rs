//! Geometry helpers for the writer: plane fitting, circle/arc tessellation,
//! rotations and texture-matrix fitting. Port of the module-level helpers in
//! OpenSKP `create.py` (plus `face_uv_basis` from `_face_groups.py`).
//!
//! Every formula is evaluated in the same order as the Python original so the
//! written doubles are bit-identical. Two Python details matter for that:
//! `x ** 0.5` is libm `pow`, not `sqrt` (see [`pow_half`]), and `sum()` over
//! floats is compensated since Python 3.12 (see [`py_sum`]).

use super::{Point3, WriteError};

/// Below this length `Z x n` is treated as vertical (`VERTICAL_TOLERANCE` in
/// `_face_groups.py`, measured against SketchUp).
const VERTICAL_TOLERANCE: f64 = 1e-3;

fn invalid(msg: impl Into<String>) -> WriteError {
    WriteError::Invalid(msg.into())
}

/// Python's `x ** 0.5`: a libm `pow` call. The exponent goes through
/// `black_box` so LLVM cannot rewrite the call into `sqrt`, which may round
/// differently in the last bit.
pub(super) fn pow_half(x: f64) -> f64 {
    x.powf(std::hint::black_box(0.5))
}

/// Python's builtin `sum()` over floats (CPython 3.12+: Neumaier compensated
/// summation, compensation added once at the end when finite and nonzero).
pub(super) fn py_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut f = 0.0f64;
    let mut c = 0.0f64;
    for x in values {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

pub(super) fn cross(a: Point3, b: Point3) -> Point3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn normalize3(v: Point3) -> Result<Point3, WriteError> {
    let length = pow_half(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    if length < 1e-9 {
        return Err(invalid(
            "cannot determine a texture-positioning basis: the face's first edge is degenerate",
        ));
    }
    Ok([v[0] / length, v[1] / length, v[2] / length])
}

/// Row-major 3x3 rotation by `angle` radians (right-hand rule) about `axis`
/// (any nonzero length): Rodrigues' formula. The same convention as
/// [`Placement::matrix`](super::Placement::matrix).
pub fn rotation_matrix(axis: Point3, angle: f64) -> Result<[f64; 9], WriteError> {
    let length = pow_half(axis[0].powi(2) + axis[1].powi(2) + axis[2].powi(2));
    if length < 1e-9 {
        return Err(invalid("rotation axis must not be the zero vector"));
    }
    let (x, y, z) = (axis[0] / length, axis[1] / length, axis[2] / length);
    let c = angle.cos();
    let s = angle.sin();
    let t = 1.0 - c;
    Ok([
        t * x * x + c,
        t * x * y - s * z,
        t * x * z + s * y,
        t * x * y + s * z,
        t * y * y + c,
        t * y * z - s * x,
        t * x * z - s * y,
        t * y * z + s * x,
        t * z * z + c,
    ])
}

/// An orthonormal in-plane basis `(u, w)` for a circle/arc with unit
/// `normal`: seed with whichever of +Z/+X is less parallel, Gram-Schmidt.
pub(super) fn circle_basis(normal: Point3) -> Result<(Point3, Point3), WriteError> {
    let seed = if normal[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let dot = seed[0] * normal[0] + seed[1] * normal[1] + seed[2] * normal[2];
    let u = normalize3([
        seed[0] - dot * normal[0],
        seed[1] - dot * normal[1],
        seed[2] - dot * normal[2],
    ])?;
    let w = normalize3(cross(normal, u))?;
    Ok((u, w))
}

fn on_circle(center: Point3, radius: f64, u: Point3, w: Point3, angle: f64) -> Point3 {
    let (c, s) = (angle.cos(), angle.sin());
    [
        center[0] + radius * (c * u[0] + s * w[0]),
        center[1] + radius * (c * u[1] + s * w[1]),
        center[2] + radius * (c * u[2] + s * w[2]),
    ]
}

/// The `n` vertices of a full circle, counter-clockwise about the normal,
/// starting at `center + radius * u`.
pub(super) fn circle_points(
    center: Point3,
    radius: f64,
    n: u32,
    u: Point3,
    w: Point3,
) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let angle = 2.0 * std::f64::consts::PI * f64::from(i) / f64::from(n);
            on_circle(center, radius, u, w, angle)
        })
        .collect()
}

/// The `n + 1` points (both ends) of an arc from `start` to `end` radians,
/// measured from `u` toward `w`.
#[allow(clippy::too_many_arguments)]
pub(super) fn arc_points(
    center: Point3,
    radius: f64,
    n: u32,
    u: Point3,
    w: Point3,
    start: f64,
    end: f64,
) -> Vec<Point3> {
    (0..=n)
        .map(|i| {
            let angle = start + (end - start) * f64::from(i) / f64::from(n);
            on_circle(center, radius, u, w, angle)
        })
        .collect()
}

/// Newell's-method normal plus a centroid-based plane offset; `None` when the
/// points are collinear or degenerate. Also returns the coplanarity tolerance.
fn fit_plane(points: &[Point3]) -> Option<([f64; 4], f64)> {
    let n = points.len();
    let (mut nx, mut ny, mut nz) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let [x0, y0, z0] = points[i];
        let [x1, y1, z1] = points[(i + 1) % n];
        nx += (y0 - y1) * (z0 + z1);
        ny += (z0 - z1) * (x0 + x1);
        nz += (x0 - x1) * (y0 + y1);
    }
    let length = pow_half(nx * nx + ny * ny + nz * nz);
    if length.is_nan() || length < 1e-9 {
        return None;
    }
    let (nx, ny, nz) = (nx / length, ny / length, nz / length);
    let nf = n as f64;
    let cx = py_sum(points.iter().map(|p| p[0])) / nf;
    let cy = py_sum(points.iter().map(|p| p[1])) / nf;
    let cz = py_sum(points.iter().map(|p| p[2])) / nf;
    let d = nx * cx + ny * cy + nz * cz;
    Some(([nx, ny, nz, d], tolerance(points)))
}

/// Coplanarity tolerance: `max(span, 1) * 1e-6` where span is the largest
/// bounding-box extent.
pub(super) fn tolerance(points: &[Point3]) -> f64 {
    let span = (0..3)
        .map(|i| {
            let max = points
                .iter()
                .map(|p| p[i])
                .fold(f64::NEG_INFINITY, f64::max);
            let min = points.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min);
            max - min
        })
        .fold(f64::NEG_INFINITY, f64::max);
    span.max(1.0) * 1e-6
}

pub(super) fn plane_distance(plane: [f64; 4], p: Point3) -> f64 {
    plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] - plane[3]
}

fn degenerate() -> WriteError {
    invalid("face points are collinear or degenerate; cannot compute a plane")
}

/// The face plane `(nx, ny, nz, d)` with `n . p == d`; errors unless every
/// point lies on it.
/// `tol` replaces the default relative tolerance when given.
pub(super) fn plane_from_polygon(
    points: &[Point3],
    tol: Option<f64>,
) -> Result<[f64; 4], WriteError> {
    let (plane, default_tol) = fit_plane(points).ok_or_else(degenerate)?;
    let tol = tol.unwrap_or(default_tol);
    for &p in points {
        let dist = plane_distance(plane, p);
        if dist.abs() > tol {
            return Err(invalid(format!(
                "face points are not coplanar (point {p:?} is {:.6e} units off the fitted plane)",
                dist.abs()
            )));
        }
    }
    Ok(plane)
}

/// Whether the points are coplanar (same fit as [`plane_from_polygon`]);
/// still an error for degenerate input.
pub(super) fn is_coplanar(points: &[Point3], tol: Option<f64>) -> Result<bool, WriteError> {
    let (plane, default_tol) = fit_plane(points).ok_or_else(degenerate)?;
    let tol = tol.unwrap_or(default_tol);
    Ok(points
        .iter()
        .all(|&p| plane_distance(plane, p).abs() <= tol))
}

fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Cramer's rule for `a x = b`.
fn solve3x3(a: &[[f64; 3]; 3], b: [f64; 3]) -> Result<[f64; 3], WriteError> {
    let d = det3(a);
    if d.abs() < 1e-9 {
        return Err(invalid(
            "the 3 texture-positioning points map to collinear (u, v) coordinates - \
             cannot determine a texture mapping from them",
        ));
    }
    let mut out = [0.0; 3];
    for (col, slot) in out.iter_mut().enumerate() {
        let mut ai = *a;
        for r in 0..3 {
            ai[r][col] = b[r];
        }
        *slot = det3(&ai) / d;
    }
    Ok(out)
}

/// SketchUp's texture basis for a face with unit normal `n`: `U = normalize(Z
/// x n)`, `W = n x U`, snapped to `(+-X, Y)` near vertical normals.
pub(super) fn face_uv_basis(n: Point3) -> (Point3, Point3) {
    let [nx, ny, nz] = n;
    let (cx, cy) = (-ny, nx);
    let clen = pow_half(cx * cx + cy * cy);
    if clen < VERTICAL_TOLERANCE {
        let xr = if nz >= 0.0 {
            [1.0, 0.0, 0.0]
        } else {
            [-1.0, 0.0, 0.0]
        };
        (xr, [0.0, 1.0, 0.0])
    } else {
        let xr = [cx / clen, cy / clen, 0.0];
        let yr = [
            ny * xr[2] - nz * xr[1],
            nz * xr[0] - nx * xr[2],
            nx * xr[1] - ny * xr[0],
        ];
        (xr, yr)
    }
}

/// Fits the 3x3 UV-to-world matrix SketchUp stores for a positioned texture
/// from 3 `(world point, (u, v))` pins, in the face's [`face_uv_basis`].
pub(super) fn uv_matrix_for_face(
    pins: &[super::UvPin; 3],
    normal: Point3,
) -> Result<[f64; 9], WriteError> {
    let (u_axis, w_axis) = face_uv_basis(normal);
    let a = pins.map(|p| [p.uv[0], p.uv[1], 1.0]);
    let dot = |pt: Point3, ax: Point3| pt[0] * ax[0] + pt[1] * ax[1] + pt[2] * ax[2];
    let bx = pins.map(|p| dot(p.point, u_axis));
    let by = pins.map(|p| dot(p.point, w_axis));
    let [a0, c0, e0] = solve3x3(&a, bx)?;
    let [b0, d0, f0] = solve3x3(&a, by)?;
    Ok([a0, b0, 0.0, c0, d0, 0.0, e0, f0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_sum_is_compensated() {
        // Plain left-to-right addition loses the 1.0 entirely.
        assert_eq!(py_sum([1e100, 1.0, -1e100]), 1.0);
        assert_eq!(py_sum([0.1, 0.2, 0.3]), 0.6);
        assert_eq!(py_sum([]), 0.0);
    }

    #[test]
    fn rotation_about_z() {
        let m = rotation_matrix([0.0, 0.0, 2.0], std::f64::consts::FRAC_PI_2).unwrap();
        assert!((m[1] + 1.0).abs() < 1e-12 && (m[3] - 1.0).abs() < 1e-12);
        assert!(rotation_matrix([0.0; 3], 1.0).is_err());
    }

    #[test]
    fn plane_rejects_warped_quads() {
        let flat = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        assert_eq!(
            plane_from_polygon(&flat, None).unwrap(),
            [0.0, 0.0, 1.0, 0.0]
        );
        let mut warped = flat;
        warped[2][2] = 0.5;
        assert!(plane_from_polygon(&warped, None).is_err());
        assert!(!is_coplanar(&warped, None).unwrap());
        assert!(plane_from_polygon(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], None).is_err());
    }
}
