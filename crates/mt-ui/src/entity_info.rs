//! The Entity Info panel: title, Tag dropdown, typed fields and read-only rows.
//!
//! Layout follows SketchUp 2026 as remembered (label column, then the control);
//! unverified: the order of the rows per entity type, the check boxes' placement
//! (SketchUp shows Hidden / Locked / shadow toggles as a row of check boxes or icon
//! toggles depending on the version), and the swatches' size.

use egui::{Color32, Sense, Stroke, StrokeKind, vec2};

use crate::{EntityInfo, Event, InfoEdit, InfoValue, Shell};

/// Width of the label column.
const LABEL_W: f32 = 78.0;

impl Shell {
    pub(crate) fn entity_info_panel(&mut self, ui: &mut egui::Ui, info: &EntityInfo) {
        let t = self.theme.clone();
        ui.label(egui::RichText::new(&info.title).strong());
        // Tag dropdown (SketchUp shows it under the title).
        if let Some(tag) = self.data.entity_tag {
            let data = &self.data;
            let pending = &mut self.pending;
            ui.horizontal(|ui| {
                label(ui, "Tag");
                let name = |i: usize| data.tags.get(i).map_or("", |t| t.name.as_str()).to_owned();
                egui::ComboBox::from_id_salt("mt_ui_entity_tag")
                    .selected_text(name(tag))
                    .width(ui.available_width() - 4.0)
                    .show_ui(ui, |ui| {
                        for i in 0..data.tags.len() {
                            if ui.selectable_label(i == tag, name(i)).clicked() && i != tag {
                                pending.push(Event::EntityTag(i));
                            }
                        }
                    });
            });
        }

        let mut open = true;
        for f in &info.fields {
            if let InfoValue::Section(default_open) = f.value {
                let id = ui.id().with(("mt_ui_info_section", &f.key));
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    default_open,
                );
                let r = ui.horizontal(|ui| {
                    state.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
                    ui.add(
                        egui::Label::new(egui::RichText::new(&f.label).strong())
                            .sense(Sense::click()),
                    )
                });
                if r.inner.clicked() {
                    state.toggle(ui);
                }
                open = state.is_open();
                state.store(ui.ctx());
                continue;
            }
            if !open {
                continue;
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(f.enabled, |ui| {
                    if !matches!(f.value, InfoValue::Check(_)) {
                        label(ui, &f.label);
                    }
                    if let Some(edit) = self.info_control(ui, &f.key, &f.label, &f.value, &t) {
                        self.pending.push(Event::EntityInfoEdit {
                            key: f.key.clone(),
                            edit,
                        });
                    }
                });
            });
        }

        if info.rows.is_empty() {
            return;
        }
        egui::Grid::new("mt_ui_entity_info")
            .num_columns(2)
            .min_col_width(LABEL_W - 6.0)
            .spacing(vec2(6.0, 4.0))
            .show(ui, |ui| {
                for (label, value) in &info.rows {
                    ui.label(label);
                    ui.label(value);
                    ui.end_row();
                }
            });
    }

    /// One field's control; the edit when the user committed one.
    fn info_control(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        label: &str,
        value: &InfoValue,
        t: &crate::theme::Theme32,
    ) -> Option<InfoEdit> {
        match value {
            InfoValue::Text(text) => {
                let id = ui.id().with(("mt_ui_info_text", key));
                let focused = ui.memory(|m| m.has_focus(id));
                let mut buf = match &self.tray_ui.info_edit {
                    Some((k, b)) if k == key && focused => b.clone(),
                    _ => text.clone(),
                };
                let r = ui.add(
                    egui::TextEdit::singleline(&mut buf)
                        .id(id)
                        .desired_width(ui.available_width() - 4.0),
                );
                if r.has_focus() {
                    self.tray_ui.info_edit = Some((key.to_owned(), buf.clone()));
                }
                if r.lost_focus() {
                    self.tray_ui.info_edit = None;
                    let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    let new = buf.trim();
                    if !cancel && new != text.trim() {
                        return Some(InfoEdit::Text(new.to_owned()));
                    }
                }
                None
            }
            InfoValue::Integer { value, min, max } => {
                // While dragging, the number changes here only; the edit is sent when
                // the drag ends (a segment count rebuilds geometry).
                let mut v = match self.tray_ui.info_drag {
                    Some((ref k, v)) if k == key => v,
                    _ => *value,
                };
                let r = ui.add(
                    egui::DragValue::new(&mut v)
                        .range(*min..=*max)
                        .speed(0.2)
                        .update_while_editing(false),
                );
                if r.dragged() {
                    self.tray_ui.info_drag = Some((key.to_owned(), v));
                    return None;
                }
                if r.drag_stopped() {
                    self.tray_ui.info_drag = None;
                }
                (v != *value && !r.dragged()).then_some(InfoEdit::Integer(v))
            }
            InfoValue::Check(on) => {
                let mut v = *on;
                ui.add_space(LABEL_W - 2.0 + ui.spacing().item_spacing.x);
                let changed = ui.checkbox(&mut v, label).changed();
                changed.then_some(InfoEdit::Check(v))
            }
            InfoValue::Swatch(color) => {
                let (rect, r) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
                let p = ui.painter();
                match color {
                    Some(c) => {
                        p.rect_filled(rect, 0.0, Color32::from_rgb(c[0], c[1], c[2]));
                    }
                    None => {
                        // The default material: white front over blue-grey back.
                        p.rect_filled(rect, 0.0, Color32::WHITE);
                        p.add(egui::Shape::convex_polygon(
                            vec![rect.right_top(), rect.right_bottom(), rect.left_bottom()],
                            Color32::from_rgb(164, 178, 187),
                            Stroke::NONE,
                        ));
                    }
                }
                let stroke = if r.hovered() {
                    Stroke::new(1.0, t.hover_border)
                } else {
                    Stroke::new(1.0, t.border)
                };
                p.rect_stroke(rect, 0.0, stroke, StrokeKind::Inside);
                let r = r.on_hover_text("Click to paint this side with the current material");
                r.clicked().then_some(InfoEdit::Swatch)
            }
            InfoValue::Section(_) => None,
        }
    }
}

/// The label column of a field row.
fn label(ui: &mut egui::Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(LABEL_W - 6.0, 18.0), Sense::hover());
    ui.painter().with_clip_rect(rect).text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(ui.style().text_styles[&egui::TextStyle::Body].size),
        ui.visuals().text_color(),
    );
}

#[cfg(test)]
mod tests {
    use crate::{EntityInfo, Event, Frame, InfoEdit, InfoValue, Shell};
    use egui::{Rect, pos2, vec2};
    use mt_spec::Spec;

    fn run(
        ctx: &egui::Context,
        s: &mut Shell,
        info: &EntityInfo,
        events: Vec<egui::Event>,
    ) -> Vec<Event> {
        let spec = Spec::builtin().unwrap();
        let frame = Frame {
            spec: &spec,
            active_tool: "tool.select",
            status: "",
            measurements_label: "Measurements",
            measurements_value: "",
            is_enabled: &|_| true,
            is_checked: &|_| false,
            entity_info: info,
        };
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 900.0))),
            events,
            ..Default::default()
        };
        let mut out = Vec::new();
        let mut o = ctx.run_ui(input, |ui| out = s.show(ui, &frame, |_| {}));
        o.textures_delta.clear();
        out
    }

    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(pos), button(true), button(false)]
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn only_entity_info(s: &mut Shell) {
        for p in &mut s.panels {
            p.shown = true;
            p.expanded = p.id == "window.entity_info";
        }
    }

    /// Clicks down the panel: the check box toggles, the text field takes typing and
    /// commits it with Enter, as `EntityInfoEdit` events.
    #[test]
    fn check_box_and_text_field_edits() {
        let spec = Spec::builtin().unwrap();
        let ctx = egui::Context::default();
        let mut s = Shell::new(&spec);
        let mut info = EntityInfo {
            title: "Edge".into(),
            ..Default::default()
        };
        info.field("length", "Length", InfoValue::Text("10\"".into()));
        info.field("soft", "Soft", InfoValue::Check(false));
        only_entity_info(&mut s);
        run(&ctx, &mut s, &info, vec![]);

        let mut toggled = None;
        'scan: for x in (1250..1600).step_by(25) {
            for y in (60..400).step_by(4) {
                only_entity_info(&mut s);
                let ev = run(&ctx, &mut s, &info, click(pos2(x as f32, y as f32)));
                if let Some(Event::EntityInfoEdit { key, edit }) = ev.into_iter().next() {
                    toggled = Some((key, edit));
                    break 'scan;
                }
            }
        }
        let mut typed = None;
        for y in (60..400).step_by(3) {
            only_entity_info(&mut s);
            run(&ctx, &mut s, &info, click(pos2(1500.0, y as f32)));
            if typed.is_none() && ctx.egui_wants_keyboard_input() {
                let all = egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                };
                run(
                    &ctx,
                    &mut s,
                    &info,
                    vec![all, egui::Event::Text(" 25mm".into())],
                );
                let ev = run(&ctx, &mut s, &info, vec![key(egui::Key::Enter)]);
                typed = ev.into_iter().next();
            }
        }
        assert_eq!(
            toggled,
            Some(("soft".to_owned(), InfoEdit::Check(true))),
            "a click on the check box"
        );
        assert_eq!(
            typed,
            Some(Event::EntityInfoEdit {
                key: "length".into(),
                edit: InfoEdit::Text("25mm".into())
            })
        );
    }
}
