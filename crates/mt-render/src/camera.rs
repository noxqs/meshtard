//! Camera with SketchUp-style navigation. World is Z-up, units are inches.
//!
//! Navigation constants are first guesses; scenario `spec/parity/00_camera_navigation.md`
//! measures the real values.

use glam::{DMat4, DVec2, DVec3, DVec4};
use mt_spec::CameraSpec;

/// Degrees of orbit per pixel of mouse drag.
pub const ORBIT_DEG_PER_PX: f64 = 0.4;
/// Fraction of the distance to the zoom point covered per wheel notch.
pub const ZOOM_PER_NOTCH: f64 = 0.2;
/// Keeps the camera from flipping over the pole while orbiting.
const MAX_ELEVATION_DEG: f64 = 89.9;
/// Near clip distance as a fraction of the eye-target distance.
const NEAR_FRACTION: f64 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub eye: DVec3,
    pub target: DVec3,
    /// Vertical field of view in degrees. In parallel projection it sets the view height
    /// at the target distance, so switching projection keeps the target the same size.
    pub fov_deg: f64,
    pub perspective: bool,
}

/// A ray in world space. `dir` is normalized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: DVec3,
    pub dir: DVec3,
}

impl Ray {
    /// Intersection with the plane through `point` with normal `normal`, if in front of the ray.
    pub fn hit_plane(&self, point: DVec3, normal: DVec3) -> Option<DVec3> {
        let denom = self.dir.dot(normal);
        if denom.abs() < 1e-12 {
            return None;
        }
        let t = (point - self.origin).dot(normal) / denom;
        (t > 0.0).then(|| self.origin + self.dir * t)
    }
}

impl Camera {
    pub fn from_spec(spec: &CameraSpec) -> Self {
        Self {
            eye: spec.eye.into(),
            target: spec.target.into(),
            fov_deg: spec.fov_deg,
            perspective: spec.perspective,
        }
    }

    pub fn distance(&self) -> f64 {
        self.eye.distance(self.target)
    }

    pub fn forward(&self) -> DVec3 {
        (self.target - self.eye).normalize()
    }

    /// Camera right vector. Looking straight down, world +X is right (SketchUp Top view).
    pub fn right(&self) -> DVec3 {
        let r = self.forward().cross(DVec3::Z);
        if r.length_squared() < 1e-12 {
            DVec3::X
        } else {
            r.normalize()
        }
    }

    pub fn up(&self) -> DVec3 {
        self.right().cross(self.forward())
    }

    pub fn view(&self) -> DMat4 {
        look_at_rh(self.eye, self.target, self.up())
    }

    /// Reverse-Z projection: depth 1 at the near plane, 0 at infinity (perspective)
    /// or at the far plane (parallel).
    pub fn projection(&self, aspect: f64) -> DMat4 {
        let near = self.near();
        if self.perspective {
            perspective_infinite_reverse_rh(self.fov_deg.to_radians(), aspect, near)
        } else {
            let half_h = self.half_height_at_target();
            let half_w = half_h * aspect;
            let far = self.distance() * 1e4;
            // Swapping near and far reverses the depth range.
            orthographic_rh(-half_w, half_w, -half_h, half_h, far, -far)
        }
    }

    /// Near clip distance; in perspective also the clip-space `w` at the near plane.
    pub fn near(&self) -> f64 {
        self.distance() * NEAR_FRACTION
    }

    pub fn view_proj(&self, aspect: f64) -> DMat4 {
        self.projection(aspect) * self.view()
    }

    fn half_height_at_target(&self) -> f64 {
        self.distance() * (self.fov_deg.to_radians() / 2.0).tan()
    }

    /// World units per pixel at the target distance.
    pub fn units_per_px(&self, viewport: DVec2) -> f64 {
        2.0 * self.half_height_at_target() / viewport.y
    }

    /// Ray through a pixel (origin top-left, y down).
    pub fn ray(&self, px: DVec2, viewport: DVec2) -> Ray {
        let ndc = DVec2::new(2.0 * px.x / viewport.x - 1.0, 1.0 - 2.0 * px.y / viewport.y);
        let inv = self.view_proj(viewport.x / viewport.y).inverse();
        let unproject = |depth: f64| {
            let p = inv * DVec4::new(ndc.x, ndc.y, depth, 1.0);
            p.truncate() / p.w
        };
        let near = unproject(1.0);
        let mid = unproject(0.5);
        Ray {
            origin: near,
            dir: (mid - near).normalize(),
        }
    }

    /// Pixel position of a world point, or `None` if behind the camera.
    pub fn project(&self, p: DVec3, viewport: DVec2) -> Option<DVec2> {
        let clip = self.view_proj(viewport.x / viewport.y) * p.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(DVec2::new(
            (ndc.x + 1.0) * 0.5 * viewport.x,
            (1.0 - ndc.y) * 0.5 * viewport.y,
        ))
    }

    /// Turntable orbit around the target: horizontal drag spins around world Z,
    /// vertical drag tilts. `delta` is the mouse movement in pixels.
    pub fn orbit(&mut self, delta: DVec2) {
        let offset = self.eye - self.target;
        let radius = offset.length();
        let mut azimuth = offset.y.atan2(offset.x);
        let mut elevation = (offset.z / radius).asin();
        azimuth -= (delta.x * ORBIT_DEG_PER_PX).to_radians();
        elevation += (delta.y * ORBIT_DEG_PER_PX).to_radians();
        let max = MAX_ELEVATION_DEG.to_radians();
        elevation = elevation.clamp(-max, max);
        self.eye = self.target
            + radius
                * DVec3::new(
                    elevation.cos() * azimuth.cos(),
                    elevation.cos() * azimuth.sin(),
                    elevation.sin(),
                );
    }

    /// Moves eye and target so the point at target depth under the cursor follows the cursor.
    pub fn pan(&mut self, delta: DVec2, viewport: DVec2) {
        let scale = self.units_per_px(viewport);
        let shift = (-delta.x * self.right() + delta.y * self.up()) * scale;
        self.eye += shift;
        self.target += shift;
    }

    /// Zooms toward the point under the cursor. `notches` > 0 zooms in.
    ///
    /// Scales eye and target about the cursor point, so that point stays under the
    /// cursor and the view direction is unchanged.
    pub fn zoom_at(&mut self, notches: f64, px: DVec2, viewport: DVec2) {
        let ray = self.ray(px, viewport);
        let Some(pivot) = ray.hit_plane(self.target, self.forward()) else {
            return;
        };
        let scale = (1.0 - ZOOM_PER_NOTCH).powf(notches);
        if self.perspective {
            self.eye = pivot + (self.eye - pivot) * scale;
            self.target = pivot + (self.target - pivot) * scale;
        } else {
            // Parallel projection: view height scales with distance, eye moves sideways only.
            let forward = self.forward();
            let side = |p: DVec3| p - forward * (p - pivot).dot(forward);
            let new_target = pivot + (side(self.target) - pivot) * scale;
            let new_dist = self.distance() * scale;
            self.target = new_target;
            self.eye = new_target - forward * new_dist;
        }
    }

    /// Drag-zoom used by the Zoom tool: dragging up zooms in toward the view center.
    pub fn zoom_drag(&mut self, delta_y: f64, viewport: DVec2) {
        self.zoom_at(-delta_y / 20.0, viewport / 2.0, viewport);
    }
}

fn look_at_rh(eye: DVec3, target: DVec3, up: DVec3) -> DMat4 {
    let f = (target - eye).normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    DMat4::from_cols(
        DVec4::new(s.x, u.x, -f.x, 0.0),
        DVec4::new(s.y, u.y, -f.y, 0.0),
        DVec4::new(s.z, u.z, -f.z, 0.0),
        DVec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// Depth `near / distance`: 1 at the near plane, 0 at infinity.
fn perspective_infinite_reverse_rh(fov_y: f64, aspect: f64, near: f64) -> DMat4 {
    let h = 1.0 / (fov_y / 2.0).tan();
    DMat4::from_cols(
        DVec4::new(h / aspect, 0.0, 0.0, 0.0),
        DVec4::new(0.0, h, 0.0, 0.0),
        DVec4::new(0.0, 0.0, 0.0, -1.0),
        DVec4::new(0.0, 0.0, near, 0.0),
    )
}

/// Maps view-space `-near..-far` to depth `0..1`.
fn orthographic_rh(left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) -> DMat4 {
    let rw = 1.0 / (right - left);
    let rh = 1.0 / (top - bottom);
    let r = 1.0 / (near - far);
    DMat4::from_cols(
        DVec4::new(2.0 * rw, 0.0, 0.0, 0.0),
        DVec4::new(0.0, 2.0 * rh, 0.0, 0.0),
        DVec4::new(0.0, 0.0, r, 0.0),
        DVec4::new(-(left + right) * rw, -(top + bottom) * rh, r * near, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: DVec2 = DVec2::new(1600.0, 900.0);

    fn camera(perspective: bool) -> Camera {
        Camera {
            eye: DVec3::new(-400.0, -600.0, 300.0),
            target: DVec3::new(10.0, 20.0, 0.0),
            fov_deg: 35.0,
            perspective,
        }
    }

    fn assert_close(a: DVec2, b: DVec2) {
        assert!(a.distance(b) < 1e-6, "{a} != {b}");
    }

    #[test]
    fn target_projects_to_center() {
        for perspective in [true, false] {
            let c = camera(perspective);
            assert_close(c.project(c.target, VIEWPORT).unwrap(), VIEWPORT / 2.0);
        }
    }

    #[test]
    fn ray_and_project_agree() {
        for perspective in [true, false] {
            let c = camera(perspective);
            let px = DVec2::new(123.0, 456.0);
            let ray = c.ray(px, VIEWPORT);
            assert_close(
                c.project(ray.origin + ray.dir * 500.0, VIEWPORT).unwrap(),
                px,
            );
        }
    }

    #[test]
    fn orbit_keeps_distance_and_target() {
        let mut c = camera(true);
        let (d, t) = (c.distance(), c.target);
        c.orbit(DVec2::new(250.0, -80.0));
        assert!((c.distance() - d).abs() < 1e-9);
        assert_eq!(c.target, t);
    }

    #[test]
    fn orbit_horizontal_spins_around_z() {
        let mut c = camera(true);
        let z = c.eye.z;
        c.orbit(DVec2::new(90.0 / ORBIT_DEG_PER_PX, 0.0));
        assert!((c.eye.z - z).abs() < 1e-9);
    }

    #[test]
    fn orbit_does_not_flip_over_the_pole() {
        let mut c = camera(true);
        c.orbit(DVec2::new(0.0, 10_000.0));
        assert!(c.up().z > 0.0);
        c.orbit(DVec2::new(0.0, -20_000.0));
        assert!(c.up().z > 0.0);
    }

    #[test]
    fn pan_keeps_point_under_cursor() {
        for perspective in [true, false] {
            let mut c = camera(perspective);
            let start = VIEWPORT / 2.0 + DVec2::new(0.0, 0.0);
            let delta = DVec2::new(37.0, -22.0);
            let p = c.target;
            c.pan(delta, VIEWPORT);
            assert_close(c.project(p, VIEWPORT).unwrap(), start + delta);
        }
    }

    #[test]
    fn zoom_keeps_point_under_cursor() {
        for perspective in [true, false] {
            let mut c = camera(perspective);
            let px = DVec2::new(300.0, 200.0);
            let p = c
                .ray(px, VIEWPORT)
                .hit_plane(c.target, c.forward())
                .unwrap();
            let d = c.distance();
            c.zoom_at(1.0, px, VIEWPORT);
            assert!(c.distance() < d);
            let after = c.project(p, VIEWPORT).unwrap();
            assert!(after.distance(px) < 1e-6, "{perspective}: {after} != {px}");
        }
    }

    #[test]
    fn straight_down_has_x_right() {
        let c = Camera {
            eye: DVec3::new(0.0, 0.0, 100.0),
            target: DVec3::ZERO,
            fov_deg: 35.0,
            perspective: true,
        };
        assert_eq!(c.right(), DVec3::X);
        assert!(c.up().distance(DVec3::Y) < 1e-12);
    }
}
