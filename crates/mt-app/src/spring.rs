//! Spring-loaded camera tools (a Meshtard addition, not SketchUp behaviour): tapping
//! O, H or Z switches to Orbit, Pan or Zoom as before; holding the key uses the tool
//! only while it is down, and releasing it returns to the previous tool with its
//! operation in progress (a line's first click, a selection, ...). A key counts as
//! held when the camera moved while it was down or it was down for `HOLD_SECS`.

use eframe::egui;
use mt_render::Camera;

use crate::nav::Tool;
use crate::{App, modify, pick_tool, tools};

/// Commands whose shortcut is spring-loaded.
pub(crate) const SPRING_COMMANDS: &[&str] = &["camera.orbit", "camera.pan", "camera.zoom"];
/// Longest press that still counts as a tap.
const HOLD_SECS: f64 = 0.3;

/// The tool to return to and the key being held.
pub(crate) struct Spring {
    key: egui::Key,
    since: f64,
    camera: Camera,
    /// The spring tool, so a tool picked another way meanwhile is not undone.
    tool: Tool,
    prev_tool: Tool,
    prev_draw: Option<Box<dyn tools::DrawTool>>,
    prev_modify: Option<Box<dyn modify::ModifyTool>>,
    prev_pick: Option<pick_tool::PickTool>,
}

/// What happens when the key goes up.
fn release_returns(held_secs: f64, camera_moved: bool) -> bool {
    camera_moved || held_secs >= HOLD_SECS
}

impl App {
    /// Called for a shortcut press. Returns false when the press is an auto-repeat of
    /// the key already held, which must not switch tools again.
    pub(crate) fn spring_press(&mut self, ctx: &egui::Context, id: &str, key: egui::Key) -> bool {
        if self.spring.as_ref().is_some_and(|s| s.key == key) {
            return false;
        }
        if SPRING_COMMANDS.contains(&id)
            && let Some(tool) = Tool::from_command(id)
            && self.tool != tool
        {
            self.spring = Some(Spring {
                key,
                since: ctx.input(|i| i.time),
                camera: self.camera.clone(),
                tool,
                prev_tool: self.tool,
                prev_draw: self.draw.take(),
                prev_modify: self.modify.take(),
                prev_pick: self.pick.take(),
            });
        } else {
            self.spring = None;
        }
        true
    }

    /// Each frame: on release of a held key, go back to the previous tool.
    pub(crate) fn spring_update(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.spring else { return };
        let (down, now) = ctx.input(|i| (i.key_down(s.key), i.time));
        if down {
            // Keep checking until the key goes up.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            return;
        }
        let s = self.spring.take().expect("checked above");
        if self.tool == s.tool && release_returns(now - s.since, self.camera != s.camera) {
            self.tool = s.prev_tool;
            self.draw = s.prev_draw;
            self.modify = s.prev_modify;
            self.pick = s.prev_pick;
            self.engine.unlock();
            self.inference = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_stays_hold_returns() {
        assert!(!release_returns(0.1, false), "a tap keeps the camera tool");
        assert!(release_returns(0.1, true), "using it while held returns");
        assert!(release_returns(0.5, false), "a long press returns");
    }
}
