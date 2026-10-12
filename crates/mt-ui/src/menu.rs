//! Menu bar from `spec/menus.toml`, shortcut text from `spec/keymap.toml`.

use mt_spec::{MenuItem, Spec};

use crate::{Frame, Shell};

/// Left gutter reserved for the check mark, as in Windows menus.
const CHECK_GUTTER: &str = "       ";

impl Shell {
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui, frame: &Frame) {
        let mut clicked = None;
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in &frame.spec.menus.menu {
                ui.menu_button(&menu.label, |ui| {
                    ui.set_min_width(220.0);
                    let items = Items {
                        labels: Some(&self.menu_labels),
                        icons: Some(&self.icon_palette),
                        ..Items::of(frame)
                    };
                    menu_items(ui, &menu.items, &items, &mut clicked);
                });
            }
        });
        if let Some(id) = clicked {
            self.command(&id);
        }
    }
}

/// What menu items need to know: shortcut text, enabled and checked state.
struct Items<'a> {
    spec: &'a Spec,
    is_enabled: &'a dyn Fn(&str) -> bool,
    is_checked: &'a dyn Fn(&str) -> bool,
    /// Labels replaced by the app ([`Shell::set_menu_label`]).
    labels: Option<&'a std::collections::HashMap<String, String>>,
    /// Draw the command's toolbar icon in the check gutter (Windows menus show tool
    /// icons since SketchUp 2026, release notes).
    icons: Option<&'a crate::icons::Palette>,
}

impl<'a> Items<'a> {
    fn of(frame: &'a Frame) -> Self {
        Self {
            spec: frame.spec,
            is_enabled: frame.is_enabled,
            is_checked: frame.is_checked,
            labels: None,
            icons: None,
        }
    }
}

/// Draws `items` into an open menu (e.g. the viewport's right-click menu from
/// `egui::Response::context_menu`) with the menu bar's look: check gutter, shortcut
/// text, greyed disabled items, submenus. Returns the command clicked; the menu is
/// closed then.
pub fn menu_list(
    ui: &mut egui::Ui,
    items: &[MenuItem],
    spec: &Spec,
    is_enabled: &dyn Fn(&str) -> bool,
    is_checked: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let mut clicked = None;
    menu_items(
        ui,
        items,
        &Items {
            spec,
            is_enabled,
            is_checked,
            labels: None,
            icons: None,
        },
        &mut clicked,
    );
    clicked
}

fn menu_items(ui: &mut egui::Ui, items: &[MenuItem], frame: &Items, clicked: &mut Option<String>) {
    for item in items {
        match item {
            MenuItem::Separator { .. } => {
                ui.separator();
            }
            MenuItem::Submenu { label, items } => {
                ui.menu_button(format!("{CHECK_GUTTER}{label}"), |ui| {
                    ui.set_min_width(180.0);
                    menu_items(ui, items, frame, clicked);
                });
            }
            MenuItem::Command { id, label } => {
                let label = frame
                    .labels
                    .and_then(|l| l.get(id))
                    .map_or(label.as_str(), String::as_str);
                let mut button = egui::Button::new(format!("{CHECK_GUTTER}{label}"));
                if let Some(text) = shortcut_text(frame.spec, id) {
                    button = button.shortcut_text(text);
                }
                let enabled = (frame.is_enabled)(id);
                let response = ui.add_enabled(enabled, button);
                if (frame.is_checked)(id) {
                    paint_check(ui, response.rect, enabled);
                } else if let Some(palette) = frame.icons.filter(|_| crate::icons::has_icon(id)) {
                    let c = egui::pos2(response.rect.left() + 12.0, response.rect.center().y);
                    let rect = egui::Rect::from_center_size(c, egui::vec2(15.0, 15.0));
                    let palette = if enabled {
                        *palette
                    } else {
                        palette.disabled()
                    };
                    crate::icons::paint(ui.painter(), rect, id, &palette);
                }
                if response.clicked() {
                    *clicked = Some(id.clone());
                    ui.close();
                }
            }
        }
    }
}

/// Shortcut text shown right of a menu item, e.g. `Ctrl+S`.
pub(crate) fn shortcut_text(spec: &Spec, id: &str) -> Option<String> {
    let text = spec.keymap.shortcut(id).map(|s| s.label())?;
    // macOS shows Mac symbols: Cmd for the keymap's Ctrl (the Mac shortcuts
    // themselves come from spec/keymap_mac.toml).
    if cfg!(target_os = "macos") {
        return Some(
            text.replace("Ctrl+", "\u{2318}")
                .replace("Shift+", "\u{21e7}")
                .replace("Alt+", "\u{2325}")
                .replace("Backspace", "\u{232b}")
                .replace("Comma", ","),
        );
    }
    Some(text)
}

fn paint_check(ui: &egui::Ui, rect: egui::Rect, enabled: bool) {
    let color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let c = egui::pos2(rect.left() + 11.0, rect.center().y);
    ui.painter().line(
        vec![
            c + egui::vec2(-4.0, 0.0),
            c + egui::vec2(-1.5, 3.0),
            c + egui::vec2(4.0, -3.5),
        ],
        egui::Stroke::new(1.5, color),
    );
}
