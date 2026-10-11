//! Camera > Position Camera, Walk, Look Around and Field of View.
//!
//! Look Around and Field of View are plain drags (see `nav::apply`); this module
//! holds Position Camera (picking where to stand), Walk (continuous motion while the
//! mouse is held) and what all four take in the Measurements box.
//!
//! From memory of SketchUp, all unverified: the eye stands 5' 6" above the clicked
//! point; after Position Camera the Look Around tool is active; Walk moves faster the
//! farther the cursor is from where the drag started (up/down walks, left/right
//! turns; Shift moves up/down and sideways; Ctrl runs); Walk does not stop at walls
//! here (no collision detection). Typed focal lengths are for 35 mm film.

use eframe::egui;
use glam::{DVec2, DVec3};
use mt_infer::units::{LengthUnit, parse_length};
use mt_render::Camera;
use mt_render::camera::fov_from_focal_length;

use crate::App;
use crate::nav::Tool;

/// SketchUp's default eye height, 5' 6" (unverified).
pub(crate) const EYE_HEIGHT: f64 = 66.0;
/// Walk speed in inches per second per pixel the cursor is from the drag start.
const WALK_SPEED: f64 = 0.5;
/// Walk turn rate in degrees per second per pixel.
const TURN_RATE: f64 = 0.3;
/// Ctrl (run) multiplies the walk speed (unverified).
const RUN_FACTOR: f64 = 3.0;
/// Cursor offsets smaller than this (pixels) do not move.
const DEAD_ZONE: f64 = 3.0;

#[derive(Debug, Clone)]
pub(crate) struct CameraTools {
    /// Eye height for Position Camera's next click.
    pub eye_height: f64,
    /// Height of the ground the eye height is measured from (the last positioned
    /// point; the ground plane until then).
    ground_z: f64,
    /// A Walk drag is in progress (its history entry is recorded).
    walking: bool,
}

impl Default for CameraTools {
    fn default() -> Self {
        Self {
            eye_height: EYE_HEIGHT,
            ground_z: 0.0,
            walking: false,
        }
    }
}

/// A value typed into the Measurements box with the Field of View tool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FovInput {
    Degrees(f64),
    /// Focal length in millimeters.
    FocalLength(f64),
}

impl FovInput {
    /// `35`, `35deg`, `35°` or `50mm`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let t = text.trim().to_ascii_lowercase().replace(',', ".");
        let (num, focal) = if let Some(n) = t.strip_suffix("mm") {
            (n, true)
        } else if let Some(n) = t.strip_suffix("deg").or_else(|| t.strip_suffix('°')) {
            (n, false)
        } else {
            (t.as_str(), false)
        };
        let v: f64 = num.trim().parse().ok()?;
        (v > 0.0 && v.is_finite()).then_some(if focal {
            Self::FocalLength(v)
        } else {
            Self::Degrees(v)
        })
    }

    pub(crate) fn degrees(self) -> f64 {
        match self {
            Self::Degrees(d) => d,
            Self::FocalLength(mm) => fov_from_focal_length(mm),
        }
    }
}

/// The camera standing at `ground` (eye `height` above it), looking at `look` or,
/// without one, horizontally in `camera`'s direction at the same distance.
pub(crate) fn stand_at(camera: &Camera, ground: DVec3, height: f64, look: Option<DVec3>) -> Camera {
    let eye = ground + DVec3::Z * height;
    let target = match look {
        Some(p) if p.distance(eye) > 1e-6 => p,
        _ => eye + camera.horizontal_forward() * camera.distance().max(1.0),
    };
    Camera {
        eye,
        target,
        ..camera.clone()
    }
}

impl App {
    /// The point under pixel `px`: on the model, else on the ground plane.
    fn ground_point(&self, px: DVec2, viewport: DVec2) -> Option<DVec3> {
        let ray = self.camera.ray(px, viewport);
        match self.bvh.ray(ray.origin, ray.dir) {
            Some(hit) => Some(ray.origin + ray.dir * hit.t),
            None => ray.hit_plane(DVec3::ZERO, DVec3::Z),
        }
    }

    /// Position Camera and Walk, per frame while active.
    pub(crate) fn camera_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
    ) {
        let ppp = ui.ctx().pixels_per_point();
        let to_px = |p: egui::Pos2| {
            DVec2::new(
                f64::from((p.x - rect.min.x) * ppp),
                f64::from((p.y - rect.min.y) * ppp),
            )
        };
        let primary = egui::PointerButton::Primary;
        match self.tool {
            Tool::PositionCamera => {
                let (from, to) = if response.clicked_by(primary) {
                    let at = response.interact_pointer_pos().map(to_px);
                    (at, None)
                } else if response.drag_stopped_by(primary) {
                    (
                        ui.input(|i| i.pointer.press_origin()).map(to_px),
                        response.interact_pointer_pos().map(to_px),
                    )
                } else {
                    return;
                };
                let Some(ground) = from.and_then(|p| self.ground_point(p, viewport_px)) else {
                    return;
                };
                let look = to.and_then(|p| self.ground_point(p, viewport_px));
                let before = self.camera.clone();
                self.camera = stand_at(&before, ground, self.camera_tools.eye_height, look);
                self.camera_tools.ground_z = ground.z;
                self.camera_history.push(before);
                self.transition = None;
                self.tool = Tool::LookAround;
            }
            Tool::Walk => {
                let dragging = response.dragged_by(primary);
                if !dragging {
                    self.camera_tools.walking = false;
                    return;
                }
                let (Some(start), Some(now)) = (
                    ui.input(|i| i.pointer.press_origin()).map(to_px),
                    response.interact_pointer_pos().map(to_px),
                ) else {
                    return;
                };
                if !self.camera_tools.walking {
                    self.camera_tools.walking = true;
                    self.camera_history.push(self.camera.clone());
                }
                let (shift, ctrl, dt) =
                    ui.input(|i| (i.modifiers.shift, i.modifiers.command, i.stable_dt));
                let dt = f64::from(dt.min(0.1));
                let offset = (now - start)
                    .to_array()
                    .map(|v| if v.abs() < DEAD_ZONE { 0.0 } else { v });
                let speed = WALK_SPEED * if ctrl { RUN_FACTOR } else { 1.0 } * dt;
                if shift {
                    self.camera
                        .walk(0.0, offset[0] * speed, -offset[1] * speed, 0.0);
                } else {
                    self.camera
                        .walk(-offset[1] * speed, 0.0, 0.0, offset[0] * TURN_RATE * dt);
                }
                self.transition = None;
                ui.ctx().request_repaint();
            }
            _ => {}
        }
    }

    /// Measurements box label and value for the camera tools; the plain box for
    /// other tools.
    pub(crate) fn camera_tool_measurement(&self, units: LengthUnit) -> (&'static str, String) {
        match self.tool {
            Tool::PositionCamera => ("Eye Height", units.format(self.camera_tools.eye_height)),
            Tool::Walk | Tool::LookAround => (
                "Eye Height",
                units.format(self.camera.eye.z - self.camera_tools.ground_z),
            ),
            Tool::FieldOfView if self.camera.perspective => {
                ("Field of View", format!("{:.2} deg", self.camera.fov_deg))
            }
            Tool::FieldOfView => ("Height", units.format(self.camera.view_height())),
            _ => ("Measurements", String::new()),
        }
    }

    /// A value typed into the Measurements box with a camera tool. Returns whether
    /// it was one of theirs.
    pub(crate) fn camera_tool_typed(&mut self, text: &str, units: LengthUnit) -> bool {
        let before = self.camera.clone();
        match self.tool {
            Tool::PositionCamera | Tool::Walk | Tool::LookAround => {
                let Some(h) = parse_length(text, units) else {
                    self.message = Some(format!("Invalid eye height: {text}"));
                    return true;
                };
                self.camera_tools.eye_height = h;
                if self.tool != Tool::PositionCamera {
                    self.camera.set_eye_z(self.camera_tools.ground_z + h);
                }
            }
            Tool::FieldOfView => {
                if let Some(v) = FovInput::parse(text).filter(|_| self.camera.perspective) {
                    self.camera.set_fov(v.degrees());
                } else if let Some(h) = parse_length(text, units).filter(|h| *h > 0.0) {
                    // A length sets the visible height (parallel projection, or a
                    // perspective view's height at the target).
                    self.camera.set_view_height(h);
                    self.camera.set_fov(self.camera.fov_deg);
                } else if let Some(v) = FovInput::parse(text) {
                    self.camera.set_fov(v.degrees());
                } else {
                    self.message = Some(format!("Invalid field of view: {text}"));
                    return true;
                }
            }
            _ => return false,
        }
        if self.camera != before {
            self.camera_history.push(before);
            self.transition = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fov_input_parses_degrees_and_focal_length() {
        assert_eq!(FovInput::parse("35"), Some(FovInput::Degrees(35.0)));
        assert_eq!(FovInput::parse(" 35deg "), Some(FovInput::Degrees(35.0)));
        assert_eq!(FovInput::parse("42,5°"), Some(FovInput::Degrees(42.5)));
        assert_eq!(FovInput::parse("50mm"), Some(FovInput::FocalLength(50.0)));
        assert_eq!(FovInput::parse("abc"), None);
        assert_eq!(FovInput::parse("-3"), None);
        let d = FovInput::FocalLength(50.0).degrees();
        assert!((d - 26.991).abs() < 1e-3, "{d}");
    }

    #[test]
    fn stand_at_puts_the_eye_above_the_point() {
        let c = Camera {
            eye: DVec3::new(0.0, -500.0, 300.0),
            target: DVec3::ZERO,
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        let ground = DVec3::new(10.0, 20.0, 5.0);
        let s = stand_at(&c, ground, EYE_HEIGHT, None);
        assert_eq!(s.eye, DVec3::new(10.0, 20.0, 71.0));
        // Looks level, the way the camera was facing.
        assert!(s.forward().distance(DVec3::Y) < 1e-9);
        let look = DVec3::new(10.0, 400.0, 0.0);
        assert_eq!(stand_at(&c, ground, 60.0, Some(look)).target, look);
    }
}
