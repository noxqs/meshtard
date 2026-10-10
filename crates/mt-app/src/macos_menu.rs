//! The macOS menu bar: the same menus as `spec/menus.toml`, as a native NSMenu (via
//! `muda`) instead of the in-window menu bar the shell draws on Windows and Linux.
//!
//! Shortcuts with a modifier become menu key equivalents (Ctrl in the keymap is Cmd).
//! Single-key tool shortcuts (L, R, Space...) are left to the app's own handling so
//! they don't fire while typing in a text field. SketchUp for Mac's menu layout differs
//! from Windows (an application menu, Preferences under it); this uses the Windows
//! layout plus a minimal application menu (unverified against SketchUp for Mac).

use std::collections::HashMap;

use mt_spec::{MenuItem as SpecItem, Spec};
use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

enum Item {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

pub struct NativeMenu {
    _menu: Menu,
    items: Vec<(String, Item)>,
    /// Last applied (enabled, checked) per command, to skip unchanged updates.
    state: HashMap<String, (bool, bool)>,
}

impl NativeMenu {
    /// Builds the menu bar and installs it for the application. `toggles` are commands
    /// shown with a check mark.
    pub fn install(spec: &Spec, toggles: &[&str]) -> Self {
        let menu = Menu::new();
        let mut items = Vec::new();
        let app = Submenu::new("meshtard", true);
        let _ = app.append_items(&[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]);
        let _ = menu.append(&app);
        for m in &spec.menus.menu {
            let sub = Submenu::new(&m.label, true);
            fill(&sub, &m.items, spec, toggles, &mut items);
            let _ = menu.append(&sub);
        }
        menu.init_for_nsapp();
        Self {
            _menu: menu,
            items,
            state: HashMap::new(),
        }
    }

    /// Commands chosen in the menu bar since the last call.
    pub fn poll() -> Vec<String> {
        MenuEvent::receiver()
            .try_iter()
            .map(|e| e.id().0.clone())
            .collect()
    }

    /// Greys out and checks items like the in-window menu does.
    pub fn update(&mut self, enabled: &dyn Fn(&str) -> bool, checked: &dyn Fn(&str) -> bool) {
        for (id, item) in &self.items {
            let now = (enabled(id), checked(id));
            if self.state.get(id) == Some(&now) {
                continue;
            }
            match item {
                Item::Plain(i) => i.set_enabled(now.0),
                Item::Check(i) => {
                    i.set_enabled(now.0);
                    i.set_checked(now.1);
                }
            }
            self.state.insert(id.clone(), now);
        }
    }
}

fn accelerator(spec: &Spec, id: &str) -> Option<Accelerator> {
    let s = spec.keymap.shortcut(id)?;
    if !s.ctrl && !s.alt {
        return None;
    }
    let mut text = String::new();
    if s.ctrl {
        text.push_str("CmdOrCtrl+");
    }
    if s.alt {
        text.push_str("Alt+");
    }
    if s.shift {
        text.push_str("Shift+");
    }
    text.push_str(&s.key);
    text.parse().ok()
}

fn fill(
    sub: &Submenu,
    items: &[SpecItem],
    spec: &Spec,
    toggles: &[&str],
    out: &mut Vec<(String, Item)>,
) {
    for item in items {
        match item {
            SpecItem::Command { id, label } => {
                let acc = accelerator(spec, id);
                if toggles.contains(&id.as_str()) {
                    let i = CheckMenuItem::with_id(id.clone(), label, true, false, acc);
                    let _ = sub.append(&i);
                    out.push((id.clone(), Item::Check(i)));
                } else {
                    let i = MenuItem::with_id(id.clone(), label, true, acc);
                    let _ = sub.append(&i);
                    out.push((id.clone(), Item::Plain(i)));
                }
            }
            SpecItem::Submenu { label, items } => {
                let s = Submenu::new(label, true);
                fill(&s, items, spec, toggles, out);
                let _ = sub.append(&s);
            }
            SpecItem::Separator { .. } => {
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
        }
    }
}
