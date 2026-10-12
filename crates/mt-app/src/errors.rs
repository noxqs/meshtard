//! Error messages shown in a dialog (loading or saving a file failed), besides the
//! status bar text.

use std::fmt::Display;
use std::path::Path;

use eframe::egui;

use crate::App;

/// An error waiting for the user to close its dialog.
pub(crate) struct ErrorMessage {
    heading: String,
    detail: String,
}

impl ErrorMessage {
    pub(crate) fn new(heading: String, detail: String) -> Self {
        Self { heading, detail }
    }
}

impl App {
    /// Shows `Could not <verb> "<file name>".` with the reason and the full path.
    pub(crate) fn file_error(&mut self, verb: &str, path: &Path, err: impl Display) {
        log::warn!("could not {verb} {}: {err}", path.display());
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        self.message = Some(format!("Could not {verb} {}: {err}", path.display()));
        let reason = capitalize(&err.to_string());
        self.error = Some(ErrorMessage {
            heading: format!("Could not {verb} \"{name}\"."),
            detail: format!("{reason}.\n\n{}", path.display()),
        });
    }

    /// The error dialog, while there is one.
    pub(crate) fn error_ui(&mut self, ctx: &egui::Context) {
        let Some(e) = &self.error else { return };
        let r = mt_ui::Dialog::new("error", "Meshtard")
            .width(400.0)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    error_icon(ui);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        mt_ui::dialog::message(ui, &e.heading, Some(&e.detail));
                    });
                });
                mt_ui::dialog::buttons(ui, &["OK"], Some(0))
            });
        if r.closed || r.inner.is_some() {
            self.error = None;
            mt_ui::dialog::reset(ctx, "error");
        }
    }
}

/// "not a valid file" -> "Not a valid file" (error texts from libraries start in
/// lower case).
fn capitalize(s: &str) -> String {
    let s = s.trim_end_matches('.');
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A red circle with a white cross, like the Windows error icon.
fn error_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    p.circle_filled(c, 15.0, egui::Color32::from_rgb(196, 43, 28));
    let s = egui::Stroke::new(2.5, egui::Color32::WHITE);
    let d = 6.0;
    p.line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], s);
    p.line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], s);
}

#[cfg(test)]
mod tests {
    #[test]
    fn capitalizes_reasons() {
        assert_eq!(super::capitalize("not valid."), "Not valid");
        assert_eq!(super::capitalize(""), "");
    }
}
