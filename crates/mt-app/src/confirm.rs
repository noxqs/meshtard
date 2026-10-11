//! "Save changes?" before New, Open, opening a dropped file and closing the window,
//! when the model has unsaved changes. SketchUp for Windows asks "Save changes to
//! "<name>"?" with Yes / No / Cancel (wording unverified).

use std::path::PathBuf;

use eframe::egui;

use crate::App;

/// What waits for the user's answer.
#[derive(Debug, Clone)]
pub enum Pending {
    New,
    /// Open this file, or ask for one.
    Open(Option<PathBuf>),
    Exit,
    /// File > Revert: asks "Revert?" instead of "Save changes?" (wording unverified).
    Revert,
}

impl App {
    /// Runs `action` now, or after asking to save when there are unsaved changes.
    pub(crate) fn request(&mut self, ctx: &egui::Context, frame: &eframe::Frame, action: Pending) {
        if self.document.as_ref().is_some_and(|d| d.is_modified()) {
            self.pending = Some(action);
        } else {
            self.perform(ctx, frame, action);
        }
    }

    fn perform(&mut self, ctx: &egui::Context, frame: &eframe::Frame, action: Pending) {
        match action {
            Pending::New => self.new_document(ctx, frame),
            Pending::Open(path) => {
                let path = path.or_else(|| {
                    rfd::FileDialog::new()
                        .add_filter("SketchUp Models", &["skp"])
                        .pick_file()
                });
                if let (Some(path), Some(rs)) = (path, frame.wgpu_render_state()) {
                    let device = rs.device.clone();
                    self.open(ctx, &device, &path);
                }
            }
            Pending::Exit => {
                self.closing = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::Revert => self.revert(ctx, frame),
        }
    }

    /// The window's close button: ask first when there are unsaved changes.
    pub(crate) fn intercept_close(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            if self.document.as_ref().is_some_and(|d| d.is_modified()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.request(ctx, frame, Pending::Exit);
            } else {
                self.closing = true;
            }
        }
    }

    /// The question, while an action waits.
    pub(crate) fn confirm_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(action) = self.pending.clone() else {
            return;
        };
        let name = self
            .document
            .as_ref()
            .and_then(|d| d.path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Untitled".into());
        let mut answer = None;
        if matches!(action, Pending::Revert) {
            egui::Modal::new(egui::Id::new("mt_revert")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label("meshtard");
                ui.add_space(6.0);
                ui.label(format!(
                    "Revert \"{name}\" to the last saved version? Unsaved changes will be lost."
                ));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Revert").clicked() {
                        answer = Some(Answer::No);
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        answer = Some(Answer::Cancel);
                    }
                });
            });
        } else {
            egui::Modal::new(egui::Id::new("mt_save_changes")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label("meshtard");
                ui.add_space(6.0);
                ui.label(format!("Save changes to \"{name}\"?"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        answer = Some(Answer::Yes);
                    }
                    if ui.button("No").clicked() {
                        answer = Some(Answer::No);
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        answer = Some(Answer::Cancel);
                    }
                });
            });
        }
        match answer {
            Some(Answer::Yes) => {
                self.pending = None;
                // A cancelled Save As keeps the model and drops the action.
                if self.save(ctx, "file.save") {
                    self.perform(ctx, frame, action);
                }
            }
            Some(Answer::No) => {
                self.pending = None;
                self.perform(ctx, frame, action);
            }
            Some(Answer::Cancel) => self.pending = None,
            None => {}
        }
    }
}

enum Answer {
    Yes,
    No,
    Cancel,
}
