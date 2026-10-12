//! The command list of Window > Preferences > Shortcuts and Help > Keyboard Shortcuts.
//!
//! Preferences > Shortcuts in SketchUp for Windows lists "every command or tool you
//! can assign a shortcut to" by Function (e.g. `Camera/Zoom Extents`), with the key
//! under Assigned (help.sketchup.com/en/creating-and-editing-shortcuts [doc]).
//! [`rows`] gives that list: the menu bar, the right-click menus and the commands of
//! loaded extensions, named by their menu path.
//!
//! Help > Keyboard Shortcuts is this app's own (SketchUp has no such list; its
//! nearest are that table and the Quick Reference Card PDF): every shortcut grouped
//! by top-level menu, with a filter, and Copy / Save as text for printing.

use eframe::egui;
use mt_spec::{Shortcut, Spec};

use crate::App;
use crate::preferences::Bindings;

/// A command a shortcut can be given to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub id: String,
    /// SketchUp's Function name: menu path and label, e.g. `Camera/Standard Views/Top`.
    pub function: String,
    /// Top-level menu, `Context menu` or the extension's menu.
    pub group: String,
}

/// Every command of the menu bar and the right-click menus, in menu order, then
/// `extensions`: (id, label, menu path).
pub(crate) fn rows(spec: &Spec, extensions: &[(String, String, Vec<String>)]) -> Vec<Row> {
    let mut out: Vec<Row> = crate::search::entries(spec)
        .into_iter()
        .map(|e| {
            let group = e.path.split(" > ").next().unwrap_or_default().to_owned();
            Row {
                function: format!("{}/{}", e.path.replace(" > ", "/"), e.label),
                id: e.id,
                group,
            }
        })
        .collect();
    for (id, label, menu) in extensions {
        if out.iter().any(|r| &r.id == id) {
            continue;
        }
        let path = if menu.is_empty() {
            "Extensions".to_owned()
        } else {
            menu.join("/")
        };
        out.push(Row {
            id: id.clone(),
            function: format!("{path}/{label}"),
            group: menu.first().cloned().unwrap_or_else(|| "Extensions".into()),
        });
    }
    // Extension commands join their menu's group (stable: menu order stays).
    let mut order: Vec<String> = Vec::new();
    for r in &out {
        if !order.contains(&r.group) {
            order.push(r.group.clone());
        }
    }
    out.sort_by_key(|r| order.iter().position(|g| g == &r.group));
    out
}

/// The shortcuts of `id` as shown to people (`Ctrl+,`), main one first.
pub(crate) fn key_labels(bindings: &Bindings, id: &str) -> Vec<String> {
    (bindings.get(id).into_iter().flatten())
        .filter_map(|s| Shortcut::parse(s).ok())
        .map(|s| s.label())
        .collect()
}

/// The shortcuts of `id` in one text (`Ctrl+C  Ctrl+Insert`), empty when none.
pub(crate) fn key_label(bindings: &Bindings, id: &str) -> String {
    key_labels(bindings, id).join("  ")
}

/// Whether `row` matches the Filter text (lowercase): function, id or one of the
/// keys.
pub(crate) fn matches(row: &Row, keys: &[String], filter: &str) -> bool {
    filter.is_empty()
        || row.function.to_lowercase().contains(filter)
        || row.id.contains(filter)
        || keys.iter().any(|k| k.to_lowercase() == filter)
}

/// The printable list: a heading per group, then function and keys per line. Only
/// commands with a shortcut unless `all`.
pub(crate) fn text(rows: &[Row], bindings: &Bindings, all: bool) -> String {
    let width = rows.iter().map(|r| r.function.len()).max().unwrap_or(0) + 2;
    let mut out = String::from("Meshtard keyboard shortcuts\n");
    let mut group = None;
    for r in rows {
        let key = key_label(bindings, &r.id);
        if key.is_empty() && !all {
            continue;
        }
        if group != Some(&r.group) {
            out.push_str(&format!("\n{}\n", r.group));
            group = Some(&r.group);
        }
        out.push_str(&format!("  {:width$}{key}\n", r.function));
    }
    out
}

/// The Help > Keyboard Shortcuts window's state.
#[derive(Debug, Default)]
pub(crate) struct ListDialog {
    pub filter: String,
    /// Also list commands without a shortcut.
    pub all: bool,
}

impl App {
    /// The commands of loaded extensions: (id, label, menu path).
    pub(crate) fn extension_commands(&self) -> Vec<(String, String, Vec<String>)> {
        self.extensions
            .command_ids()
            .into_iter()
            .filter_map(|id| {
                let (_, c) = self.extensions.command(&id)?;
                Some((id, c.label, c.menu))
            })
            .collect()
    }

    /// Every command a shortcut can be given to (see [`rows`]).
    pub(crate) fn shortcut_rows(&self) -> Vec<Row> {
        rows(&self.spec, &self.extension_commands())
    }

    /// Help > Context Help (Shift+F1, FEATURES.md 74). SketchUp's opens the help
    /// page of the active tool ("Sketchup/Context Help" in a Windows shortcut export
    /// [inf]; its behavior is from memory, unverified); Meshtard shows its own help
    /// instead: the Instructor panel, which follows the active tool, opened and
    /// expanded in the tray.
    pub(crate) fn context_help(&mut self) {
        if let Some(shell) = &mut self.shell {
            shell.open_panel("window.instructor");
        }
    }

    pub(crate) fn open_shortcut_list(&mut self) {
        self.prefs
            .shortcut_list
            .get_or_insert_with(Default::default);
    }

    /// Help > Keyboard Shortcuts while open.
    pub(crate) fn shortcut_list_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.prefs.shortcut_list.take() else {
            return;
        };
        let rows = self.shortcut_rows();
        let bindings = &self.spec.keymap.bindings;
        let mut save = false;
        let r = mt_ui::Dialog::new("keyboard_shortcuts", "Keyboard Shortcuts")
            .width(480.0)
            .modal(false)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut d.filter);
                });
                ui.checkbox(&mut d.all, "Also list commands without a shortcut");
                let filter = d.filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("mt_shortcut_list")
                            .num_columns(2)
                            .spacing([24.0, 3.0])
                            .show(ui, |ui| {
                                let mut group = None;
                                for r in &rows {
                                    let keys = key_labels(bindings, &r.id);
                                    if (keys.is_empty() && !d.all) || !matches(r, &keys, &filter) {
                                        continue;
                                    }
                                    let key = keys.join("  ");
                                    if group != Some(&r.group) {
                                        ui.strong(&r.group);
                                        ui.end_row();
                                        group = Some(&r.group);
                                    }
                                    ui.label(format!("    {}", r.function));
                                    ui.monospace(&key);
                                    ui.end_row();
                                }
                            });
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(text(&rows, bindings, d.all));
                    }
                    if ui.button("Save...").clicked() {
                        save = true;
                    }
                    ui.label(
                        egui::RichText::new(
                            "Change shortcuts in Window > Preferences > Shortcuts.",
                        )
                        .small(),
                    );
                });
            });
        if save
            && let Some(path) =
                crate::filedialog::save(&[("Text", &["txt"])], "meshtard-shortcuts.txt")
        {
            let body = text(&rows, &self.spec.keymap.bindings, d.all);
            self.message = Some(match std::fs::write(&path, body) {
                Ok(()) => format!("Saved the shortcut list to {}.", path.display()),
                Err(e) => format!("Could not write {}: {e}", path.display()),
            });
        }
        if !r.closed {
            self.prefs.shortcut_list = Some(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_name_commands_by_menu_path() {
        let spec = Spec::builtin().unwrap();
        let rows = rows(&spec, &[]);
        let top = rows.iter().find(|r| r.id == "camera.view_top").unwrap();
        assert_eq!(top.function, "Camera/Standard Views/Top");
        assert_eq!(top.group, "Camera");
        // Right-click-only commands are listed too.
        assert!(rows.iter().any(|r| r.id == "edit.find_center"));
        // One row per command.
        let mut ids: Vec<_> = rows.iter().map(|r| &r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), rows.len());
    }

    #[test]
    fn every_menu_command_has_a_row() {
        let spec = Spec::builtin().unwrap();
        let rows = rows(&spec, &[]);
        for id in spec.commands().keys() {
            assert!(rows.iter().any(|r| &r.id == id), "{id} missing");
        }
    }

    #[test]
    fn extension_rows_use_their_menu() {
        let spec = Spec::builtin().unwrap();
        let ext = [(
            "ext:demo:hello".to_owned(),
            "Hello".to_owned(),
            vec!["Extensions".to_owned(), "Demo".to_owned()],
        )];
        let rows = rows(&spec, &ext);
        let r = rows.iter().find(|r| r.id == "ext:demo:hello").unwrap();
        assert_eq!(r.function, "Extensions/Demo/Hello");
        assert_eq!(r.group, "Extensions");
        // Next to the Extensions menu's own commands, before the right-click ones.
        let at = |id: &str| rows.iter().position(|r| r.id == id).unwrap();
        assert!(at("ext:demo:hello") < at("edit.find_center"));
        assert!(at("ext:demo:hello") > at("extensions.manager"));
    }

    /// Every default shortcut's key is one egui knows (else it can never be
    /// pressed), every bound command is in the menus and no key is bound twice.
    #[test]
    fn default_keymap_is_usable() {
        let spec = Spec::builtin().unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for spec in [spec.clone(), spec.mac_layout().unwrap()] {
            // Each layout's own menus (the Mac adds File > Close).
            let commands = spec.commands();
            seen.clear();
            for (id, texts) in &spec.keymap.bindings {
                assert!(commands.contains_key(id), "{id} is not a menu command");
                for text in texts {
                    let s = Shortcut::parse(text).unwrap();
                    assert!(egui::Key::from_name(&s.key).is_some(), "{id}: key {text}");
                    if let Some(other) = seen.insert(s, id.clone()) {
                        panic!("{text} is bound to both {other} and {id}");
                    }
                }
            }
        }
    }

    /// FEATURES.md 75: SketchUp's second keys (Windows export [inf]) and the list
    /// shows every key of a command.
    #[test]
    fn second_keys_are_bound_and_listed() {
        let spec = Spec::builtin().unwrap();
        let b = &spec.keymap.bindings;
        for (id, keys) in [
            ("camera.zoom_extents", ["Shift+Z", "Ctrl+Shift+E"]),
            ("edit.undo", ["Ctrl+Z", "Alt+Backspace"]),
            ("edit.cut", ["Ctrl+X", "Shift+Delete"]),
            ("edit.copy", ["Ctrl+C", "Ctrl+Insert"]),
            ("edit.paste", ["Ctrl+V", "Shift+Insert"]),
        ] {
            assert_eq!(b[id], keys, "{id}");
        }
        assert_eq!(key_label(b, "edit.copy"), "Ctrl+C  Ctrl+Insert");
        assert_eq!(key_labels(b, "window.preferences"), ["Ctrl+,"]);
        let rows = rows(&spec, &[]);
        let copy = rows.iter().find(|r| r.id == "edit.copy").unwrap();
        // The filter finds a command by any of its keys.
        assert!(matches(copy, &key_labels(b, "edit.copy"), "ctrl+insert"));
        assert!(!matches(copy, &key_labels(b, "edit.copy"), "ctrl+v"));
        assert!(text(&rows, b, false).contains("Ctrl+C  Ctrl+Insert"));
        assert_eq!(b["help.context_help"], ["Shift+F1"]);
    }

    /// The SketchUp 2026 (Windows) defaults researched in spec/parity/SHORTCUTS.md.
    #[test]
    fn sketchup_defaults_are_bound() {
        let spec = Spec::builtin().unwrap();
        for (id, key) in [
            ("tool.select", "Space"),
            ("tool.lasso", "Shift+Space"),
            ("tool.paint_bucket", "B"),
            ("tool.eraser", "E"),
            ("tool.line", "L"),
            ("tool.rectangle", "R"),
            ("tool.circle", "C"),
            ("tool.arc_2pt", "A"),
            ("tool.move", "M"),
            ("tool.push_pull", "P"),
            ("tool.rotate", "Q"),
            ("tool.scale", "S"),
            ("tool.offset", "F"),
            ("tool.tape_measure", "T"),
            ("camera.orbit", "O"),
            ("camera.pan", "H"),
            ("camera.zoom", "Z"),
            ("camera.zoom_extents", "Shift+Z"),
            ("camera.zoom_window", "Ctrl+Shift+W"),
            ("edit.make_component", "G"),
            ("view.back_edges", "K"),
            ("help.search", "Shift+S"),
            ("file.new", "Ctrl+N"),
            ("file.open", "Ctrl+O"),
            ("file.save", "Ctrl+S"),
            ("file.print", "Ctrl+P"),
            ("file.exit", "Ctrl+Q"),
            ("window.preferences", "Ctrl+Comma"),
            ("edit.undo", "Ctrl+Z"),
            ("edit.redo", "Ctrl+Y"),
            ("edit.cut", "Ctrl+X"),
            ("edit.copy", "Ctrl+C"),
            ("edit.paste", "Ctrl+V"),
            ("edit.delete", "Delete"),
            ("edit.select_all", "Ctrl+A"),
            ("edit.select_none", "Ctrl+T"),
            ("view.anim_previous", "PageUp"),
            ("view.anim_next", "PageDown"),
        ] {
            assert_eq!(
                spec.keymap.bindings.get(id).map(|k| k[0].as_str()),
                Some(key),
                "{id}"
            );
        }
    }

    #[test]
    fn printable_list_groups_by_menu() {
        let spec = Spec::builtin().unwrap();
        let rows = rows(&spec, &[]);
        let t = text(&rows, &spec.keymap.bindings, false);
        assert!(t.contains("\nFile\n"), "{t}");
        assert!(t.contains("File/New"), "{t}");
        assert!(t.contains("Ctrl+,"), "{t}");
        // Only bound commands unless `all`.
        assert!(!t.contains("File/Revert"), "{t}");
        assert!(text(&rows, &spec.keymap.bindings, true).contains("File/Revert"));
        // Groups appear once each, in menu order.
        assert!(t.find("\nFile\n").unwrap() < t.find("\nEdit\n").unwrap());
        assert_eq!(t.matches("\nFile\n").count(), 1);
    }

    #[test]
    fn filter_matches_function_id_or_key() {
        let r = Row {
            id: "camera.zoom_extents".into(),
            function: "Camera/Zoom Extents".into(),
            group: "Camera".into(),
        };
        assert!(matches(&r, &["Shift+Z".to_owned()], "zoom"));
        assert!(matches(&r, &["Shift+Z".to_owned()], "camera.zoom"));
        assert!(matches(&r, &["Shift+Z".to_owned()], "shift+z"));
        assert!(!matches(&r, &["Shift+Z".to_owned()], "orbit"));
    }
}
