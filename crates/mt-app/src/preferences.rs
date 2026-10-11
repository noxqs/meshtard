//! Window > Preferences: Drawing, General and Shortcuts pages, and auto-save.
//!
//! Shortcuts changed here are saved to `shortcuts.toml` in the user config folder
//! (next to `toolbars.toml`, see `settings`) as the differences from the built-in
//! keymap (`""` removes a shortcut), and applied at startup. The General page's
//! settings go to `preferences.toml`.
//!
//! From memory of SketchUp, all unverified: auto-save is on every 5 minutes and a
//! backup is made on save; the auto-saved copy here is `<name>~autosave.skp` next to
//! the model (SketchUp writes `AutoSave_<name>.skp` in a temporary folder), only for
//! models that have a file, and is removed when the model is saved; the backup is the
//! previous file renamed `<name>.skb`. The Drawing page's click style is shown but
//! cannot be changed: the drawing tools take click, move, click.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use mt_spec::{Shortcut, Spec};

use crate::App;

/// Preferences > General.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct General {
    pub autosave: bool,
    pub autosave_minutes: u32,
    pub backup: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            autosave: true,
            autosave_minutes: 5,
            backup: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Drawing,
    General,
    Shortcuts,
}

#[derive(Debug, Clone)]
struct Dialog {
    page: Page,
    filter: String,
    selected: Option<String>,
    /// Waiting for the keys of the selected command's new shortcut.
    capturing: bool,
    note: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Prefs {
    pub general: General,
    /// The built-in keymap (Reset All goes back to it).
    defaults: BTreeMap<String, String>,
    dialog: Option<Dialog>,
    /// Auto-save: since when the model has changes not auto-saved yet.
    dirty_since: Option<Instant>,
    /// The document state last auto-saved.
    autosaved: Option<u64>,
}

/// The shortcut table the app matches key presses against: implemented commands
/// with a shortcut whose key egui knows.
pub(crate) fn shortcut_table(
    spec: &Spec,
    implemented: &[&'static str],
) -> Vec<(Shortcut, egui::Key, &'static str)> {
    implemented
        .iter()
        .filter_map(|&id| {
            let shortcut = spec.keymap.shortcut(id)?;
            let key = egui::Key::from_name(&shortcut.key)?;
            Some((shortcut, key, id))
        })
        .collect()
}

/// Modifier keys, which egui also reports as key presses: a shortcut needs another
/// key.
fn is_modifier(key: egui::Key) -> bool {
    use egui::Key as K;
    matches!(
        key,
        K::ShiftLeft
            | K::ShiftRight
            | K::ControlLeft
            | K::ControlRight
            | K::AltLeft
            | K::AltRight
            | K::SuperLeft
            | K::SuperRight
    )
}

/// Reads `shortcuts.toml` text: command id to shortcut, `""` for removed.
pub(crate) fn parse_overrides(text: &str) -> Result<BTreeMap<String, String>, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let Some(bindings) = table.get("bindings") else {
        return Ok(BTreeMap::new());
    };
    let bindings = bindings.as_table().ok_or("`bindings` is not a table")?;
    bindings
        .iter()
        .map(|(id, v)| {
            let s = v.as_str().ok_or(format!("`{id}`: not a string"))?;
            if !s.is_empty() {
                Shortcut::parse(s).map_err(|e| format!("`{id}`: {e}"))?;
            }
            Ok((id.clone(), s.to_owned()))
        })
        .collect()
}

pub(crate) fn overrides_text(overrides: &BTreeMap<String, String>) -> String {
    let bindings: toml::Table = overrides
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
        .collect();
    let mut table = toml::Table::new();
    table.insert("bindings".into(), toml::Value::Table(bindings));
    format!(
        "# meshtard keyboard shortcuts that differ from the built-in ones (\"\" = none).\n{table}"
    )
}

/// The differences of `current` from `defaults`.
pub(crate) fn diff(
    defaults: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for id in defaults.keys().filter(|id| !current.contains_key(*id)) {
        out.insert(id.clone(), String::new());
    }
    for (id, s) in current {
        if defaults.get(id) != Some(s) {
            out.insert(id.clone(), s.clone());
        }
    }
    out
}

/// Gives `id` the shortcut `to` (none: removes it). A command that had the same
/// shortcut loses it; returns that command.
pub(crate) fn rebind(
    bindings: &mut BTreeMap<String, String>,
    id: &str,
    to: Option<&Shortcut>,
) -> Option<String> {
    let Some(to) = to else {
        bindings.remove(id);
        return None;
    };
    let other = bindings
        .iter()
        .find(|(k, v)| *k != id && Shortcut::parse(v).ok().as_ref() == Some(to))
        .map(|(k, _)| k.clone());
    if let Some(o) = &other {
        bindings.remove(o);
    }
    bindings.insert(id.to_owned(), to.to_string());
    other
}

/// Applies saved overrides to a keymap.
pub(crate) fn apply_overrides(
    bindings: &mut BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
) {
    for (id, s) in overrides {
        let shortcut = Shortcut::parse(s).ok();
        rebind(bindings, id, shortcut.as_ref());
    }
}

fn general_from_text(text: &str) -> General {
    let mut g = General::default();
    let Ok(t) = text.parse::<toml::Table>() else {
        return g;
    };
    if let Some(v) = t.get("autosave").and_then(toml::Value::as_bool) {
        g.autosave = v;
    }
    if let Some(v) = t.get("autosave_minutes").and_then(toml::Value::as_integer) {
        g.autosave_minutes = v.clamp(1, 999) as u32;
    }
    if let Some(v) = t.get("backup").and_then(toml::Value::as_bool) {
        g.backup = v;
    }
    g
}

fn general_text(g: &General) -> String {
    let mut t = toml::Table::new();
    t.insert("autosave".into(), g.autosave.into());
    t.insert(
        "autosave_minutes".into(),
        i64::from(g.autosave_minutes).into(),
    );
    t.insert("backup".into(), g.backup.into());
    t.to_string()
}

/// Where the model at `path` is auto-saved.
pub(crate) fn autosave_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    path.with_file_name(format!("{stem}~autosave.skp"))
}

/// The backup of the model at `path`, made when saving over it.
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("skb")
}

fn config_file(name: &str) -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join(name))
}

fn write_config(name: &str, text: &str) {
    let Some(path) = config_file(name) else {
        return;
    };
    let result = std::fs::create_dir_all(path.parent().expect("has a parent"))
        .and_then(|()| std::fs::write(&path, text));
    if let Err(e) = result {
        log::warn!("could not save {}: {e}", path.display());
    }
}

/// Loads saved preferences and applies saved shortcuts to `spec`'s keymap.
pub(crate) fn load(spec: &mut Spec) -> Prefs {
    let defaults = spec.keymap.bindings.clone();
    if let Some(text) = config_file("shortcuts.toml").and_then(|p| std::fs::read_to_string(p).ok())
    {
        match parse_overrides(&text) {
            Ok(o) => apply_overrides(&mut spec.keymap.bindings, &o),
            Err(e) => log::warn!("ignoring saved shortcuts: {e}"),
        }
    }
    let general = config_file("preferences.toml")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| general_from_text(&t))
        .unwrap_or_default();
    Prefs {
        general,
        defaults,
        ..Default::default()
    }
}

/// Before File > Save writes over `path`: keeps the old file as a backup.
pub(crate) fn before_save(prefs: &Prefs, path: &Path) {
    if prefs.general.backup
        && path.exists()
        && let Err(e) = std::fs::copy(path, backup_path(path))
    {
        log::warn!("could not back up {}: {e}", path.display());
    }
}

/// After the model was saved to `path`: its auto-saved copy is not needed.
pub(crate) fn after_save(path: &Path) {
    let _ = std::fs::remove_file(autosave_path(path));
}

impl App {
    pub(crate) fn open_preferences(&mut self) {
        self.prefs.dialog.get_or_insert(Dialog {
            page: Page::Shortcuts,
            filter: String::new(),
            selected: None,
            capturing: false,
            note: None,
        });
    }

    /// Applies a changed keymap: the shortcut table, menus (they read the spec) and
    /// the saved file.
    fn keymap_changed(&mut self) {
        self.shortcuts = shortcut_table(&self.spec, crate::IMPLEMENTED);
        let overrides = diff(&self.prefs.defaults, &self.spec.keymap.bindings);
        write_config("shortcuts.toml", &overrides_text(&overrides));
    }

    /// Auto-saves the model when it has had unsaved changes for the set time.
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        let g = &self.prefs.general;
        let Some(doc) = self.document.as_ref().filter(|d| g.autosave && d.has_file) else {
            self.prefs.dirty_since = None;
            return;
        };
        if !doc.is_modified() || self.prefs.autosaved == Some(doc.state_id()) {
            self.prefs.dirty_since = None;
            return;
        }
        let now = Instant::now();
        let since = *self.prefs.dirty_since.get_or_insert(now);
        let every = Duration::from_secs(u64::from(g.autosave_minutes.max(1)) * 60);
        let elapsed = now - since;
        if elapsed < every {
            ctx.request_repaint_after(every - elapsed);
            return;
        }
        let path = autosave_path(&doc.path);
        let result = doc
            .to_skp()
            .map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => log::info!("auto-saved to {}", path.display()),
            Err(e) => self.message = Some(format!("Could not auto-save {}: {e}", path.display())),
        }
        self.prefs.autosaved = Some(doc.state_id());
        self.prefs.dirty_since = None;
    }

    pub(crate) fn preferences_ui(&mut self, ctx: &egui::Context) {
        self.autosave_tick(ctx);
        let Some(mut d) = self.prefs.dialog.take() else {
            return;
        };
        let mut general = self.prefs.general.clone();
        let mut keymap_changed = false;
        let capturing = d.capturing;
        let r = mt_ui::Dialog::new("preferences", "Preferences")
            .width(560.0)
            .modal(false)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                // A fixed-height body: the page list on the left, the page on the right.
                let size = egui::vec2(ui.available_width(), 360.0);
                let layout = egui::Layout::left_to_right(egui::Align::Min);
                ui.allocate_ui_with_layout(size, layout, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(100.0);
                        for (p, label) in [
                            (Page::Drawing, "Drawing"),
                            (Page::General, "General"),
                            (Page::Shortcuts, "Shortcuts"),
                        ] {
                            if ui.selectable_label(d.page == p, label).clicked() {
                                d.page = p;
                                d.capturing = false;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| match d.page {
                        Page::Drawing => {
                            ui.strong("Click Style");
                            ui.add_space(4.0);
                            ui.add_enabled_ui(false, |ui| {
                                for (label, on) in [
                                    ("Click-drag-click", false),
                                    ("Auto detect", false),
                                    ("Click-click", true),
                                    ("Click-drag", false),
                                ] {
                                    let _ = ui.radio(on, label);
                                }
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(
                                    "The drawing tools take click, move, click; other styles \
                                     are not supported yet.",
                                )
                                .small()
                                .weak(),
                            );
                        }
                        Page::General => {
                            ui.strong("Saving");
                            ui.add_space(4.0);
                            ui.checkbox(&mut general.backup, "Create backup");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut general.autosave, "Auto-save every");
                                ui.add_enabled(
                                    general.autosave,
                                    egui::DragValue::new(&mut general.autosave_minutes)
                                        .range(1..=999),
                                );
                                ui.label("minutes");
                            });
                        }
                        Page::Shortcuts => {
                            keymap_changed |= self.shortcuts_page(ui, &mut d);
                        }
                    });
                });
            });
        // Escape while recording a shortcut cancels the recording, not the dialog.
        let open = !r.closed || (capturing && ctx.input(|i| i.key_pressed(egui::Key::Escape)));
        if general != self.prefs.general {
            write_config("preferences.toml", &general_text(&general));
            self.prefs.general = general;
        }
        if keymap_changed {
            self.keymap_changed();
        }
        if open {
            self.prefs.dialog = Some(d);
        }
    }

    /// The Shortcuts page. Returns whether the keymap changed.
    fn shortcuts_page(&mut self, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut d.filter);
        });
        let filter = d.filter.to_lowercase();
        let commands = self.spec.commands();
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("mt_pref_shortcuts")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (id, label) in &commands {
                            if !filter.is_empty()
                                && !label.to_lowercase().contains(&filter)
                                && !id.contains(&filter)
                            {
                                continue;
                            }
                            let selected = d.selected.as_deref() == Some(id.as_str());
                            let text = format!("{label}  ({id})");
                            if ui.selectable_label(selected, text).clicked() {
                                d.selected = Some(id.clone());
                                d.capturing = false;
                            }
                            ui.label(self.spec.keymap.bindings.get(id).map_or("", String::as_str));
                            ui.end_row();
                        }
                    });
            });
        ui.separator();
        let Some(id) = d.selected.clone() else {
            ui.label("Select a command to change its shortcut.");
            if ui.button("Reset All").clicked() {
                self.spec.keymap.bindings = self.prefs.defaults.clone();
                changed = true;
            }
            return changed;
        };
        ui.horizontal(|ui| {
            let label = self.spec.command_label(&id).unwrap_or_else(|| id.clone());
            ui.label(format!("{label}:"));
            let current = self
                .spec
                .keymap
                .bindings
                .get(&id)
                .cloned()
                .unwrap_or_default();
            let text = if d.capturing {
                "Press keys...".to_owned()
            } else if current.is_empty() {
                "(none)".to_owned()
            } else {
                current
            };
            let r = ui.button(text);
            if r.clicked() {
                d.capturing = true;
                d.note = None;
            }
            if d.capturing {
                // Focus keeps the app's shortcuts and Measurements typing away from
                // the keys pressed.
                r.request_focus();
                let pressed = ui.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } if !is_modifier(*key) => Some((*key, *modifiers)),
                        _ => None,
                    })
                });
                match pressed {
                    Some((egui::Key::Escape, m)) if !m.any() => d.capturing = false,
                    Some((key, m)) => {
                        let s = Shortcut {
                            ctrl: m.command,
                            shift: m.shift,
                            alt: m.alt,
                            key: key.name().to_owned(),
                        };
                        if let Some(other) = rebind(&mut self.spec.keymap.bindings, &id, Some(&s)) {
                            let name = self.spec.command_label(&other).unwrap_or(other);
                            d.note = Some(format!("{s} was removed from {name}."));
                        }
                        d.capturing = false;
                        r.surrender_focus();
                        changed = true;
                    }
                    None => {}
                }
            }
            if ui.button("Remove").clicked() {
                rebind(&mut self.spec.keymap.bindings, &id, None);
                changed = true;
            }
            if ui.button("Reset All").clicked() {
                self.spec.keymap.bindings = self.prefs.defaults.clone();
                changed = true;
            }
        });
        if let Some(note) = &d.note {
            ui.label(egui::RichText::new(note).small());
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn overrides_round_trip_through_text() {
        let o = map(&[("tool.line", "Ctrl+Shift+L"), ("tool.move", "")]);
        let text = overrides_text(&o);
        assert_eq!(parse_overrides(&text).unwrap(), o);
        assert!(parse_overrides("[bindings]\n\"tool.line\" = \"Hyper+L\"").is_err());
        assert!(parse_overrides("").unwrap().is_empty());
    }

    #[test]
    fn rebinding_moves_a_taken_shortcut() {
        let defaults = map(&[("tool.line", "L"), ("tool.move", "M"), ("tool.eraser", "E")]);
        let mut b = defaults.clone();
        let m = Shortcut::parse("M").unwrap();
        assert_eq!(
            rebind(&mut b, "tool.line", Some(&m)),
            Some("tool.move".into())
        );
        assert_eq!(b, map(&[("tool.line", "M"), ("tool.eraser", "E")]));
        rebind(&mut b, "tool.eraser", None);
        let o = diff(&defaults, &b);
        assert_eq!(
            o,
            map(&[("tool.line", "M"), ("tool.move", ""), ("tool.eraser", "")])
        );
        // Applying the saved differences to the defaults gives the same keymap.
        let mut again = defaults.clone();
        apply_overrides(&mut again, &o);
        assert_eq!(again, b);
    }

    #[test]
    fn general_round_trips() {
        let g = General {
            autosave: false,
            autosave_minutes: 12,
            backup: false,
        };
        assert_eq!(general_from_text(&general_text(&g)), g);
        assert_eq!(general_from_text("nonsense ="), General::default());
    }

    #[test]
    fn autosave_and_backup_names() {
        let p = Path::new("/models/house.skp");
        assert_eq!(autosave_path(p), Path::new("/models/house~autosave.skp"));
        assert_eq!(backup_path(p), Path::new("/models/house.skb"));
    }
}
