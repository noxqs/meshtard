//! Viewport navigation: the Orbit / Pan / Zoom tools and the mouse shortcuts that
//! work in every tool (middle drag, shift + middle drag, wheel).

use glam::DVec2;
use mt_render::Camera;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    /// Tools > Lasso (see `lasso`).
    Lasso,
    /// A drawing tool from `tools`, by command id.
    Draw(&'static str),
    /// A modify tool from `modify`, by command id.
    Modify(&'static str),
    Eraser,
    /// A click tool from `pick_tool` (Solid Tools, Tag), by command id.
    Pick(&'static str),
    Orbit,
    Pan,
    Zoom,
    ZoomWindow,
    /// Camera > Position Camera, Walk, Look Around and Field of View (see
    /// `camera_tools`).
    PositionCamera,
    Walk,
    LookAround,
    FieldOfView,
}

/// Degrees of field of view per pixel of Field of View drag (unverified).
pub const FOV_DEG_PER_PX: f64 = 0.1;

impl Tool {
    pub fn from_command(id: &str) -> Option<Self> {
        Some(match id {
            "tool.select" => Self::Select,
            "tool.lasso" => Self::Lasso,
            "tool.eraser" => Self::Eraser,
            "tool.line" => Self::Draw("tool.line"),
            "tool.rectangle" => Self::Draw("tool.rectangle"),
            "tool.rotated_rectangle" => Self::Draw("tool.rotated_rectangle"),
            "tool.circle" => Self::Draw("tool.circle"),
            "tool.polygon" => Self::Draw("tool.polygon"),
            "tool.arc" => Self::Draw("tool.arc"),
            "tool.arc_2pt" => Self::Draw("tool.arc_2pt"),
            "tool.arc_3pt" => Self::Draw("tool.arc_3pt"),
            "tool.pie" => Self::Draw("tool.pie"),
            "tool.freehand" => Self::Draw("tool.freehand"),
            "tool.tape_measure" => Self::Draw("tool.tape_measure"),
            "tool.protractor" => Self::Draw("tool.protractor"),
            "tool.dimensions" => Self::Draw("tool.dimensions"),
            "tool.text" => Self::Draw("tool.text"),
            "tool.push_pull" => Self::Modify("tool.push_pull"),
            "tool.move" => Self::Modify("tool.move"),
            "tool.rotate" => Self::Modify("tool.rotate"),
            "tool.scale" => Self::Modify("tool.scale"),
            "tool.offset" => Self::Modify("tool.offset"),
            "tool.follow_me" => Self::Modify("tool.follow_me"),
            "tool.paint_bucket" => Self::Modify("tool.paint_bucket"),
            "tool.flip" => Self::Modify("tool.flip"),
            "tool.section_plane" => Self::Modify("tool.section_plane"),
            "tool.axes" => Self::Modify("tool.axes"),
            "tool.sandbox_from_scratch" => Self::Modify("tool.sandbox_from_scratch"),
            "tool.snaps" => Self::Modify("tool.snaps"),
            "tool.tag" => Self::Pick("tool.tag"),
            "tool.outer_shell" => Self::Pick("tool.outer_shell"),
            "tool.solid_intersect" => Self::Pick("tool.solid_intersect"),
            "tool.solid_union" => Self::Pick("tool.solid_union"),
            "tool.solid_subtract" => Self::Pick("tool.solid_subtract"),
            "tool.solid_trim" => Self::Pick("tool.solid_trim"),
            "tool.solid_split" => Self::Pick("tool.solid_split"),
            "camera.orbit" => Self::Orbit,
            "camera.pan" => Self::Pan,
            "camera.zoom" => Self::Zoom,
            "camera.zoom_window" => Self::ZoomWindow,
            "camera.position" => Self::PositionCamera,
            "camera.walk" => Self::Walk,
            "camera.look_around" => Self::LookAround,
            "camera.field_of_view" => Self::FieldOfView,
            _ => return None,
        })
    }

    /// Command id that selects this tool.
    pub fn command(self) -> &'static str {
        match self {
            Self::Select => "tool.select",
            Self::Lasso => "tool.lasso",
            Self::Draw(id) | Self::Modify(id) | Self::Pick(id) => id,
            Self::Eraser => "tool.eraser",
            Self::Orbit => "camera.orbit",
            Self::Pan => "camera.pan",
            Self::Zoom => "camera.zoom",
            Self::ZoomWindow => "camera.zoom_window",
            Self::PositionCamera => "camera.position",
            Self::Walk => "camera.walk",
            Self::LookAround => "camera.look_around",
            Self::FieldOfView => "camera.field_of_view",
        }
    }

    /// Status bar hint. Draft wording; check against SketchUp.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Select => {
                "Select objects. Shift to extend select. Drag mouse to select multiple."
            }
            Self::Lasso => {
                "Drag a lasso: clockwise selects what it touches, counterclockwise what is inside. Click points, double-click to finish."
            }
            Self::Draw(_) | Self::Modify(_) | Self::Pick(_) => "",
            Self::Eraser => {
                "Select objects to erase. Shift = Hide, Ctrl = Soften/Smooth, Ctrl+Shift = Unsoften."
            }
            Self::Orbit => "Drag in direction to orbit. Shift = Pan, Ctrl = suspend gravity.",
            Self::Pan => "Drag in direction to pan.",
            Self::Zoom => "Drag up or down to zoom.",
            Self::ZoomWindow => "Drag a window around the area to zoom to.",
            Self::PositionCamera => {
                "Click to stand at a point, or drag from the eye point to the point to look at."
            }
            Self::Walk => "Drag to walk. Shift = move up/down and sideways, Ctrl = run.",
            Self::LookAround => "Drag to look around. Type an eye height.",
            Self::FieldOfView => "Drag up or down to change the field of view.",
        }
    }
}

/// Input for one frame, in physical pixels.
#[derive(Debug, Default, Clone, Copy)]
pub struct Input {
    pub cursor: Option<DVec2>,
    pub left_drag: DVec2,
    pub middle_drag: DVec2,
    pub shift: bool,
    /// Wheel notches; positive zooms in.
    pub wheel: f64,
}

pub fn apply(tool: Tool, camera: &mut Camera, input: &Input, viewport: DVec2) {
    if input.middle_drag != DVec2::ZERO {
        if input.shift {
            camera.pan(input.middle_drag, viewport);
        } else {
            camera.orbit(input.middle_drag);
            // Orbiting leaves Two-Point Perspective (unverified).
            camera.two_point = false;
        }
    }
    if input.left_drag != DVec2::ZERO {
        match tool {
            Tool::Select
            | Tool::Lasso
            | Tool::Draw(_)
            | Tool::Modify(_)
            | Tool::Pick(_)
            | Tool::Eraser
            | Tool::ZoomWindow
            | Tool::PositionCamera
            | Tool::Walk => {}
            Tool::Orbit if input.shift => camera.pan(input.left_drag, viewport),
            Tool::Orbit => {
                camera.orbit(input.left_drag);
                camera.two_point = false;
            }
            Tool::LookAround => camera.look_around(input.left_drag),
            // Dragging up narrows the view, as zooming in (unverified).
            Tool::FieldOfView => {
                camera.set_fov(camera.fov_deg + input.left_drag.y * FOV_DEG_PER_PX)
            }
            Tool::Pan => camera.pan(input.left_drag, viewport),
            // Shift held: the drag changes the field of view [doc].
            Tool::Zoom if input.shift => {
                camera.set_fov(camera.fov_deg + input.left_drag.y * FOV_DEG_PER_PX)
            }
            Tool::Zoom => camera.zoom_drag(input.left_drag.y, viewport),
        }
    }
    if input.wheel != 0.0 {
        let at = input.cursor.unwrap_or(viewport / 2.0);
        camera.zoom_at(input.wheel, at, viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn camera() -> Camera {
        Camera {
            eye: DVec3::new(-300.0, -500.0, 120.0),
            target: DVec3::ZERO,
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        }
    }

    const VIEWPORT: DVec2 = DVec2::new(800.0, 600.0);

    #[test]
    fn select_tool_ignores_left_drag() {
        let mut c = camera();
        let input = Input {
            left_drag: DVec2::new(10.0, 5.0),
            ..Default::default()
        };
        apply(Tool::Select, &mut c, &input, VIEWPORT);
        assert_eq!(c, camera());
    }

    #[test]
    fn middle_drag_orbits_and_shift_pans_in_any_tool() {
        let drag = DVec2::new(10.0, 5.0);
        let mut orbited = camera();
        apply(
            Tool::Select,
            &mut orbited,
            &Input {
                middle_drag: drag,
                ..Default::default()
            },
            VIEWPORT,
        );
        assert_eq!(orbited.target, camera().target);
        assert_ne!(orbited.eye, camera().eye);

        let mut panned = camera();
        apply(
            Tool::Select,
            &mut panned,
            &Input {
                middle_drag: drag,
                shift: true,
                ..Default::default()
            },
            VIEWPORT,
        );
        assert_ne!(panned.target, camera().target);
        let offset = panned.eye - panned.target;
        assert!((offset - (camera().eye - camera().target)).length() < 1e-9);
    }

    #[test]
    fn orbit_tool_left_drag_matches_middle_drag() {
        let drag = DVec2::new(-30.0, 12.0);
        let mut a = camera();
        let mut b = camera();
        apply(
            Tool::Orbit,
            &mut a,
            &Input {
                left_drag: drag,
                ..Default::default()
            },
            VIEWPORT,
        );
        apply(
            Tool::Select,
            &mut b,
            &Input {
                middle_drag: drag,
                ..Default::default()
            },
            VIEWPORT,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn wheel_zooms_in() {
        let mut c = camera();
        apply(
            Tool::Select,
            &mut c,
            &Input {
                wheel: 1.0,
                cursor: Some(VIEWPORT / 2.0),
                ..Default::default()
            },
            VIEWPORT,
        );
        assert!(c.distance() < camera().distance());
    }

    #[test]
    fn orbit_leaves_two_point() {
        let mut c = camera();
        c.two_point = true;
        let input = Input {
            left_drag: DVec2::new(10.0, 0.0),
            ..Default::default()
        };
        apply(Tool::Orbit, &mut c, &input, VIEWPORT);
        assert!(!c.two_point && c.perspective);
    }

    #[test]
    fn field_of_view_drag_up_narrows() {
        let mut c = camera();
        let input = Input {
            left_drag: DVec2::new(0.0, -50.0),
            ..Default::default()
        };
        apply(Tool::FieldOfView, &mut c, &input, VIEWPORT);
        assert!((c.fov_deg - 30.0).abs() < 1e-9);
        assert_eq!(c.eye, camera().eye);
    }

    #[test]
    fn camera_tool_commands_round_trip() {
        for id in [
            "camera.position",
            "camera.walk",
            "camera.look_around",
            "camera.field_of_view",
        ] {
            assert_eq!(Tool::from_command(id).unwrap().command(), id);
        }
    }
}
