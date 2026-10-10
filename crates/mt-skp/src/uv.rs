//! Texture coordinates of points on a face. Port of OpenSKP's `face_uv_basis` and
//! `compute_face_uv` (Python `_face_groups.py`), the recipe documented on
//! `Face.uv_transform` in its `model.py`.
//!
//! UVs are in texture repeats: `(0, 0)` and `(1, 1)` are opposite corners of one tile,
//! `v` up (SketchUp's convention; image row 0 is at `v = 1`).

/// `|Z x n|` below which SketchUp projects with the world axes instead of the cross
/// product (measured by OpenSKP with the SDK).
pub const VERTICAL_TOLERANCE: f64 = 1e-3;

/// SketchUp's texture plane basis `(xr, yr)` for a face with unit normal `n`:
/// `xr = normalize(Z x n)`, `yr = n x xr`; near-vertical normals use `(X, Y)` looking up
/// and `(-X, Y)` looking down.
pub fn face_uv_basis(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let [nx, ny, nz] = n;
    let (cx, cy) = (-ny, nx);
    let clen = (cx * cx + cy * cy).sqrt();
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

/// Inverse of a row-major 3x3 matrix; identity when singular (as OpenSKP does).
fn invert_3x3(m: &[f64; 9]) -> [f64; 9] {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let k = 1.0 / det;
    [
        (e * i - f * h) * k,
        (c * h - b * i) * k,
        (b * f - c * e) * k,
        (f * g - d * i) * k,
        (a * i - c * g) * k,
        (c * d - a * f) * k,
        (d * h - e * g) * k,
        (b * g - a * h) * k,
        (a * e - b * d) * k,
    ]
}

/// The texture mapping of one side of a face, set up once per face and evaluated per
/// vertex with [`UvMapping::uv`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvMapping {
    xr: [f64; 3],
    yr: [f64; 3],
    /// Inverse of the face's `uv_transform`, `None` for the default projection.
    inverse: Option<[f64; 9]>,
    tile: [f64; 2],
}

impl UvMapping {
    /// `normal`: the face's unit normal; `transform`: the side's `uv_transform` (texture
    /// space to face plane, row-major, `None` when the texture was never positioned);
    /// `tile`: the material texture's tile width and height in inches (sizes of 0 count
    /// as 1, as OpenSKP does).
    ///
    /// Projected textures (`uv_projected`) are mapped the same way, which is
    /// approximate: the projection plane is not read.
    pub fn new(normal: [f64; 3], transform: Option<&[f64; 9]>, tile: [f64; 2]) -> Self {
        let (xr, yr) = face_uv_basis(normal);
        let size = |s: f64| if s > 1e-9 { s } else { 1.0 };
        Self {
            xr,
            yr,
            inverse: transform.map(invert_3x3),
            tile: [size(tile[0]), size(tile[1])],
        }
    }

    /// UV of point `p` (inches, in the face's own definition space), in repeats.
    pub fn uv(&self, p: [f64; 3]) -> [f64; 2] {
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let (px, py) = (dot(p, self.xr), dot(p, self.yr));
        let Some(inv) = self.inverse else {
            return [px / self.tile[0], py / self.tile[1]];
        };
        let u = px * inv[0] + py * inv[3] + inv[6];
        let v = px * inv[1] + py * inv[4] + inv[7];
        let mut q = px * inv[2] + py * inv[5] + inv[8];
        if q.abs() < 1e-12 {
            q = 1.0;
        }
        [(u / q) / self.tile[0], (v / q) / self.tile[1]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_snaps_near_vertical_normals() {
        assert_eq!(
            face_uv_basis([0.0, 0.0, 1.0]),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        );
        assert_eq!(
            face_uv_basis([1e-4, 0.0, -1.0]),
            ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        );
        // A wall facing -Y: U along +X, V up.
        let (xr, yr) = face_uv_basis([0.0, -1.0, 0.0]);
        assert_eq!(xr, [1.0, 0.0, 0.0]);
        assert_eq!(yr, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn default_projection_divides_by_tile_size() {
        let m = UvMapping::new([0.0, 0.0, 1.0], None, [10.0, 20.0]);
        assert_eq!(m.uv([5.0, 10.0, 3.0]), [0.5, 0.5]);
        let unsized_tile = UvMapping::new([0.0, 0.0, 1.0], None, [0.0, 0.0]);
        assert_eq!(unsized_tile.uv([2.0, 3.0, 0.0]), [2.0, 3.0]);
    }

    #[test]
    fn transform_is_inverted() {
        // Texture space to plane: scale 2 and shift by (4, 6).
        let t = [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 4.0, 6.0, 1.0];
        let m = UvMapping::new([0.0, 0.0, 1.0], Some(&t), [1.0, 1.0]);
        assert_eq!(m.uv([4.0, 6.0, 0.0]), [0.0, 0.0]);
        assert_eq!(m.uv([6.0, 10.0, 0.0]), [1.0, 2.0]);
    }
}
