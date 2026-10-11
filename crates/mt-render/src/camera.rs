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

/// The Camera > Standard Views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardView {
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
    Iso,
}

impl StandardView {
    pub fn from_command(id: &str) -> Option<Self> {
        Some(match id {
            "camera.view_top" => Self::Top,
            "camera.view_bottom" => Self::Bottom,
            "camera.view_front" => Self::Front,
            "camera.view_back" => Self::Back,
            "camera.view_left" => Self::Left,
            "camera.view_right" => Self::Right,
            "camera.view_iso" => Self::Iso,
            _ => return None,
        })
    }
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

pub(crate) fn look_at_rh(eye: DVec3, target: DVec3, up: DVec3) -> DMat4 {
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
pub(crate) fn orthographic_rh(
    left: f64,
    right: f64,
    bottom: f64,
    top: f64,
    near: f64,
    far: f64,
) -> DMat4 {
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

/// Smooth ease-in-out (cubic smoothstep) of `t` clamped to 0..1.
pub fn ease_in_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Camera {
    /// Moves the eye along the view direction so the sphere around `min`-`max` fits
    /// the view (`aspect` = width / height), looking at its center. Fitting the
    /// bounding sphere leaves a margin; SketchUp's exact margin is unverified.
    pub fn fit(&mut self, min: DVec3, max: DVec3, aspect: f64) {
        let center = (min + max) / 2.0;
        let radius = ((max - min).length() / 2.0).max(1.0);
        let half_v = (self.fov_deg.to_radians() / 2.0).max(1e-3);
        let distance = if self.perspective {
            let half_h = (half_v.tan() * aspect).atan();
            radius / half_v.min(half_h).sin()
        } else {
            // The view height at the target scales with the distance.
            radius / aspect.min(1.0) / half_v.tan()
        };
        let forward = self.forward();
        self.target = center;
        self.eye = center - forward * distance;
    }

    /// Zoom Window: centers the view on the pixel box `a`-`b` and zooms so the box
    /// fills the view, keeping the view direction. Depth is taken at the target's
    /// distance (SketchUp may zoom to the geometry under the box; unverified).
    pub fn zoom_window(&mut self, a: DVec2, b: DVec2, viewport: DVec2) {
        let size = (b - a).abs();
        if size.x < 2.0 || size.y < 2.0 {
            return;
        }
        let center = (a + b) / 2.0;
        let Some(on_plane) = self
            .ray(center, viewport)
            .hit_plane(self.target, self.forward())
        else {
            return;
        };
        let scale = (size.x / viewport.x).max(size.y / viewport.y);
        let forward = self.forward();
        self.target = on_plane;
        self.eye = on_plane - forward * self.distance() * scale;
    }

    /// The camera looking along `view` (from the eye towards the target) at the
    /// same target, distance, field of view and projection.
    pub fn looking(&self, view: StandardView) -> Camera {
        let dir = match view {
            StandardView::Top => -DVec3::Z,
            StandardView::Bottom => DVec3::Z,
            StandardView::Front => DVec3::Y,
            StandardView::Back => -DVec3::Y,
            StandardView::Left => DVec3::X,
            StandardView::Right => -DVec3::X,
            StandardView::Iso => {
                // The isometric view from the quadrant the camera is in (unverified:
                // SketchUp may always use the front-left one).
                let f = self.forward();
                let sx = if f.x < 0.0 { -1.0 } else { 1.0 };
                let sy = if f.y < 0.0 { -1.0 } else { 1.0 };
                DVec3::new(sx, sy, -1.0).normalize()
            }
        };
        Camera {
            eye: self.target - dir * self.distance(),
            ..self.clone()
        }
    }

    /// The view a fraction `t` (0..1, not eased) of the way from `self` to `to`, for
    /// scene transitions. The target moves in a straight line; the eye swings around it
    /// (azimuth the short way round, elevation linearly) while its distance changes
    /// geometrically, so the camera never passes through the target. The field of view
    /// is interpolated linearly; the projection switches at the end. SketchUp's own
    /// transition path is unverified.
    pub fn interpolate(&self, to: &Camera, t: f64) -> Camera {
        if t <= 0.0 {
            return self.clone();
        }
        if t >= 1.0 {
            return to.clone();
        }
        let polar = |c: &Camera| {
            let d = c.eye - c.target;
            let r = d.length().max(1e-9);
            (d.y.atan2(d.x), (d.z / r).clamp(-1.0, 1.0).asin(), r)
        };
        let (az0, el0, r0) = polar(self);
        let (az1, el1, r1) = polar(to);
        let tau = std::f64::consts::TAU;
        let daz = (az1 - az0 + std::f64::consts::PI).rem_euclid(tau) - std::f64::consts::PI;
        let az = az0 + daz * t;
        let el = el0 + (el1 - el0) * t;
        let r = r0 * (r1 / r0).powf(t);
        let target = self.target.lerp(to.target, t);
        let dir = DVec3::new(el.cos() * az.cos(), el.cos() * az.sin(), el.sin());
        Camera {
            eye: target + dir * r,
            target,
            fov_deg: self.fov_deg + (to.fov_deg - self.fov_deg) * t,
            perspective: self.perspective,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: DVec2 = DVec2::new(1600.0, 900.0);

    #[test]
    fn fit_keeps_direction_and_shows_the_box() {
        for perspective in [true, false] {
            let mut c = camera(perspective);
            let forward = c.forward();
            let (min, max) = (DVec3::new(-50.0, 0.0, 0.0), DVec3::new(150.0, 80.0, 60.0));
            c.fit(min, max, VIEWPORT.x / VIEWPORT.y);
            assert!((c.forward() - forward).length() < 1e-9);
            assert_eq!(c.target, (min + max) / 2.0);
            for x in [min.x, max.x] {
                for y in [min.y, max.y] {
                    for z in [min.z, max.z] {
                        let p = c.project(DVec3::new(x, y, z), VIEWPORT).unwrap();
                        assert!(p.x >= 0.0 && p.x <= VIEWPORT.x, "{p} {perspective}");
                        assert!(p.y >= 0.0 && p.y <= VIEWPORT.y, "{p} {perspective}");
                    }
                }
            }
        }
    }

    #[test]
    fn zoom_window_centers_and_fills() {
        for perspective in [true, false] {
            let mut c = camera(perspective);
            // A point at the target's depth.
            let p = c.target + c.right() * 30.0 + c.up() * 20.0;
            let px = c.project(p, VIEWPORT).unwrap();
            let half = DVec2::new(80.0, 30.0);
            c.zoom_window(px - half, px + half, VIEWPORT);
            // The box center is now at the screen center and the box's width fills it.
            let center = c.project(p, VIEWPORT).unwrap();
            assert!(
                (center - VIEWPORT / 2.0).length() < 1e-6,
                "{center} {perspective}"
            );
            let edge = c.project(p + c.right() * 1.0, VIEWPORT).unwrap();
            let px_per_unit = (edge - center).length();
            assert!(px_per_unit > 5.0, "zoomed in {px_per_unit}");
        }
    }

    #[test]
    fn standard_views() {
        let c = camera(true);
        let top = c.looking(StandardView::Top);
        assert_eq!(top.target, c.target);
        assert!((top.distance() - c.distance()).abs() < 1e-9);
        assert!((top.forward() + DVec3::Z).length() < 1e-12);
        assert_eq!(top.right(), DVec3::X, "Top: red runs right");
        let front = c.looking(StandardView::Front);
        assert!((front.up() - DVec3::Z).length() < 1e-12);
        assert!((front.right() - DVec3::X).length() < 1e-12);
        let iso = c.looking(StandardView::Iso);
        // The camera is at -x -y looking towards +x +y.
        let f = iso.forward();
        assert!(f.x > 0.0 && f.y > 0.0 && f.z < 0.0);
        assert!((f.z.abs() - 1.0 / 3f64.sqrt()).abs() < 1e-12);
    }

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

    #[test]
    fn ease_in_out_is_smooth_and_clamped() {
        assert_eq!(ease_in_out(-1.0), 0.0);
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(0.5), 0.5);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert_eq!(ease_in_out(2.0), 1.0);
        // Slow at both ends.
        assert!(ease_in_out(0.1) < 0.1 && ease_in_out(0.9) > 0.9);
    }

    #[test]
    fn interpolate_hits_both_ends_and_swings_around_target() {
        let a = Camera {
            eye: DVec3::new(100.0, 0.0, 0.0),
            target: DVec3::ZERO,
            fov_deg: 30.0,
            perspective: true,
        };
        let b = Camera {
            eye: DVec3::new(10.0, 400.0, 0.0),
            target: DVec3::new(10.0, 0.0, 0.0),
            fov_deg: 60.0,
            perspective: false,
        };
        assert_eq!(a.interpolate(&b, 0.0), a);
        assert_eq!(a.interpolate(&b, 1.0), b);
        let mid = a.interpolate(&b, 0.5);
        assert!(mid.target.distance(DVec3::new(5.0, 0.0, 0.0)) < 1e-9);
        assert!((mid.fov_deg - 45.0).abs() < 1e-9);
        assert!(mid.perspective);
        // Halfway between 100 and 400 geometrically, at 45 degrees.
        assert!((mid.distance() - 200.0).abs() < 1e-9);
        let d = (mid.eye - mid.target).normalize();
        assert!(d.distance(DVec3::new(1.0, 1.0, 0.0).normalize()) < 1e-9);
        // Continuous: small steps move the eye a little.
        let mut prev = a.eye;
        for i in 1..=100 {
            let c = a.interpolate(&b, f64::from(i) / 100.0);
            assert!(c.eye.distance(prev) < 20.0);
            prev = c.eye;
        }
    }

    #[test]
    fn interpolate_takes_the_short_way_round() {
        let at = |deg: f64| Camera {
            eye: DVec3::new(deg.to_radians().cos(), deg.to_radians().sin(), 0.0) * 100.0,
            target: DVec3::ZERO,
            fov_deg: 35.0,
            perspective: true,
        };
        let mid = at(170.0).interpolate(&at(-170.0), 0.5);
        assert!(mid.eye.distance(DVec3::new(-100.0, 0.0, 0.0)) < 1e-6);
    }
}
