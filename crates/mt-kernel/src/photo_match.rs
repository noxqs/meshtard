//! Camera > Match New Photo: the camera that took a photo, solved from what the user
//! lines up on it (SketchUp's Match Photo, help.sketchup.com "Matching a Photo to a
//! Model (or a Model to a Photo)").
//!
//! The user drags two red bars along two lines of the photo that run along the red
//! axis, two green bars along lines on the green axis, the axes origin onto a corner
//! and a point on the blue axis to set the scale. That is a standard two-vanishing-
//! point calibration:
//!
//! - each pair of bars meets at a vanishing point (the red and green axes' images at
//!   infinity); the yellow horizon is the line through both;
//! - the principal point is the center of the photo (an uncropped photo) and pixels
//!   are square, so the focal length follows from the two axes being perpendicular:
//!   `(v_red - c) . (v_green - c) + f^2 = 0`;
//! - the red and green axes in camera space point at their vanishing points (or away
//!   from them for an inside corner), blue is their cross product;
//! - the origin's pixel gives the direction to the world origin, and the blue scale
//!   point (a pixel of the point at `scale_height` on the blue axis) its distance.
//!
//! Photo pixels: x right, y down, origin at the top-left corner. Camera space: x
//! right, y down, z forward (into the photo). World: model axes, Z up, inches.
//!
//! The matched photo is kept with its scene in the model while it is open (part of
//! undo); `mt-skp` does not read or write SketchUp's matched photos, so it is not
//! saved in `.skp` files.

use std::sync::Arc;

use glam::{DMat3, DMat4, DVec2, DVec3};

/// The bars, origin and scale point lined up on a photo (all in photo pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoBars {
    /// Photo size in pixels.
    pub size: DVec2,
    /// The two red bars (lines along the red axis), each from one end to the other.
    pub red: [[DVec2; 2]; 2],
    /// The two green bars.
    pub green: [[DVec2; 2]; 2],
    /// Where the axes origin is on the photo.
    pub origin: DVec2,
    /// A point on the blue axis: the image of `(0, 0, scale_height)`.
    pub scale: DVec2,
    /// World height of `scale` above the origin, in inches (> 0).
    pub scale_height: f64,
    /// The axes run from the origin towards the camera (looking into an inside corner,
    /// grid style Inside) instead of away from it (an outside corner).
    pub inside: bool,
}

/// The grid styles of the Match Photo panel: what the photo shows. They set the
/// starting layout of the bars and whether the axes come towards the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridStyle {
    /// Inside a room, looking into a corner.
    Inside,
    /// An object seen from above (its corner below the horizon).
    #[default]
    Above,
    /// An object seen from below (its corner above the horizon).
    Below,
}

impl GridStyle {
    pub const ALL: [GridStyle; 3] = [GridStyle::Inside, GridStyle::Above, GridStyle::Below];

    pub fn label(self) -> &'static str {
        match self {
            GridStyle::Inside => "Inside",
            GridStyle::Above => "Above",
            GridStyle::Below => "Below",
        }
    }
}

/// A scene's matched photo: the image, how it is lined up and the Match Photo panel
/// settings.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchedPhoto {
    /// File name (the scene is named after it).
    pub filename: String,
    /// Image file bytes (PNG or JPEG), shared by model copies.
    pub data: Arc<[u8]>,
    pub bars: PhotoBars,
    pub style: GridStyle,
    /// Grid planes shown: red/green, red/blue, green/blue.
    pub planes: [bool; 3],
    /// Grid line spacing in inches.
    pub spacing: f64,
    pub show_grid: bool,
    /// The model is drawn (unchecked: only the photo, to line up the bars).
    pub show_model: bool,
    /// The photo is drawn behind the model (unchecked: in front of it).
    pub background: bool,
    /// Opacity of the photo, 0..1.
    pub opacity: f32,
}

impl MatchedPhoto {
    /// A photo of `size` pixels with the bars of `style` and default settings.
    pub fn new(filename: String, data: Arc<[u8]>, size: DVec2, style: GridStyle) -> Self {
        Self {
            filename,
            data,
            bars: default_bars(size, style),
            style,
            planes: [true, false, false],
            // Unverified: SketchUp's default spacing.
            spacing: 60.0,
            show_grid: true,
            show_model: true,
            background: true,
            opacity: 1.0,
        }
    }
}

/// Why the bars do not give a camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchError {
    /// A bar has (almost) no length.
    ShortBar,
    /// The bars of an axis are (nearly) parallel: no vanishing point on the photo.
    Parallel(Axis),
    /// The vanishing points are on the same side of the photo's center (or too close
    /// to it): the axes cannot be perpendicular.
    NotPerpendicular,
    /// The scale point is on the origin or makes the model's distance negative.
    Scale,
}

/// The red, green or blue axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Red,
    Green,
    Blue,
}

impl std::fmt::Display for MatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchError::ShortBar => write!(f, "A perspective bar is too short."),
            MatchError::Parallel(a) => write!(
                f,
                "The {} bars are parallel: they need to meet at a vanishing point.",
                match a {
                    Axis::Red => "red",
                    Axis::Green => "green",
                    Axis::Blue => "blue",
                }
            ),
            MatchError::NotPerpendicular => write!(
                f,
                "The red and green vanishing points must be on opposite sides of the photo."
            ),
            MatchError::Scale => write!(f, "Drag the scale point up the blue axis."),
        }
    }
}

impl std::error::Error for MatchError {}

/// A pinhole camera of a photo: `pixel = principal + focal * (x / z, y / z)` for the
/// camera-space point `rotation * world + translation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoCamera {
    /// World to camera rotation: its columns are the red, green and blue axes in
    /// camera space.
    pub rotation: DMat3,
    /// The world origin in camera space.
    pub translation: DVec3,
    /// Focal length in pixels.
    pub focal: f64,
    /// Principal point in pixels.
    pub principal: DVec2,
    /// Photo size in pixels.
    pub size: DVec2,
}

/// Vanishing points farther than this many photo diagonals from the center count as
/// parallel bars (the camera would be a narrow telephoto beyond any lens).
const MAX_VP_DIAGONALS: f64 = 200.0;
/// Shortest focal length allowed, in photo diagonals (a field of view under ~176
/// degrees across the diagonal).
const MIN_FOCAL_DIAGONALS: f64 = 0.02;

/// Solves the camera from the bars: see the module comment.
pub fn solve(b: &PhotoBars) -> Result<PhotoCamera, MatchError> {
    let c = b.size / 2.0;
    let diag = b.size.length().max(1.0);
    // Normalized photo coordinates: centered, in diagonals (better conditioned).
    let n = |p: DVec2| (p - c) / diag;
    let vanishing = |bars: &[[DVec2; 2]; 2], axis| -> Result<DVec2, MatchError> {
        let line = |[a, b]: [DVec2; 2]| {
            let (a, b) = (n(a), n(b));
            if (b - a).length() < 1e-6 {
                return Err(MatchError::ShortBar);
            }
            let l = a.extend(1.0).cross(b.extend(1.0));
            Ok(l / l.truncate().length())
        };
        let v = line(bars[0])?.cross(line(bars[1])?);
        if v.z.abs() * MAX_VP_DIAGONALS <= v.truncate().length() {
            return Err(MatchError::Parallel(axis));
        }
        Ok(v.truncate() / v.z)
    };
    let vx = vanishing(&b.red, Axis::Red)?;
    let vy = vanishing(&b.green, Axis::Green)?;
    let f2 = -vx.dot(vy);
    if f2 <= MIN_FOCAL_DIAGONALS * MIN_FOCAL_DIAGONALS {
        return Err(MatchError::NotPerpendicular);
    }
    let f = f2.sqrt();
    let sign = if b.inside { -1.0 } else { 1.0 };
    let x = vx.extend(f).normalize() * sign;
    let mut y = vy.extend(f).normalize() * sign;
    let mut z = x.cross(y);
    // Blue must point up in the photo (camera y is down); red and green placed the
    // other way round would make a left-handed set: green is turned around.
    if z.y > 0.0 {
        y = -y;
        z = -z;
    }
    let rotation = DMat3::from_cols(x, y, z);
    // Distance: (lambda * d_origin + h * z) points at the scale pixel.
    let d_origin = n(b.origin).extend(f);
    let d_scale = n(b.scale).extend(f);
    let a = d_origin.cross(d_scale);
    if a.length() <= 1e-9 * d_origin.length() * d_scale.length() || b.scale_height <= 0.0 {
        return Err(MatchError::Scale);
    }
    let lambda = -b.scale_height * a.dot(z.cross(d_scale)) / a.length_squared();
    if !(lambda.is_finite() && lambda > 0.0) {
        return Err(MatchError::Scale);
    }
    Ok(PhotoCamera {
        rotation,
        translation: d_origin * lambda,
        focal: f * diag,
        principal: c,
        size: b.size,
    })
}

impl PhotoCamera {
    /// A camera at `eye` looking at `target` with no roll (`roll_deg` turns it
    /// about the view direction),
    /// its principal point at the photo's center.
    pub fn look_at(eye: DVec3, target: DVec3, roll_deg: f64, focal: f64, size: DVec2) -> Self {
        let forward = (target - eye).normalize();
        let mut right = forward.cross(DVec3::Z);
        if right.length_squared() < 1e-12 {
            right = DVec3::X;
        }
        let right = right.normalize();
        let down = forward.cross(right);
        let (s, co) = roll_deg.to_radians().sin_cos();
        let (right, down) = (right * co + down * s, down * co - right * s);
        // Rows: right, down, forward.
        let rotation = DMat3::from_cols(right, down, forward).transpose();
        Self {
            rotation,
            translation: -(rotation * eye),
            focal,
            principal: size / 2.0,
            size,
        }
    }

    /// The camera position in the world.
    pub fn eye(&self) -> DVec3 {
        -(self.rotation.transpose() * self.translation)
    }

    /// World direction of the view (into the photo).
    pub fn forward(&self) -> DVec3 {
        self.rotation.row(2)
    }

    /// World direction of the photo's +x.
    pub fn right(&self) -> DVec3 {
        self.rotation.row(0)
    }

    /// World direction of the photo's +y (down the photo).
    pub fn down(&self) -> DVec3 {
        self.rotation.row(1)
    }

    /// Vertical field of view in degrees (photo height).
    pub fn fov_y_deg(&self) -> f64 {
        (2.0 * (self.size.y / 2.0 / self.focal).atan()).to_degrees()
    }

    pub fn to_camera(&self, p: DVec3) -> DVec3 {
        self.rotation * p + self.translation
    }

    /// Pixel of a world point; `None` behind the camera.
    pub fn project(&self, p: DVec3) -> Option<DVec2> {
        let q = self.to_camera(p);
        (q.z > 1e-9 * self.translation.length().max(1.0))
            .then(|| self.principal + self.focal * DVec2::new(q.x / q.z, q.y / q.z))
    }

    /// Pixels of the part of the segment `a`-`b` in front of the camera (clipped a
    /// little in front of the eye).
    pub fn project_segment(&self, a: DVec3, b: DVec3) -> Option<(DVec2, DVec2)> {
        let (mut qa, mut qb) = (self.to_camera(a), self.to_camera(b));
        let near = 1e-3 * self.translation.length().max(1e-6);
        if qa.z < near && qb.z < near {
            return None;
        }
        let cut = |p: DVec3, q: DVec3| p + (q - p) * ((near - p.z) / (q.z - p.z));
        if qa.z < near {
            qa = cut(qa, qb);
        } else if qb.z < near {
            qb = cut(qb, qa);
        }
        let px = |q: DVec3| self.principal + self.focal * DVec2::new(q.x / q.z, q.y / q.z);
        Some((px(qa), px(qb)))
    }

    /// Vanishing point of world direction `d`; `None` when it is at infinity.
    pub fn vanishing_point(&self, d: DVec3) -> Option<DVec2> {
        let q = self.rotation * d;
        (q.z.abs() > 1e-9 * q.length())
            .then(|| self.principal + self.focal * DVec2::new(q.x / q.z, q.y / q.z))
    }

    /// The horizon (image of the horizontal plane at infinity) as a point on it and
    /// its unit direction; `None` looking straight up or down.
    pub fn horizon(&self) -> Option<(DVec2, DVec2)> {
        let z = self.rotation * DVec3::Z;
        let nrm = DVec2::new(z.x, z.y);
        let len2 = nrm.length_squared();
        if len2 < 1e-12 {
            return None;
        }
        // Pixels p with z . ((p - c) / f, 1) = 0.
        let point = self.principal - nrm * (self.focal * z.z / len2);
        Some((point, nrm.perp().normalize()))
    }

    /// Ray through a pixel: eye and unit world direction.
    pub fn ray(&self, px: DVec2) -> (DVec3, DVec3) {
        let d = ((px - self.principal) / self.focal).extend(1.0);
        (self.eye(), (self.rotation.transpose() * d).normalize())
    }

    /// Distance from the eye to the world origin along the view direction.
    pub fn depth(&self) -> f64 {
        self.translation.z
    }

    /// The bars, origin and scale point a perfect match of this camera has for the
    /// box `0..len` (red) by `0..width` (green) by `0..height` (blue): the bars on its
    /// bottom and top edges along red and green (their far 70%, clear of the origin's
    /// grip), the scale point at `scale_height`.
    pub fn bars_for_box(
        &self,
        [len, width, height]: [f64; 3],
        scale_height: f64,
        inside: bool,
    ) -> Option<PhotoBars> {
        let p = |x, y, z| self.project(DVec3::new(x, y, z));
        let (l0, w0) = (len * 0.3, width * 0.3);
        Some(PhotoBars {
            size: self.size,
            red: [
                [p(l0, 0.0, 0.0)?, p(len, 0.0, 0.0)?],
                [p(l0, 0.0, height)?, p(len, 0.0, height)?],
            ],
            green: [
                [p(0.0, w0, 0.0)?, p(0.0, width, 0.0)?],
                [p(0.0, w0, height)?, p(0.0, width, height)?],
            ],
            origin: p(0.0, 0.0, 0.0)?,
            scale: p(0.0, 0.0, scale_height)?,
            scale_height,
            inside,
        })
    }

    /// Texture-to-plane matrix (as [`crate::FaceUv`] stores it, for a texture whose
    /// tile is 1 x 1) that projects this photo onto the plane of a face: the face's
    /// unit `normal` and a point `on_face`, both in the face's own coordinates, and
    /// `xf` its coordinates to world. Texture u runs along the photo's width, v up its
    /// height (0..1 over the photo). `None` when the plane passes through the eye.
    pub fn face_uv(&self, xf: DMat4, normal: DVec3, on_face: DVec3) -> Option<[f64; 9]> {
        let (xr, yr) = mt_skp::uv::face_uv_basis(normal.to_array());
        let (xr, yr) = (DVec3::from_array(xr), DVec3::from_array(yr));
        // Plane coordinates (px, py) to the face point: xr px + yr py + n d.
        let base = normal * normal.dot(on_face);
        let lin = glam::DMat3::from_mat4(xf);
        let cam = |v: DVec3| self.rotation * (lin * v);
        let c0 = cam(xr);
        let c1 = cam(yr);
        let c2 = self.to_camera(xf.transform_point3(base));
        // Camera space to texture (u, v, w): u = x_px / W, v = 1 - y_px / H.
        let k = DMat3::from_cols(
            DVec3::new(self.focal / self.size.x, 0.0, 0.0),
            DVec3::new(0.0, -self.focal / self.size.y, 0.0),
            DVec3::new(
                self.principal.x / self.size.x,
                1.0 - self.principal.y / self.size.y,
                1.0,
            ),
        );
        // Plane (px, py, 1) to texture (u w, v w, w), column-vector form.
        let g = k * DMat3::from_cols(c0, c1, c2);
        // Keep w near 1 on the face.
        let g = g * (1.0 / c2.z.abs().max(1e-12));
        if g.determinant().abs() < 1e-15 {
            return None;
        }
        // Stored row-vector form: [px py q] = [u v 1] * m, m = (g^-1)^T.
        let m = g.inverse().transpose();
        let c = m.to_cols_array();
        // Row-major array of the row-vector matrix: m[r][c] = (g^-1)[c][r].
        let out = [c[0], c[3], c[6], c[1], c[4], c[7], c[2], c[5], c[8]];
        out.iter().all(|x| x.is_finite()).then_some(out)
    }
}

/// The starting bars of a grid style on a photo of `size` pixels: those of a box
/// corner seen by a typical camera (focal length the photo's width), so they solve.
pub fn default_bars(size: DVec2, style: GridStyle) -> PhotoBars {
    let (eye, target, dims, inside) = match style {
        GridStyle::Above => (
            DVec3::new(-520.0, -400.0, 330.0),
            DVec3::new(120.0, 140.0, 40.0),
            [300.0, 300.0, 120.0],
            false,
        ),
        GridStyle::Below => (
            DVec3::new(-560.0, -440.0, 30.0),
            DVec3::new(120.0, 140.0, 190.0),
            [300.0, 300.0, 240.0],
            false,
        ),
        GridStyle::Inside => (
            DVec3::new(420.0, 380.0, 60.0),
            DVec3::new(0.0, 0.0, 70.0),
            [200.0, 200.0, 110.0],
            true,
        ),
    };
    let size = size.max(DVec2::ONE);
    let cam = PhotoCamera::look_at(eye, target, 0.0, size.x, size);
    cam.bars_for_box(dims, dims[2] * 0.75, inside)
        .unwrap_or(PhotoBars {
            size,
            red: [[DVec2::ZERO; 2]; 2],
            green: [[DVec2::ZERO; 2]; 2],
            origin: size / 2.0,
            scale: size / 2.0,
            scale_height: 1.0,
            inside,
        })
}

impl PhotoBars {
    /// Moves the origin to `to` keeping the model's distance along the view (the
    /// scale) of `cam` (the current solve): the scale point moves with it.
    pub fn move_origin(&mut self, to: DVec2, cam: &PhotoCamera) {
        self.origin = to;
        let d = ((to - cam.principal) / cam.focal).extend(1.0);
        let moved = PhotoCamera {
            translation: d * cam.depth(),
            ..*cam
        };
        if let Some(s) = moved.project(DVec3::Z * self.scale_height) {
            self.scale = s;
        }
    }

    /// Puts the scale point back on the blue axis of `cam` (after the bars changed the
    /// blue axis's direction).
    pub fn snap_scale(&mut self, cam: &PhotoCamera) {
        if let Some(s) = cam.project(DVec3::Z * self.scale_height) {
            self.scale = s;
        }
    }

    /// Every bar end, in the order red 0a, red 0b, red 1a, red 1b, green 0a, ...
    pub fn ends(&self) -> [DVec2; 8] {
        let [[a, b], [c, d]] = self.red;
        let [[e, f], [g, h]] = self.green;
        [a, b, c, d, e, f, g, h]
    }

    pub fn end_mut(&mut self, i: usize) -> &mut DVec2 {
        let bars = if i < 4 {
            &mut self.red
        } else {
            &mut self.green
        };
        &mut bars[(i / 2) % 2][i % 2]
    }

    /// Moves every bar by `by` (dragging the horizon).
    pub fn shift_bars(&mut self, by: DVec2) {
        for i in 0..8 {
            *self.end_mut(i) += by;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator (no test dependency needed).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn range(&mut self, a: f64, b: f64) -> f64 {
            a + (b - a) * self.next()
        }
    }

    fn assert_same(found: &PhotoCamera, truth: &PhotoCamera, tol: f64) {
        let scale = truth.eye().length();
        assert!(
            found.eye().distance(truth.eye()) < tol * scale,
            "eye {} vs {}",
            found.eye(),
            truth.eye()
        );
        assert!(
            (found.focal - truth.focal).abs() < tol * truth.focal,
            "focal {} vs {}",
            found.focal,
            truth.focal
        );
        for k in 0..3 {
            let (a, b) = (found.rotation.col(k), truth.rotation.col(k));
            assert!(a.distance(b) < tol * 10.0, "axis {k}: {a} vs {b}");
        }
    }

    fn photo_size(rng: &mut Rng) -> DVec2 {
        if rng.next() < 0.5 {
            DVec2::new(1600.0, 1200.0)
        } else {
            DVec2::new(1080.0, 1440.0)
        }
    }

    #[test]
    fn recovers_random_outside_cameras() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut tested = 0;
        while tested < 40 {
            let size = photo_size(&mut rng);
            // A box at the origin seen from the outside: the camera in the -x, -y
            // quadrant so both axes run away from it.
            let az = rng.range(200.0, 250.0).to_radians();
            let dist = rng.range(300.0, 3000.0);
            let eye = DVec3::new(az.cos() * dist, az.sin() * dist, rng.range(-150.0, 900.0));
            let target = DVec3::new(
                rng.range(0.0, 200.0),
                rng.range(0.0, 200.0),
                rng.range(0.0, 150.0),
            );
            let focal = size.x * rng.range(0.6, 2.5);
            let truth = PhotoCamera::look_at(eye, target, rng.range(-12.0, 12.0), focal, size);
            let dims = [
                rng.range(50.0, 400.0),
                rng.range(50.0, 400.0),
                rng.range(30.0, 300.0),
            ];
            let Some(bars) = truth.bars_for_box(dims, dims[2], false) else {
                continue;
            };
            let found = solve(&bars).expect("solvable");
            assert_same(&found, &truth, 1e-7);
            // The scale point is the box's top corner.
            let top = found.project(DVec3::Z * dims[2]).unwrap();
            assert!(top.distance(bars.scale) < 1e-6);
            tested += 1;
        }
    }

    #[test]
    fn recovers_random_inside_cameras() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for _ in 0..30 {
            let size = photo_size(&mut rng);
            // Inside a room in the +x, +y quadrant looking at its corner.
            let eye = DVec3::new(
                rng.range(150.0, 600.0),
                rng.range(150.0, 600.0),
                rng.range(20.0, 250.0),
            );
            let target = DVec3::new(rng.range(-20.0, 40.0), rng.range(-20.0, 40.0), 60.0);
            let focal = size.x * rng.range(0.5, 1.5);
            let truth = PhotoCamera::look_at(eye, target, rng.range(-8.0, 8.0), focal, size);
            let bars = truth
                .bars_for_box([100.0, 100.0, 96.0], 96.0, true)
                .expect("in front");
            let found = solve(&bars).expect("solvable");
            assert_same(&found, &truth, 1e-7);
        }
    }

    #[test]
    fn noisy_bars_give_a_close_camera() {
        let mut rng = Rng(42);
        let size = DVec2::new(1600.0, 1200.0);
        let truth = PhotoCamera::look_at(
            DVec3::new(-700.0, -550.0, 260.0),
            DVec3::new(150.0, 150.0, 80.0),
            3.0,
            1500.0,
            size,
        );
        let mut bars = truth
            .bars_for_box([400.0, 400.0, 250.0], 250.0, false)
            .unwrap();
        for i in 0..8 {
            *bars.end_mut(i) += DVec2::new(rng.range(-0.5, 0.5), rng.range(-0.5, 0.5));
        }
        let found = solve(&bars).unwrap();
        let dist = truth.eye().length();
        assert!(found.eye().distance(truth.eye()) < 0.02 * dist);
        assert!((found.focal - truth.focal).abs() < 0.02 * truth.focal);
    }

    #[test]
    fn default_bars_solve_inside_the_photo() {
        for size in [DVec2::new(1600.0, 1200.0), DVec2::new(900.0, 1600.0)] {
            for style in GridStyle::ALL {
                let bars = default_bars(size, style);
                let cam = solve(&bars).expect("defaults solve");
                assert!((cam.focal - size.x).abs() < 1e-6 * size.x);
                for p in bars.ends().into_iter().chain([bars.origin, bars.scale]) {
                    assert!(
                        p.x > 0.0 && p.y > 0.0 && p.x < size.x && p.y < size.y,
                        "{style:?} {size}: {p}"
                    );
                }
                // The horizon passes through both vanishing points.
                let (h, dir) = cam.horizon().unwrap();
                for d in [DVec3::X, DVec3::Y] {
                    let v = cam.vanishing_point(d).unwrap();
                    assert!((v - h).perp_dot(dir).abs() < 1e-6 * size.x);
                }
            }
        }
    }

    #[test]
    fn degenerate_bars_are_errors() {
        let size = DVec2::new(1600.0, 1200.0);
        // Looking square at the red/blue face: red lines are parallel in the photo.
        let front = PhotoCamera::look_at(
            DVec3::new(100.0, -800.0, 100.0),
            DVec3::new(100.0, 0.0, 100.0),
            0.0,
            1600.0,
            size,
        );
        let bars = front
            .bars_for_box([200.0, 200.0, 200.0], 200.0, false)
            .unwrap();
        assert_eq!(solve(&bars), Err(MatchError::Parallel(Axis::Red)));
        // Nearly square (a millionth of a degree off): still an error, not a camera
        // a thousand miles away.
        let almost = PhotoCamera::look_at(
            DVec3::new(100.0, -800.0, 100.0),
            DVec3::new(100.0 + 800.0 * (1e-6f64).to_radians().tan(), 0.0, 100.0),
            0.0,
            1600.0,
            size,
        );
        let bars = almost
            .bars_for_box([200.0, 200.0, 200.0], 200.0, false)
            .unwrap();
        assert!(matches!(solve(&bars), Err(MatchError::Parallel(_))));
        // Both vanishing points to the right of the center.
        let mut bars = default_bars(size, GridStyle::Above);
        let v = |a: DVec2, b: DVec2| [a, b];
        bars.red = [
            v(DVec2::new(800.0, 700.0), DVec2::new(1200.0, 650.0)),
            v(DVec2::new(800.0, 400.0), DVec2::new(1200.0, 450.0)),
        ];
        bars.green = [
            v(DVec2::new(900.0, 700.0), DVec2::new(1100.0, 640.0)),
            v(DVec2::new(900.0, 300.0), DVec2::new(1100.0, 420.0)),
        ];
        assert_eq!(solve(&bars), Err(MatchError::NotPerpendicular));
        // A bar of no length.
        let mut bars = default_bars(size, GridStyle::Above);
        bars.green[1][1] = bars.green[1][0];
        assert_eq!(solve(&bars), Err(MatchError::ShortBar));
        // The scale point on the origin, or below it.
        let mut bars = default_bars(size, GridStyle::Above);
        bars.scale = bars.origin;
        assert_eq!(solve(&bars), Err(MatchError::Scale));
        let mut bars = default_bars(size, GridStyle::Above);
        bars.scale = bars.origin * 2.0 - bars.scale;
        assert_eq!(solve(&bars), Err(MatchError::Scale));
    }

    #[test]
    fn mirrored_bars_keep_blue_up() {
        // Red and green swapped: green is turned around so blue still points up.
        let size = DVec2::new(1600.0, 1200.0);
        let mut bars = default_bars(size, GridStyle::Above);
        std::mem::swap(&mut bars.red, &mut bars.green);
        let cam = solve(&bars).unwrap();
        let up = cam.rotation * DVec3::Z;
        assert!(up.y < 0.0);
        assert!((cam.rotation.determinant() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn moving_the_origin_keeps_the_scale() {
        let size = DVec2::new(1600.0, 1200.0);
        let mut bars = default_bars(size, GridStyle::Above);
        let cam = solve(&bars).unwrap();
        bars.move_origin(bars.origin + DVec2::new(40.0, -25.0), &cam);
        let moved = solve(&bars).unwrap();
        assert!((moved.depth() - cam.depth()).abs() < 1e-6 * cam.depth());
        assert!(moved.project(DVec3::ZERO).unwrap().distance(bars.origin) < 1e-6);
    }

    #[test]
    fn projected_texture_lands_on_the_photo_pixels() {
        let size = DVec2::new(1600.0, 1200.0);
        let cam = PhotoCamera::look_at(
            DVec3::new(-600.0, -500.0, 300.0),
            DVec3::new(100.0, 100.0, 50.0),
            4.0,
            1400.0,
            size,
        );
        let xf = DMat4::from_translation(DVec3::new(10.0, 20.0, 0.0));
        // A wall (normal -y) and the ground, in the face's own coordinates.
        for (normal, on) in [
            (DVec3::NEG_Y, DVec3::new(0.0, 0.0, 0.0)),
            (DVec3::Z, DVec3::new(0.0, 0.0, 0.0)),
        ] {
            let m = cam.face_uv(xf, normal, on).unwrap();
            let map = mt_skp::uv::UvMapping::new(normal.to_array(), Some(&m), [1.0, 1.0]);
            let (xr, yr) = mt_skp::uv::face_uv_basis(normal.to_array());
            let (xr, yr) = (DVec3::from_array(xr), DVec3::from_array(yr));
            for (a, b) in [(0.0, 0.0), (50.0, 10.0), (120.0, -30.0)] {
                let p = on + xr * a + yr * b;
                let px = cam.project(xf.transform_point3(p)).unwrap();
                let uv = map.uv(p.to_array());
                let want = [px.x / size.x, 1.0 - px.y / size.y];
                assert!(
                    (uv[0] - want[0]).abs() < 1e-9 && (uv[1] - want[1]).abs() < 1e-9,
                    "{uv:?} vs {want:?}"
                );
            }
        }
    }
}
