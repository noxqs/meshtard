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
                    menu_items(ui, &menu.items, frame, &mut clicked);
                });
            }
        });
        if let Some(id) = clicked {
            self.command(&id);
        }
    }
}

fn menu_items(ui: &mut egui::Ui, items: &[MenuItem], frame: &Frame, clicked: &mut Option<String>) {
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
    spec.keymap.shortcut(id).map(|s| s.to_string())
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
