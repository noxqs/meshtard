//! Status bar: hint text on the left, the Measurements box on the right. The box is
//! also the Measurements toolbar's widget (`measurements_field`).

use egui::{Align, Id, Layout, Stroke, vec2};

use crate::{Event, Frame, Shell};

impl Shell {
    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui, frame: &Frame) {
        let t = self.theme.clone();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            let field_id = ui.id().with("mt_ui_measurements");
            self.measurements_field(ui, frame, field_id, t.sizes.measurements_width);
            ui.label(frame.measurements_label);
            ui.add_space(4.0);
            let (sep, _) = ui.allocate_exact_size(vec2(1.0, 16.0), egui::Sense::hover());
            ui.painter()
                .vline(sep.center().x, sep.y_range(), Stroke::new(1.0, t.separator));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                self.status_buttons(ui);
                ui.add(egui::Label::new(frame.status).truncate());
            });
        });
    }

    /// The geo-location pin (opens Model Info > Geo-location; filled when the model is
    /// geo-located) and the Model Info button, left of the hint (help [doc]; their
    /// drawings are meshtard's own, size and order unverified).
    fn status_buttons(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        let button = |ui: &mut egui::Ui, tip: &str| {
            let (rect, r) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::click());
            if r.hovered() {
                ui.painter().rect(
                    rect,
                    2.0,
                    t.hover,
                    Stroke::new(1.0, t.hover_border),
                    egui::StrokeKind::Inside,
                );
            }
            (rect, r.on_hover_text(tip))
        };
        let (rect, r) = button(
            ui,
            if self.geo_located {
                "Geo-location: the model is located"
            } else {
                "Geo-location: the model is not located"
            },
        );
        let c = rect.center();
        let head = c + vec2(0.0, -2.5);
        let fill = if self.geo_located {
            egui::Color32::from_rgb(214, 60, 50)
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                head + vec2(-3.6, 1.2),
                c + vec2(0.0, 6.5),
                head + vec2(3.6, 1.2),
            ],
            if self.geo_located { fill } else { t.status_bar },
            Stroke::new(1.2, t.text),
        ));
        ui.painter()
            .circle(head, 4.0, fill, Stroke::new(1.2, t.text));
        ui.painter().circle_filled(head, 1.3, t.text);
        if r.clicked() {
            self.pending
                .push(Event::Command("window.model_info_geo".to_owned()));
        }
        let (rect, r) = button(ui, "Model Info");
        ui.painter()
            .circle_stroke(rect.center(), 6.5, Stroke::new(1.2, t.text));
        ui.painter().text(
            rect.center() + vec2(0.0, 0.5),
            egui::Align2::CENTER_CENTER,
            "i",
            egui::FontId::proportional(11.0),
            t.text,
        );
        if r.clicked() {
            self.pending
                .push(Event::Command("window.model_info".to_owned()));
        }
        ui.add_space(2.0);
    }

    /// The Measurements value field with id `field_id`, `width` points wide. Every
    /// such field (status bar, Measurements toolbar) shows and edits the same text.
    pub(crate) fn measurements_field(
        &mut self,
        ui: &mut egui::Ui,
        frame: &Frame,
        field_id: Id,
        width: f32,
    ) -> egui::Response {
        let t = &self.theme;
        let focused = ui.memory(|m| m.has_focus(field_id));
        if self.typing {
            self.field_text.clone_from(&self.typed);
        } else if !focused {
            self.field_text = frame.measurements_value.to_owned();
        }
        let edit = egui::TextEdit::singleline(&mut self.field_text)
            .id(field_id)
            .desired_width(width)
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
        response
    }
}
