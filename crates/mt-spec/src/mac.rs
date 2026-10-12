//! The macOS overlay: `menus_mac.toml` and `keymap_mac.toml` turn the Windows menus
//! and shortcuts into SketchUp for Mac's. Plain data and functions, so every system
//! (and Linux CI) builds and tests it; only the app decides to apply it on macOS.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::{Error, Menu, MenuItem, Shortcut, Spec};

/// `menus_mac.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacMenus {
    pub verified: bool,
    /// The application menu, put first. Its commands leave their Windows places.
    pub app_menu: Menu,
    /// Submenus ("Menu/Submenu/...") replaced by their items.
    #[serde(default)]
    pub flatten: Vec<String>,
    /// New labels by command id.
    #[serde(default)]
    pub relabel: BTreeMap<String, String>,
}

/// `keymap_mac.toml`: shortcuts replacing the Windows ones; empty text unbinds.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacKeymap {
    pub verified: bool,
    pub bindings: BTreeMap<String, String>,
}

/// Both macOS files.
#[derive(Debug, Clone)]
pub struct MacOverlay {
    pub menus: MacMenus,
    pub keymap: MacKeymap,
}

fn menus_err(message: String) -> Error {
    Error {
        file: "menus_mac.toml",
        message,
    }
}

impl Spec {
    /// Whether the macOS overlay has been applied.
    pub fn is_mac_layout(&self) -> bool {
        self.menus.app_menu
    }

    /// This spec with SketchUp for Mac's menu layout and default shortcuts. The
    /// application menu becomes `menus.menu[0]` and `menus.app_menu` is set.
    pub fn mac_layout(&self) -> Result<Spec, Error> {
        let mut spec = self.clone();
        if spec.menus.app_menu {
            return Ok(spec);
        }
        let mac = &self.mac;
        let known = self.commands();

        // Application menu: its commands leave their Windows places.
        let mut app = mac.menus.app_menu.clone();
        let mut moved = BTreeSet::new();
        let mut unknown = None;
        visit_items(&app.items, &mut |id| {
            if !known.contains_key(id) && unknown.is_none() {
                unknown = Some(id.to_owned());
            }
            moved.insert(id.to_owned());
        });
        if let Some(id) = unknown {
            return Err(menus_err(format!(
                "app_menu: `{id}` is not a command in menus.toml"
            )));
        }
        for menu in &mut spec.menus.menu {
            remove_commands(&mut menu.items, &moved);
        }

        for path in &mac.menus.flatten {
            if !flatten(&mut spec.menus.menu, path) {
                return Err(menus_err(format!("flatten: no submenu `{path}`")));
            }
        }

        for (id, label) in &mac.menus.relabel {
            let mut found = relabel(&mut app.items, id, label);
            for menu in &mut spec.menus.menu {
                found |= relabel(&mut menu.items, id, label);
            }
            if !found {
                return Err(menus_err(format!(
                    "relabel: `{id}` is not a command in menus.toml"
                )));
            }
        }

        for menu in &mut spec.menus.menu {
            tidy_separators(&mut menu.items);
        }
        tidy_separators(&mut app.items);
        spec.menus.menu.insert(0, app);
        spec.menus.app_menu = true;

        for (id, key) in &mac.keymap.bindings {
            let err = |message: String| Error {
                file: "keymap_mac.toml",
                message: format!("`{id}`: {message}"),
            };
            if !known.contains_key(id) {
                return Err(err("not a command in menus.toml".into()));
            }
            if key.is_empty() {
                spec.keymap.bindings.remove(id);
            } else {
                // Stored in keymap.toml's spelling ("Ctrl"), so string compares and
                // the menu's shortcut text work the same on every system.
                let s = Shortcut::parse(key).map_err(err)?;
                spec.keymap.bindings.insert(id.clone(), s.to_string());
            }
        }
        spec.validate()?;
        if spec.commands().keys().ne(known.keys()) {
            return Err(menus_err(
                "the Mac layout has other commands than menus.toml".into(),
            ));
        }
        Ok(spec)
    }
}

fn visit_items(items: &[MenuItem], f: &mut impl FnMut(&str)) {
    crate::visit_items(items, &mut |id, _| f(id));
}

fn remove_commands(items: &mut Vec<MenuItem>, ids: &BTreeSet<String>) {
    items.retain_mut(|item| match item {
        MenuItem::Command { id, .. } => !ids.contains(id),
        MenuItem::Submenu { items, .. } => {
            remove_commands(items, ids);
            !items.is_empty()
        }
        MenuItem::Separator { .. } => true,
    });
}

/// Replaces the submenu at `path` ("Menu/Submenu/...") by its items.
fn flatten(menus: &mut [Menu], path: &str) -> bool {
    let mut parts = path.split('/');
    let Some(top) = parts.next() else {
        return false;
    };
    let names: Vec<&str> = parts.collect();
    let Some((last, subs)) = names.split_last() else {
        return false;
    };
    let Some(menu) = menus.iter_mut().find(|m| m.label == top) else {
        return false;
    };
    let mut items = &mut menu.items;
    for name in subs {
        let Some(MenuItem::Submenu { items: inner, .. }) = items
            .iter_mut()
            .find(|it| matches!(it, MenuItem::Submenu { label, .. } if label == name))
        else {
            return false;
        };
        items = inner;
    }
    let Some(pos) = items
        .iter()
        .position(|it| matches!(it, MenuItem::Submenu { label, .. } if label == last))
    else {
        return false;
    };
    let MenuItem::Submenu { items: inner, .. } = items.remove(pos) else {
        unreachable!("matched a submenu");
    };
    items.splice(pos..pos, inner);
    true
}

fn relabel(items: &mut [MenuItem], target: &str, new: &str) -> bool {
    let mut found = false;
    for item in items {
        match item {
            MenuItem::Command { id, label } if id == target => {
                *label = new.to_owned();
                found = true;
            }
            MenuItem::Submenu { items, .. } => found |= relabel(items, target, new),
            _ => {}
        }
    }
    found
}

/// Drops separators at the start or end of a list and doubled ones.
fn tidy_separators(items: &mut Vec<MenuItem>) {
    let sep = |it: &MenuItem| matches!(it, MenuItem::Separator { .. });
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for mut item in items.drain(..) {
        if let MenuItem::Submenu { items, .. } = &mut item {
            tidy_separators(items);
        }
        if sep(&item) && out.last().is_none_or(sep) {
            continue;
        }
        out.push(item);
    }
    if out.last().is_some_and(sep) {
        out.pop();
    }
    *items = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BUILTIN, Sources};

    fn both() -> (Spec, Spec) {
        let win = Spec::builtin().unwrap();
        let mac = win.mac_layout().unwrap();
        (win, mac)
    }

    /// Occurrences of every command id in a layout.
    fn counts(spec: &Spec) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for menu in &spec.menus.menu {
            menu.visit_commands(&mut |id, _| *out.entry(id.to_owned()).or_insert(0) += 1);
        }
        out
    }

    fn labels(spec: &Spec) -> Vec<&str> {
        spec.menus.menu.iter().map(|m| m.label.as_str()).collect()
    }

    fn items_of<'a>(spec: &'a Spec, menu: &str) -> &'a [MenuItem] {
        &spec
            .menus
            .menu
            .iter()
            .find(|m| m.label == menu)
            .unwrap()
            .items
    }

    fn ids_of(items: &[MenuItem]) -> Vec<&str> {
        items
            .iter()
            .filter_map(|it| match it {
                MenuItem::Command { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn both_layouts_have_every_command_exactly_once() {
        let (win, mac) = both();
        let (w, m) = (counts(&win), counts(&mac));
        assert!(w.values().all(|n| *n == 1), "Windows: {w:?}");
        assert!(m.values().all(|n| *n == 1), "Mac: {m:?}");
        assert!(w.keys().eq(m.keys()));
        assert_eq!(win.commands().len(), mac.commands().len());
    }

    #[test]
    fn shortcuts_do_not_collide_in_either_layout() {
        let (win, mac) = both();
        for spec in [&win, &mac] {
            let mut seen = BTreeMap::new();
            for (id, key) in &spec.keymap.bindings {
                let s = Shortcut::parse(key).unwrap();
                if let Some(other) = seen.insert(s, id) {
                    panic!("`{key}` bound to `{other}` and `{id}`");
                }
                assert!(spec.commands().contains_key(id), "{id}");
            }
        }
    }

    #[test]
    fn mac_keymap_leaves_system_shortcuts_free() {
        // Cmd+H Hide, Cmd+Alt+H Hide Others, Cmd+M Minimize come from the native
        // menu bar's predefined items.
        let (_, mac) = both();
        for key in ["Ctrl+H", "Ctrl+Alt+H", "Ctrl+M"] {
            let s = Shortcut::parse(key).unwrap();
            assert!(
                !mac.keymap
                    .bindings
                    .values()
                    .any(|k| Shortcut::parse(k).unwrap() == s),
                "{key}"
            );
        }
    }

    #[test]
    fn mac_layout_moves_app_items_and_keeps_windows_unchanged() {
        let (win, mac) = both();
        assert!(!win.is_mac_layout());
        assert!(mac.is_mac_layout());
        assert_eq!(win.menus.menu[0].label, "File");
        assert_eq!(
            labels(&mac),
            [
                "Meshtard",
                "File",
                "Edit",
                "View",
                "Camera",
                "Draw",
                "Tools",
                "Window",
                "Extensions",
                "Help"
            ]
        );
        assert_eq!(
            ids_of(&mac.menus.menu[0].items),
            ["help.about", "window.preferences", "file.exit"]
        );
        assert_eq!(
            mac.command_label("window.preferences").unwrap(),
            "Settings..."
        );
        assert_eq!(mac.command_label("file.exit").unwrap(), "Quit Meshtard");
        assert_eq!(win.command_label("file.exit").unwrap(), "Exit");
        assert!(!ids_of(items_of(&mac, "File")).contains(&"file.exit"));
        assert!(!ids_of(items_of(&mac, "Help")).contains(&"help.about"));
        let window = items_of(&mac, "Window");
        assert!(!ids_of(window).contains(&"window.preferences"));
        // Default Tray flattened into the Window menu.
        assert!(ids_of(window).contains(&"window.tray_show"));
        assert_eq!(
            mac.command_label("window.tray_show").unwrap(),
            "Show Dialogs"
        );
        assert!(ids_of(items_of(&win, "Window")).contains(&"window.preferences"));
        // No menu starts or ends with a separator, nor has two in a row.
        for menu in &mac.menus.menu {
            let sep: Vec<bool> = (menu.items.iter())
                .map(|it| matches!(it, MenuItem::Separator { .. }))
                .collect();
            assert!(
                !sep.first().unwrap() && !sep.last().unwrap(),
                "{}",
                menu.label
            );
            assert!(!sep.windows(2).any(|w| w[0] && w[1]), "{}", menu.label);
        }
        // Applying twice changes nothing.
        assert_eq!(labels(&mac.mac_layout().unwrap()), labels(&mac));
    }

    #[test]
    fn mac_shortcuts_replace_windows_ones() {
        let (win, mac) = both();
        let key = |s: &Spec, id: &str| s.keymap.bindings.get(id).cloned();
        assert_eq!(key(&win, "edit.redo").unwrap(), "Ctrl+Y");
        assert_eq!(key(&mac, "edit.redo").unwrap(), "Ctrl+Shift+Z");
        assert_eq!(key(&win, "edit.select_none").unwrap(), "Ctrl+T");
        assert_eq!(key(&mac, "edit.select_none").unwrap(), "Ctrl+Shift+A");
        assert_eq!(key(&mac, "edit.delete").unwrap(), "Backspace");
        assert_eq!(key(&mac, "window.preferences").unwrap(), "Ctrl+Comma");
        assert_eq!(key(&mac, "file.exit").unwrap(), "Ctrl+Q");
        assert_eq!(key(&win, "file.exit"), None);
        // Tool keys are the same on both systems.
        for id in ["tool.line", "tool.select", "tool.push_pull", "help.search"] {
            assert_eq!(key(&win, id), key(&mac, id), "{id}");
        }
    }

    #[test]
    fn disk_mac_overlay_matches_builtin() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec");
        let disk = Spec::load_dir(&dir).unwrap().mac_layout().unwrap();
        let (_, mac) = both();
        assert_eq!(disk.keymap.bindings, mac.keymap.bindings);
        assert_eq!(disk.commands(), mac.commands());
    }

    #[test]
    fn bad_overlays_are_rejected() {
        let bad = |menus_mac: &str, keymap_mac: &str| {
            Spec::parse(&Sources {
                menus_mac,
                keymap_mac,
                ..BUILTIN
            })
            .unwrap_err()
            .to_string()
        };
        let m = BUILTIN.menus_mac;
        let k = BUILTIN.keymap_mac;
        let err = bad(&m.replacen("\"help.about\"", "\"help.nope\"", 1), k);
        assert!(err.contains("help.nope"), "{err}");
        let err = bad(&m.replacen("Window/Default Tray", "Window/Nope", 1), k);
        assert!(err.contains("Window/Nope"), "{err}");
        let err = bad(m, &k.replacen("\"Cmd+Shift+A\"", "\"Cmd+Shift+Z\"", 1));
        assert!(err.contains("bound to both"), "{err}");
        let err = bad(m, &k.replacen("\"edit.redo\"", "\"edit.nope\"", 1));
        assert!(err.contains("edit.nope"), "{err}");
    }

    #[test]
    fn empty_mac_binding_unbinds() {
        let keymap_mac = "verified = false\n[bindings]\n\"edit.select_none\" = \"\"\n";
        let spec = Spec::parse(&Sources {
            keymap_mac,
            ..BUILTIN
        })
        .unwrap();
        let mac = spec.mac_layout().unwrap();
        assert!(!mac.keymap.bindings.contains_key("edit.select_none"));
        assert!(spec.keymap.bindings.contains_key("edit.select_none"));
    }

    #[test]
    fn flatten_and_tidy() {
        let mut items = vec![
            MenuItem::Separator { separator: true },
            MenuItem::Command {
                id: "a".into(),
                label: "A".into(),
            },
            MenuItem::Separator { separator: true },
            MenuItem::Separator { separator: true },
            MenuItem::Command {
                id: "b".into(),
                label: "B".into(),
            },
            MenuItem::Separator { separator: true },
        ];
        tidy_separators(&mut items);
        assert_eq!(items.len(), 3);
        let mut menus = vec![Menu {
            label: "M".into(),
            items: vec![MenuItem::Submenu {
                label: "S".into(),
                items: items.clone(),
            }],
        }];
        assert!(!flatten(&mut menus, "M"));
        assert!(!flatten(&mut menus, "M/T"));
        assert!(flatten(&mut menus, "M/S"));
        assert_eq!(ids_of(&menus[0].items), ["a", "b"]);
    }
}
