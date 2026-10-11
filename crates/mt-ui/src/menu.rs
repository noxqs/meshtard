//! Menu bar from `spec/menus.toml`, shortcut text from `spec/keymap.toml`.

use mt_spec::{MenuItem, Spec};

use crate::{Frame, Shell};

/// Left gutter reserved for the check mark, as in Windows menus.
const CHECK_GUTTER: &str = "     ";

impl Shell {
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui, frame: &Frame) {
        let mut clicked = None;
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in &frame.spec.menus.menu {
                ui.menu_button(&menu.label, |ui| {
                    ui.set_min_width(220.0);
                    menu_items(ui, &menu.items, &Items::of(frame), &mut clicked);
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
}

impl<'a> Items<'a> {
    fn of(frame: &'a Frame) -> Self {
        Self {
            spec: frame.spec,
            is_enabled: frame.is_enabled,
            is_checked: frame.is_checked,
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
                let mut button = egui::Button::new(format!("{CHECK_GUTTER}{label}"));
                if let Some(text) = shortcut_text(frame.spec, id) {
                    button = button.shortcut_text(text);
                }
                let enabled = (frame.is_enabled)(id);
                let response = ui.add_enabled(enabled, button);
                if (frame.is_checked)(id) {
                    paint_check(ui, response.rect, enabled);
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
    let text = spec.keymap.shortcut(id).map(|s| s.to_string())?;
    // macOS shows Cmd for the keymap's Ctrl (SketchUp for Mac's own shortcut list is
    // not covered by the spec yet).
    if cfg!(target_os = "macos") {
        return Some(
            text.replace("Ctrl+", "\u{2318}")
                .replace("Shift+", "\u{21e7}"),
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
