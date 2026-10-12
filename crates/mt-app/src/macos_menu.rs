//! The macOS menu bar: the spec's menus after `Spec::mac_layout` (SketchUp for Mac's
//! layout: application menu first, `spec/menus_mac.toml`) as a native NSMenu (via
//! `muda`) instead of the in-window menu bar the shell draws on Windows and Linux.
//!
//! Shortcuts with a modifier become menu key equivalents (Ctrl in the keymap is Cmd).
//! Single-key tool shortcuts (L, R, Space, Backspace...) are left to the app's own
//! handling so they don't fire while typing in a text field. The system's own items
//! are added here: Services, Hide, Hide Others and Show All in the application menu,
//! Minimize and Zoom at the top of the Window menu.

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
        let mut menus = spec.menus.menu.iter();
        let app = Submenu::new("Meshtard", true);
        if spec.is_mac_layout() {
            let m = menus
                .next()
                .expect("the Mac layout has an application menu");
            fill_app_menu(&app, &m.items, spec, toggles, &mut items);
        } else {
            // Windows layout (main applies the Mac one): a minimal application menu.
            for item in system_items() {
                let _ = app.append(&item);
            }
            let _ = app.append(&PredefinedMenuItem::separator());
            let _ = app.append(&PredefinedMenuItem::quit(None));
        }
        let _ = menu.append(&app);
        let mut window_menu = None;
        let mut help_menu = None;
        for m in menus {
            let sub = Submenu::new(&m.label, true);
            if m.label == "Window" {
                let _ = sub.append_items(&[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(Some("Zoom")),
                    &PredefinedMenuItem::separator(),
                ]);
            }
            fill(&sub, &m.items, spec, toggles, &mut items);
            let _ = menu.append(&sub);
            match m.label.as_str() {
                "Window" => window_menu = Some(sub),
                "Help" => help_menu = Some(sub),
                _ => {}
            }
        }
        menu.init_for_nsapp();
        if let Some(sub) = &window_menu {
            sub.set_as_windows_menu_for_nsapp();
        }
        if let Some(sub) = &help_menu {
            sub.set_as_help_menu_for_nsapp();
        }
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

/// Services, Hide, Hide Others, Show All.
fn system_items() -> [PredefinedMenuItem; 5] {
    [
        PredefinedMenuItem::services(None),
        PredefinedMenuItem::separator(),
        PredefinedMenuItem::hide(None),
        PredefinedMenuItem::hide_others(None),
        PredefinedMenuItem::show_all(None),
    ]
}

/// The application menu from the spec, with the system's items put before the
/// separator that precedes the last item (Quit).
fn fill_app_menu(
    sub: &Submenu,
    items: &[SpecItem],
    spec: &Spec,
    toggles: &[&str],
    out: &mut Vec<(String, Item)>,
) {
    let split = items
        .iter()
        .rposition(|it| matches!(it, SpecItem::Separator { .. }))
        .unwrap_or(items.len());
    fill(sub, &items[..split], spec, toggles, out);
    let _ = sub.append(&PredefinedMenuItem::separator());
    for item in system_items() {
        let _ = sub.append(&item);
    }
    fill(sub, &items[split..], spec, toggles, out);
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
