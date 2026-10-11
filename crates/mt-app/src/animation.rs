//! View > Animation: Add, Update and Delete Scene, Previous and Next Scene, Play and
//! Settings. The scene commands do what the Scenes panel's buttons do (see `tray`).
//!
//! Play activates the scenes one after another, looping, each with the scene
//! transition and then the scene delay; Escape or any click stops it. Settings is a
//! dialog with SketchUp's Model Info > Animation fields: "Enable scene transitions"
//! with the transition time, and the scene delay. Defaults (on, 2 s, 1 s) are from
//! memory of SketchUp (unverified); they are app settings here, not saved in the file
//! (SketchUp keeps them in the model). SketchUp also shows a small Pause / Stop window
//! while playing; not done.

use eframe::egui;

use crate::App;

/// Animation settings, the open Settings dialog and the playback state.
#[derive(Debug, Clone)]
pub(crate) struct Animation {
    pub transitions: bool,
    pub transition_secs: f64,
    pub delay_secs: f64,
    /// The Settings dialog's fields while it is open.
    dialog: Option<(bool, f64, f64)>,
    /// Playing: egui time when the next scene starts, once the current transition
    /// has ended.
    playing: Option<Option<f64>>,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            transitions: true,
            transition_secs: 2.0,
            delay_secs: 1.0,
            dialog: None,
            playing: None,
        }
    }
}

impl Animation {
    /// Duration of a scene transition (0 when transitions are off).
    pub(crate) fn transition_secs(&self) -> f64 {
        if self.transitions {
            self.transition_secs.max(0.0)
        } else {
            0.0
        }
    }

    pub(crate) fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Stops playing (File > Export > Animation takes over the camera).
    pub(crate) fn stop(&mut self) {
        self.playing = None;
    }
}

/// The scene after (`step` 1) or before (`step` -1) `current` among `count`, wrapping;
/// with no current scene, the first or the last.
pub(crate) fn step_scene(current: Option<usize>, count: usize, step: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current {
        Some(i) => (i as isize + step).rem_euclid(count as isize) as usize,
        None if step < 0 => count - 1,
        None => 0,
    })
}

impl App {
    /// Runs a `view.anim_*` command. Returns whether the scene must be rebuilt.
    pub(crate) fn animation_command(&mut self, id: &str) -> bool {
        let count = self.document.as_ref().map_or(0, |d| d.model.pages.len());
        let active = self.active_scene.filter(|&i| i < count);
        match id {
            "view.anim_add_scene" => self.tray_event(mt_ui::Event::SceneAdd),
            "view.anim_update_scene" => {
                active.is_some_and(|i| self.tray_event(mt_ui::Event::SceneUpdate(i)))
            }
            "view.anim_delete_scene" => {
                active.is_some_and(|i| self.tray_event(mt_ui::Event::SceneDelete(i)))
            }
            "view.anim_previous" | "view.anim_next" => {
                let step = if id == "view.anim_next" { 1 } else { -1 };
                step_scene(active, count, step)
                    .is_some_and(|i| self.tray_event(mt_ui::Event::SceneActivate(i)))
            }
            "view.anim_play" => {
                // Scenes left out of the animation (Scenes panel) are skipped.
                let first = self
                    .document
                    .as_ref()
                    .and_then(|d| crate::tray_more::step_animated(&d.model.pages, active, 1));
                let Some(first) = first else {
                    self.message = Some("Add scenes to play an animation.".into());
                    return false;
                };
                self.animation.playing = Some(None);
                self.tray_event(mt_ui::Event::SceneActivate(first))
            }
            "view.anim_settings" => {
                let a = &self.animation;
                self.animation.dialog = Some((a.transitions, a.transition_secs, a.delay_secs));
                false
            }
            _ => false,
        }
    }

    /// Per frame: the Settings dialog and playback.
    pub(crate) fn animation_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.animation_settings_ui(ctx);
        let Some(next_at) = self.animation.playing else {
            return;
        };
        // Escape or a click anywhere stops playing where the camera is.
        let stop = ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.any_pressed());
        let count = self.document.as_ref().map_or(0, |d| d.model.pages.len());
        if stop || count == 0 {
            self.animation.playing = None;
            self.transition = None;
            return;
        }
        if self.transition.is_some() {
            return; // animate_camera repaints until the move ends
        }
        let now = ctx.input(|i| i.time);
        match next_at {
            None => {
                let at = now + self.animation.delay_secs.max(0.0);
                self.animation.playing = Some(Some(at));
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    self.animation.delay_secs.max(0.0),
                ));
            }
            Some(at) if now >= at => {
                self.animation.playing = Some(None);
                let next = self.document.as_ref().and_then(|d| {
                    crate::tray_more::step_animated(&d.model.pages, self.active_scene, 1)
                });
                if let Some(i) = next
                    && self.tray_event(mt_ui::Event::SceneActivate(i))
                    && let Some(rs) = frame.wgpu_render_state()
                {
                    let device = rs.device.clone();
                    self.rebuild(&device);
                }
                ctx.request_repaint();
            }
            Some(at) => {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(at - now));
            }
        }
    }

    fn animation_settings_ui(&mut self, ctx: &egui::Context) {
        let Some((mut on, mut secs, mut delay)) = self.animation.dialog else {
            return;
        };
        let r = mt_ui::Dialog::new("animation_settings", "Animation Settings")
            .width(300.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                ui.strong("Scene Transitions");
                ui.add_space(4.0);
                ui.checkbox(&mut on, "Enable scene transitions");
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        on,
                        egui::DragValue::new(&mut secs)
                            .range(0.0..=60.0)
                            .speed(0.1)
                            .max_decimals(1),
                    );
                    ui.label("seconds");
                });
                ui.add_space(10.0);
                ui.strong("Scene Delay");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut delay)
                            .range(0.0..=60.0)
                            .speed(0.1)
                            .max_decimals(1),
                    );
                    ui.label("seconds");
                });
                crate::presentation::ok_cancel(ui, "OK")
            });
        let done = crate::presentation::answer(r);
        match done {
            None => self.animation.dialog = Some((on, secs, delay)),
            Some(ok) => {
                self.animation.dialog = None;
                if ok {
                    self.animation.transitions = on;
                    self.animation.transition_secs = secs;
                    self.animation.delay_secs = delay;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_step_and_wrap() {
        assert_eq!(step_scene(None, 0, 1), None);
        assert_eq!(step_scene(None, 3, 1), Some(0));
        assert_eq!(step_scene(None, 3, -1), Some(2));
        assert_eq!(step_scene(Some(2), 3, 1), Some(0));
        assert_eq!(step_scene(Some(0), 3, -1), Some(2));
        assert_eq!(step_scene(Some(1), 3, 1), Some(2));
    }

    #[test]
    fn transitions_off_jump() {
        let mut a = Animation::default();
        assert_eq!(a.transition_secs(), 2.0);
        a.transitions = false;
        assert_eq!(a.transition_secs(), 0.0);
    }
}
