//! Camera menu commands: Zoom Extents, Standard Views, projection, and Previous /
//! Next through the camera history.

use glam::{DVec2, DVec3};
use mt_render::{Camera, StandardView};

use eframe::egui;

use crate::App;

/// Camera views to go back and forward through (Camera > Previous / Next).
#[derive(Default)]
pub(crate) struct History {
    back: Vec<Camera>,
    forward: Vec<Camera>,
    /// The camera moved during the last frame (one drag or wheel spin is one entry).
    moving: bool,
}

/// How many views Previous can go back (SketchUp's limit is unverified).
const MAX_HISTORY: usize = 50;

impl History {
    /// Records `before` when a camera move starts.
    pub(crate) fn push(&mut self, before: Camera) {
        if self.back.last() != Some(&before) {
            self.back.push(before);
            if self.back.len() > MAX_HISTORY {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    /// Called once a frame with the camera before and after navigation.
    pub(crate) fn track(&mut self, before: &Camera, after: &Camera) {
        let moved = before != after;
        if moved && !self.moving {
            self.push(before.clone());
        }
        self.moving = moved;
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// The box around everything drawn, or `None` for an empty model.
pub(crate) fn scene_bounds(scene: &crate::document::Scene) -> Option<(DVec3, DVec3)> {
    let points = scene
        .edges
        .iter()
        .flat_map(|s| [s.a, s.b])
        .chain(scene.vertices.iter().map(|v| v.position));
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    for p in points {
        let p = DVec3::new(p[0].into(), p[1].into(), p[2].into());
        min = min.min(p);
        max = max.max(p);
    }
    (min.x <= max.x).then_some((min, max))
}

pub(crate) fn is_checked(camera: &Camera, id: &str) -> bool {
    match id {
        // A radio group: exactly one of the three is checked.
        "camera.parallel" => !camera.perspective,
        "camera.perspective" => camera.perspective && !camera.two_point,
        "camera.two_point" => camera.perspective && camera.two_point,
        _ => false,
    }
}

impl App {
    /// Runs a Camera menu command; false if `id` is not one.
    pub(crate) fn camera_command(&mut self, id: &str) -> bool {
        let aspect = aspect(self.viewport_px);
        let bounds = scene_bounds(&self.scene);
        let current = self.camera.clone();
        let to = match id {
            "camera.zoom_extents" => {
                let mut c = current.clone();
                if let Some((min, max)) = bounds {
                    c.fit(min, max, aspect);
                }
                c
            }
            // Fits the selection like Zoom Extents fits the model (context menu).
            "camera.zoom_selection" => {
                let items = self.selection.items();
                let Some((min, max)) = crate::context_menu::selection_bounds(&self.scene, items)
                else {
                    return true;
                };
                let mut c = current.clone();
                c.fit(min, max, aspect);
                c
            }
            "camera.parallel" => Camera {
                perspective: false,
                two_point: false,
                ..current.clone()
            },
            "camera.perspective" => Camera {
                perspective: true,
                two_point: false,
                ..current.clone()
            },
            "camera.two_point" => Camera {
                perspective: true,
                two_point: true,
                ..current.clone()
            },
            "camera.previous" => {
                let Some(c) = self.camera_history.back.pop() else {
                    return true;
                };
                self.camera_history.forward.push(current);
                self.go(c);
                return true;
            }
            "camera.next" => {
                let Some(c) = self.camera_history.forward.pop() else {
                    return true;
                };
                self.camera_history.back.push(current);
                self.go(c);
                return true;
            }
            _ => {
                let Some(view) = StandardView::from_command(id) else {
                    return false;
                };
                // A standard view also fits the model, as SketchUp's do (unverified).
                // Standard views leave Two-Point Perspective (unverified).
                let mut c = Camera {
                    two_point: false,
                    ..current.looking(view)
                };
                if let Some((min, max)) = bounds {
                    c.fit(min, max, aspect);
                }
                c
            }
        };
        if to != current {
            self.camera_history.push(current);
            self.go(to);
        }
        true
    }

    /// The Zoom Window tool: drag a box; on release the view zooms to it (one
    /// history step). The box is drawn solid in both drag directions.
    pub(crate) fn zoom_window_tool(
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
        if response.drag_started_by(primary) {
            self.zoom_box = ui.input(|i| i.pointer.press_origin()).map(to_px);
        }
        if response.drag_stopped_by(primary)
            && let (Some(start), Some(end)) = (
                self.zoom_box.take(),
                response.interact_pointer_pos().map(to_px),
            )
        {
            let before = self.camera.clone();
            self.camera.zoom_window(start, end, viewport_px);
            if self.camera != before {
                self.camera_history.push(before);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.zoom_box = None;
        }
    }

    /// Draws the Zoom Window box over the rendered view.
    pub(crate) fn draw_zoom_box(&self, ui: &egui::Ui, response: &egui::Response, rect: egui::Rect) {
        if let (Some(_), Some(start), Some(end)) = (
            self.zoom_box,
            ui.input(|i| i.pointer.press_origin()),
            response.interact_pointer_pos(),
        ) {
            let r = egui::Rect::from_two_pos(start, end).intersect(rect);
            let stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
            ui.painter()
                .rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
        }
    }

    /// Moves to `to` with the scene transition animation; a projection change is
    /// immediate.
    pub(crate) fn go(&mut self, to: Camera) {
        let projection_only =
            to.perspective != self.camera.perspective || to.two_point != self.camera.two_point;
        if projection_only && to.eye == self.camera.eye {
            self.camera = to;
            self.transition = None;
        } else {
            self.transition = Some(crate::tray::Transition::new(self.camera.clone(), to));
        }
    }
}

fn aspect(viewport: DVec2) -> f64 {
    if viewport.y > 0.0 {
        viewport.x / viewport.y
    } else {
        16.0 / 9.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(x: f64) -> Camera {
        Camera {
            eye: DVec3::new(x, -100.0, 50.0),
            target: DVec3::ZERO,
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        }
    }

    #[test]
    fn one_move_is_one_entry() {
        let mut h = History::default();
        h.track(&cam(0.0), &cam(1.0));
        h.track(&cam(1.0), &cam(2.0));
        h.track(&cam(2.0), &cam(2.0));
        h.track(&cam(2.0), &cam(3.0));
        assert_eq!(h.back, [cam(0.0), cam(2.0)]);
    }

    #[test]
    fn a_new_move_drops_forward_views() {
        let mut h = History::default();
        h.track(&cam(0.0), &cam(1.0));
        h.forward.push(cam(5.0));
        h.track(&cam(1.0), &cam(1.0));
        h.track(&cam(1.0), &cam(2.0));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn projection_checks_are_a_radio_group() {
        let ids = ["camera.parallel", "camera.perspective", "camera.two_point"];
        for (perspective, two_point) in [(false, false), (false, true), (true, false), (true, true)]
        {
            let c = Camera {
                perspective,
                two_point,
                ..cam(0.0)
            };
            assert_eq!(ids.iter().filter(|id| is_checked(&c, id)).count(), 1);
        }
    }
}
