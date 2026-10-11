//! The Default Tray from `spec/tray.toml`: a column of collapsible panels. Panel
//! contents other than Entity Info are in `panels.rs`.

use egui::{Align2, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::{Frame, Shell};

impl Shell {
    pub(crate) fn tray(&mut self, ui: &mut egui::Ui, frame: &Frame) {
        let t = self.theme.clone();
        let h = t.sizes.panel_header_height;
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

        // Title bar: tray name, pin and close.
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), h + 2.0), Sense::hover());
        ui.painter().rect_filled(bar, 0.0, t.tray_title);
        ui.painter().hline(
            bar.x_range(),
            bar.bottom() - 0.5,
            Stroke::new(1.0, t.border),
        );
        if self.trays.trays.len() > 1 {
            // Several trays: the title switches between them.
            let names: Vec<String> = self.trays.trays.iter().map(|t| t.name.clone()).collect();
            let active = self.trays.active;
            let area = Rect::from_min_max(
                pos2(bar.left() + 4.0, bar.top() + 1.0),
                pos2(bar.right() - 44.0, bar.bottom() - 1.0),
            );
            let mut pick = None;
            // A child ui over the bar, not taking layout space.
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            egui::ComboBox::from_id_salt("mt_ui_tray_switch")
                .selected_text(&names[active])
                .width(area.width() - 8.0)
                .show_ui(&mut child, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        if ui.selectable_label(i == active, n).clicked() {
                            pick = Some(i);
                        }
                    }
                });
            if let Some(i) = pick {
                self.switch_tray(i);
            }
        } else {
            ui.painter().text(
                pos2(bar.left() + 8.0, bar.center().y),
                Align2::LEFT_CENTER,
                self.active_tray_name(),
                FontId::proportional(t.sizes.font),
                t.text,
            );
        }
        let close =
            Rect::from_center_size(pos2(bar.right() - 12.0, bar.center().y), vec2(16.0, 16.0));
        if close_button(ui, close, "mt_ui_tray_close", &t).clicked() {
            self.tray_visible = false;
        }
        let pin =
            Rect::from_center_size(pos2(bar.right() - 32.0, bar.center().y), vec2(16.0, 16.0));
        paint_pin(ui, pin, t.text);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for i in 0..self.panels.len() {
                    if !self.panels[i].shown {
                        continue;
                    }
                    self.panel(ui, frame, i);
                }
            });
    }

    fn panel(&mut self, ui: &mut egui::Ui, frame: &Frame, i: usize) {
        let t = self.theme.clone();
        let h = t.sizes.panel_header_height;
        let id = self.panels[i].id.clone();
        let (header, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        let painter = ui.painter();
        let fill = if response.hovered() {
            t.hover
        } else {
            t.panel_header
        };
        painter.rect(
            header,
            0.0,
            fill,
            Stroke::new(1.0, t.border),
            StrokeKind::Inside,
        );

        // Expand triangle.
        let c = pos2(header.left() + 10.0, header.center().y);
        let tri = if self.panels[i].expanded {
            vec![
                pos2(c.x - 4.0, c.y - 2.0),
                pos2(c.x + 4.0, c.y - 2.0),
                pos2(c.x, c.y + 3.0),
            ]
        } else {
            vec![
                pos2(c.x - 2.0, c.y - 4.0),
                pos2(c.x + 3.0, c.y),
                pos2(c.x - 2.0, c.y + 4.0),
            ]
        };
        painter.add(egui::Shape::convex_polygon(tri, t.text, Stroke::NONE));
        painter.text(
            pos2(header.left() + 20.0, header.center().y),
            Align2::LEFT_CENTER,
            &self.panels[i].title,
            FontId::proportional(t.sizes.font),
            t.text,
        );
        let close = Rect::from_center_size(
            pos2(header.right() - 12.0, header.center().y),
            vec2(14.0, 14.0),
        );
        let closed = close_button(ui, close, ("mt_ui_panel_close", &id), &t).clicked();
        if closed {
            // Closing a panel takes it out of the tray (Window menu puts it back).
            self.set_panel_in_tray(&id, false);
            return;
        }
        if response.clicked() {
            self.panels[i].expanded = !self.panels[i].expanded;
        }
        if !self.panels[i].expanded {
            return;
        }
        egui::Frame::NONE
            .fill(t.panel_body)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                if id == "window.entity_info" {
                    self.entity_info_panel(ui, frame.entity_info);
                } else if !self.panel_body(ui, &id) {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: not implemented yet.",
                            self.panels[i].title
                        ))
                        .color(t.text_disabled),
                    );
                }
            });
    }
}

/// Small flat "x" button.
pub(crate) fn close_button(
    ui: &mut egui::Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    t: &crate::theme::Theme32,
) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect(
            rect,
            0.0,
            t.hover,
            Stroke::new(1.0, t.hover_border),
            StrokeKind::Inside,
        );
    }
    let r = rect.shrink(4.5);
    let s = Stroke::new(1.2, t.text);
    painter.line_segment([r.left_top(), r.right_bottom()], s);
    painter.line_segment([r.right_top(), r.left_bottom()], s);
    response
}

/// Push pin (auto-hide toggle in SketchUp; shown, not functional yet).
fn paint_pin(ui: &egui::Ui, rect: Rect, color: egui::Color32) {
    let p = ui.painter();
    let c = rect.center();
    let s = Stroke::new(1.2, color);
    p.rect_stroke(
        Rect::from_center_size(c + vec2(0.0, -2.0), vec2(5.0, 7.0)),
        0.0,
        s,
        StrokeKind::Middle,
    );
    p.line_segment([c + vec2(-5.0, 1.5), c + vec2(5.0, 1.5)], s);
    p.line_segment([c + vec2(0.0, 1.5), c + vec2(0.0, 7.0)], s);
}
