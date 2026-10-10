//! Status bar: hint text on the left, the Measurements box on the right.

use egui::{Align, Layout, Stroke, vec2};

use crate::{Event, Frame, Shell};

impl Shell {
    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui, frame: &Frame) {
        let t = self.theme.clone();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            let field_id = ui.id().with("mt_ui_measurements");
            let focused = ui.memory(|m| m.has_focus(field_id));
            if self.typing {
                self.field_text.clone_from(&self.typed);
            } else if !focused {
                self.field_text = frame.measurements_value.to_owned();
            }
            let edit = egui::TextEdit::singleline(&mut self.field_text)
                .id(field_id)
                .desired_width(t.sizes.measurements_width)
                .background_color(t.field)
                .margin(egui::Margin::symmetric(4, 1))
                .frame(
                    egui::Frame::NONE
                        .fill(t.field)
                        .stroke(Stroke::new(1.0, t.border)),
                );
            let response = ui.add(edit);
            if response.changed() {
                // Editing the box directly.
                self.typing = true;
                self.typed.clone_from(&self.field_text);
            }
            if response.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.typing = false;
                    let text = std::mem::take(&mut self.typed);
                    self.pending.push(Event::Measurements(text));
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.cancel_typing();
                }
            }
            ui.label(frame.measurements_label);
            ui.add_space(4.0);
            let (sep, _) = ui.allocate_exact_size(vec2(1.0, 16.0), egui::Sense::hover());
            ui.painter()
                .vline(sep.center().x, sep.y_range(), Stroke::new(1.0, t.separator));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(frame.status).truncate());
            });
        });
    }
}
