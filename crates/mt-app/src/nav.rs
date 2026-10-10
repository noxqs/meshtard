//! Viewport navigation: the Orbit / Pan / Zoom tools and the mouse shortcuts that
//! work in every tool (middle drag, shift + middle drag, wheel).

use glam::DVec2;
use mt_render::Camera;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    /// A drawing tool from `tools`, by command id.
    Draw(&'static str),
    Eraser,
    Orbit,
    Pan,
    Zoom,
}

impl Tool {
    pub fn from_command(id: &str) -> Option<Self> {
        Some(match id {
            "tool.select" => Self::Select,
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
            "camera.orbit" => Self::Orbit,
            "camera.pan" => Self::Pan,
            "camera.zoom" => Self::Zoom,
            _ => return None,
        })
    }

    /// Command id that selects this tool.
    pub fn command(self) -> &'static str {
        match self {
            Self::Select => "tool.select",
            Self::Draw(id) => id,
            Self::Eraser => "tool.eraser",
            Self::Orbit => "camera.orbit",
            Self::Pan => "camera.pan",
            Self::Zoom => "camera.zoom",
        }
    }

    /// Status bar hint. Draft wording; check against SketchUp.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Select => {
                "Select objects. Shift to extend select. Drag mouse to select multiple."
            }
            Self::Draw(_) => "",
            Self::Eraser => {
                "Select objects to erase. Shift = Hide, Ctrl = Soften/Smooth, Ctrl+Shift = Unsoften."
            }
            Self::Orbit => "Drag in direction to orbit. Shift = Pan, Ctrl = suspend gravity.",
            Self::Pan => "Drag in direction to pan.",
            Self::Zoom => "Drag up or down to zoom.",
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
        }
    }
    if input.left_drag != DVec2::ZERO {
        match tool {
            Tool::Select | Tool::Draw(_) | Tool::Eraser => {}
            Tool::Orbit if input.shift => camera.pan(input.left_drag, viewport),
            Tool::Orbit => camera.orbit(input.left_drag),
            Tool::Pan => camera.pan(input.left_drag, viewport),
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
}
