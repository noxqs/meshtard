//! File > New from Template and Save As Template.
//!
//! Templates are `.skp` files in `templates/` in the user config folder (see
//! `settings`), named by their file stem. The built-in template is the empty model
//! meshtard starts with. The default template (used by File > New) is kept in
//! `templates.toml` there as `default = "<name>"` (empty or missing: the built-in one);
//! "Set as default template" in either dialog changes it, as SketchUp's Preferences >
//! Template page does (SketchUp's template browser, thumbnails and descriptions are
//! not done; the start-up model is always the built-in template). A model opened from
//! a template is a new untitled model: Save asks for a file name.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::App;
use crate::confirm::Pending;
use crate::document::Document;

/// Label of the built-in template.
pub(crate) const BUILT_IN: &str = "Simple Template (built-in)";

pub(crate) fn templates_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("templates"))
}

fn default_file() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("templates.toml"))
}

/// The templates in `dir`: (name, path), sorted by name.
pub(crate) fn list_templates(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("skp")))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
        .collect();
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

/// A template file name from what the user typed: path separators and characters
/// Windows forbids become `_`; empty if nothing usable is left.
pub(crate) fn sanitize(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim_matches('.')
        .trim()
        .to_owned()
}

/// The default template's name in `templates.toml` text (`None`: built-in).
pub(crate) fn parse_default(text: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let name = table.get("default")?.as_str()?.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// The saved default template name.
pub(crate) fn load_default() -> Option<String> {
    parse_default(&std::fs::read_to_string(default_file()?).ok()?)
}

pub(crate) fn save_default(name: Option<&str>) {
    let Some(path) = default_file() else { return };
    let text = format!(
        "# File > New uses this template (empty: the built-in one).\ndefault = {}\n",
        toml::Value::String(name.unwrap_or_default().to_owned())
    );
    let result = std::fs::create_dir_all(path.parent().expect("has a parent"))
        .and_then(|()| std::fs::write(&path, text));
    if let Err(e) = result {
        log::warn!("could not save the default template: {e}");
    }
}

/// The default template's file, when it is set and exists.
pub(crate) fn default_template() -> Option<PathBuf> {
    let name = load_default()?;
    let path = templates_dir()?.join(format!("{name}.skp"));
    path.is_file().then_some(path)
}

/// Save As Template dialog fields.
#[derive(Debug, Clone, Default)]
pub(crate) struct SaveTemplate {
    name: String,
    make_default: bool,
    note: Option<String>,
}

/// New from Template dialog: the templates and the chosen one (`None`: built-in).
#[derive(Debug, Clone, Default)]
pub(crate) struct NewFromTemplate {
    list: Vec<(String, PathBuf)>,
    chosen: Option<usize>,
    make_default: bool,
}

impl App {
    pub(crate) fn open_save_template(&mut self) {
        self.present.save_template = Some(SaveTemplate {
            name: self.model_stem(),
            ..Default::default()
        });
    }

    pub(crate) fn open_new_from_template(&mut self) {
        let list = templates_dir().map_or_else(Vec::new, |d| list_templates(&d));
        let default = load_default();
        let chosen = default.and_then(|n| list.iter().position(|(m, _)| *m == n));
        self.present.new_template = Some(NewFromTemplate {
            list,
            chosen,
            make_default: false,
        });
    }

    /// File > New: a new model from the default template.
    pub(crate) fn new_document(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.new_from_template(ctx, frame, default_template());
    }

    /// A new untitled model from `template` (`None`: the built-in empty model).
    pub(crate) fn new_from_template(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        template: Option<PathBuf>,
    ) {
        let Some(path) = template else {
            self.new_document_builtin(ctx, frame);
            return;
        };
        let mut doc = match Document::open(&path) {
            Ok(doc) => doc,
            Err(e) => {
                self.file_error("open the template", &path, e);
                return;
            }
        };
        doc.path = PathBuf::from("Untitled.skp");
        doc.has_file = false;
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.install(ctx, &device, Path::new("Untitled.skp"), doc);
        }
    }

    pub(crate) fn templates_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.save_template_ui(ctx);
        self.new_template_ui(ctx, frame);
    }

    fn save_template_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.present.save_template.take() else {
            return;
        };
        let r = mt_ui::Dialog::new("save_template", "Save As Template")
            .width(340.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                egui::Grid::new("mt_save_template_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name:");
                        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(260.0));
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.checkbox(&mut d.make_default, "Set as default template");
                if let Some(dir) = templates_dir() {
                    ui.add_space(6.0);
                    ui.weak(format!("Saved in {}", dir.display()));
                }
                if let Some(n) = &d.note {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(180, 30, 30), n);
                }
                crate::presentation::ok_cancel(ui, "Save")
            });
        let done = crate::presentation::answer(r);
        match done {
            None => self.present.save_template = Some(d),
            Some(false) => {}
            Some(true) => {
                let name = sanitize(&d.name);
                let (Some(dir), false) = (templates_dir(), name.is_empty()) else {
                    d.note = Some("Enter a name.".into());
                    self.present.save_template = Some(d);
                    return;
                };
                let Some(doc) = &self.document else { return };
                let path = dir.join(format!("{name}.skp"));
                let result = std::fs::create_dir_all(&dir)
                    .map_err(|e| e.to_string())
                    .and_then(|()| doc.to_skp().map_err(|e| e.to_string()))
                    .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()));
                self.message = Some(match result {
                    Ok(()) => {
                        if d.make_default {
                            save_default(Some(&name));
                        }
                        format!("Saved template \"{name}\" ({}).", path.display())
                    }
                    Err(e) => format!("Could not save the template: {e}"),
                });
            }
        }
    }

    fn new_template_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut d) = self.present.new_template.take() else {
            return;
        };
        let default = load_default();
        let mut done = None;
        let r = mt_ui::Dialog::new("new_template", "New from Template")
            .width(340.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                let mark = |n: &str| {
                    if default.as_deref() == Some(n) {
                        format!("{n}  (default)")
                    } else {
                        n.to_owned()
                    }
                };
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        let builtin = if default.is_none() {
                            format!("{BUILT_IN}  (default)")
                        } else {
                            BUILT_IN.to_owned()
                        };
                        let r = ui.selectable_label(d.chosen.is_none(), builtin);
                        if r.clicked() {
                            d.chosen = None;
                        }
                        if r.double_clicked() {
                            done = Some(true);
                        }
                        for (i, (name, _)) in d.list.iter().enumerate() {
                            let r = ui.selectable_label(d.chosen == Some(i), mark(name));
                            if r.clicked() {
                                d.chosen = Some(i);
                            }
                            if r.double_clicked() {
                                done = Some(true);
                            }
                        }
                    });
                if d.list.is_empty() {
                    ui.add_space(4.0);
                    ui.weak("No saved templates yet: use File > Save As Template.");
                }
                ui.add_space(6.0);
                ui.checkbox(&mut d.make_default, "Set as default template");
                crate::presentation::ok_cancel(ui, "Create")
            });
        let done = done.or(crate::presentation::answer(r));
        match done {
            None => self.present.new_template = Some(d),
            Some(false) => {}
            Some(true) => {
                let chosen = d.chosen.and_then(|i| d.list.get(i).cloned());
                if d.make_default {
                    save_default(chosen.as_ref().map(|(n, _)| n.as_str()));
                }
                self.request(ctx, frame, Pending::NewFrom(chosen.map(|(_, p)| p)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_skp_files_by_name() {
        let dir = std::env::temp_dir().join(format!("mt_templates_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["Zeta.skp", "architectural.SKP", "notes.txt", "Beta.skp"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        std::fs::create_dir_all(dir.join("folder.skp")).unwrap();
        let names: Vec<String> = list_templates(&dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["architectural", "Beta", "Zeta"]);
        assert!(list_templates(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn names_and_default_setting() {
        assert_eq!(sanitize("  Plan: 1/50 "), "Plan_ 1_50");
        assert_eq!(sanitize(" .. "), "");
        assert_eq!(
            parse_default("default = \"Arch mm\"\n").as_deref(),
            Some("Arch mm")
        );
        assert_eq!(parse_default("default = \"\"\n"), None);
        assert_eq!(parse_default("not toml ["), None);
        let text = format!("default = {}\n", toml::Value::String("a \"b\"".into()));
        assert_eq!(parse_default(&text).as_deref(), Some("a \"b\""));
    }
}
