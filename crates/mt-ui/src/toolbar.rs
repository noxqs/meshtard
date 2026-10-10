//! Docked toolbars from `spec/toolbars.toml`.

use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use mt_spec::{Dock, Toolbar, ToolbarButton, ToolbarItem};

use crate::menu::shortcut_text;
use crate::{Frame, Shell, icons};

/// Width of the drag grip at the start of a toolbar.
const GRIP: f32 = 7.0;

impl Shell {
    pub(crate) fn top_toolbars(&mut self, ui: &mut egui::Ui, frame: &Frame) {
        let rows = docked(frame, Dock::Top);
        if rows.is_empty() {
            return;
        }
        let t = self.theme.clone();
        egui::Panel::top("mt_ui_toolbars_top")
            .frame(
                egui::Frame::NONE
                    .fill(t.toolbar)
                    .inner_margin(egui::Margin::symmetric(2, 2)),
            )
            .resizable(false)
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                for row in rows {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for toolbar in row {
                            self.horizontal_toolbar(ui, frame, toolbar);
                            ui.add_space(6.0);
                        }
                    });
                }
            });
    }

    pub(crate) fn left_toolbars(&mut self, ui: &mut egui::Ui, frame: &Frame) {
        let rows = docked(frame, Dock::Left);
        if rows.is_empty() {
            return;
        }
        let t = self.theme.clone();
        let pad = t.sizes.toolbar_padding;
        let width: f32 = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|tb| tb.columns as f32 * t.sizes.toolbar_button)
                    .fold(0.0, f32::max)
                    + 2.0 * pad
            })
            .sum();
        egui::Panel::left("mt_ui_toolbars_left")
            .frame(egui::Frame::NONE.fill(t.toolbar))
            .exact_size(width)
            .resizable(false)
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    for row in rows {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                            for toolbar in row {
                                self.vertical_toolbar(ui, frame, toolbar);
                            }
                        });
                    }
                });
            });
    }

    fn horizontal_toolbar(&mut self, ui: &mut egui::Ui, frame: &Frame, toolbar: &Toolbar) {
        let t = &self.theme;
        let size = t.sizes.toolbar_button;
        let (grip, _) = ui.allocate_exact_size(vec2(GRIP, size), Sense::hover());
        paint_grip(ui, grip, true);
        for item in &toolbar.items {
            match item {
                ToolbarItem::Button(button) => self.tool_button(ui, frame, &toolbar.id, button),
                ToolbarItem::Separator { .. } => {
                    let gap = self.theme.sizes.separator_gap;
                    let (rect, _) = ui.allocate_exact_size(vec2(gap, size), Sense::hover());
                    let x = rect.center().x.round() + 0.5;
                    ui.painter().vline(
                        x,
                        rect.y_range().shrink(4.0),
                        Stroke::new(1.0, self.theme.separator),
                    );
                }
            }
        }
    }

    fn vertical_toolbar(&mut self, ui: &mut egui::Ui, frame: &Frame, toolbar: &Toolbar) {
        let size = self.theme.sizes.toolbar_button;
        let pad = self.theme.sizes.toolbar_padding;
        let width = toolbar.columns as f32 * size;
        ui.add_space(pad);
        let (grip, _) = ui.allocate_exact_size(vec2(width + 2.0 * pad, GRIP), Sense::hover());
        paint_grip(ui, grip.shrink2(vec2(pad + 2.0, 0.0)), false);

        let mut group: Vec<&ToolbarButton> = Vec::new();
        let items = toolbar.items.iter().map(Some).chain(std::iter::once(None));
        for item in items {
            match item {
                Some(ToolbarItem::Button(b)) => group.push(b),
                _ => {
                    for chunk in group.chunks(toolbar.columns as usize) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                            ui.add_space(pad);
                            for button in chunk {
                                self.tool_button(ui, frame, &toolbar.id, button);
                            }
                        });
                    }
                    group.clear();
                    if item.is_some() {
                        let gap = self.theme.sizes.separator_gap;
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(width + 2.0 * pad, gap), Sense::hover());
                        let y = rect.center().y.round() + 0.5;
                        ui.painter().hline(
                            rect.x_range().shrink(pad + 3.0),
                            y,
                            Stroke::new(1.0, self.theme.separator),
                        );
                    }
                }
            }
        }
        ui.add_space(pad);
    }

    fn tool_button(
        &mut self,
        ui: &mut egui::Ui,
        frame: &Frame,
        toolbar_id: &str,
        button: &ToolbarButton,
    ) {
        let key = format!("{toolbar_id}/{}", button.id);
        let shown: String = if button.flyout.iter().any(|id| id == frame.active_tool) {
            frame.active_tool.to_owned()
        } else {
            self.flyouts
                .get(&key)
                .cloned()
                .unwrap_or_else(|| button.id.clone())
        };
        let mut tooltip = if shown == button.id {
            button.tooltip.clone()
        } else {
            self.label(&shown).to_owned()
        };
        if let Some(sc) = shortcut_text(frame.spec, &shown) {
            tooltip = format!("{tooltip} ({sc})");
        }
        let enabled = (frame.is_enabled)(&shown);
        let pressed = shown == frame.active_tool || (frame.is_checked)(&shown);

        let t = &self.theme;
        let size = t.sizes.toolbar_button;
        let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let hovered = response.hovered() && enabled;
        let held = hovered && response.is_pointer_button_down_on();
        let painter = ui.painter();
        let body = rect.shrink(1.0);
        if pressed || held {
            painter.rect(
                body,
                0.0,
                t.pressed,
                Stroke::new(1.0, t.pressed_border),
                egui::StrokeKind::Inside,
            );
        } else if hovered {
            painter.rect(
                body,
                0.0,
                t.hover,
                Stroke::new(1.0, t.hover_border),
                egui::StrokeKind::Inside,
            );
        }
        let icon_rect = Rect::from_center_size(
            rect.center(),
            vec2(t.sizes.toolbar_icon, t.sizes.toolbar_icon),
        );
        let palette = if enabled {
            self.icon_palette
        } else {
            self.icon_palette.disabled()
        };
        icons::paint(painter, icon_rect, &shown, &palette);

        let arrow = Rect::from_min_max(rect.max - vec2(9.0, 9.0), rect.max);
        if !button.flyout.is_empty() {
            let c = arrow.center();
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x + 3.0, c.y - 3.0),
                    pos2(c.x + 3.0, c.y + 3.0),
                    pos2(c.x - 3.0, c.y + 3.0),
                ],
                if enabled { t.text } else { t.text_disabled },
                Stroke::NONE,
            ));
        }

        let on_arrow = response
            .interact_pointer_pos()
            .is_some_and(|p| arrow.expand(2.0).contains(p));
        let open = (response.clicked() && on_arrow && !button.flyout.is_empty())
            .then_some(egui::SetOpenCommand::Toggle);
        let mut chosen = None;
        if !button.flyout.is_empty() {
            egui::Popup::menu(&response).open_memory(open).show(|ui| {
                ui.set_min_width(170.0);
                for id in &button.flyout {
                    let mut b = egui::Button::new(format!("       {}", self.label(id)));
                    if let Some(sc) = shortcut_text(frame.spec, id) {
                        b = b.shortcut_text(sc);
                    }
                    let ok = (frame.is_enabled)(id);
                    let r = ui.add_enabled(ok, b);
                    let icon = Rect::from_center_size(
                        pos2(r.rect.left() + 12.0, r.rect.center().y),
                        vec2(16.0, 16.0),
                    );
                    let pal = if ok {
                        self.icon_palette
                    } else {
                        self.icon_palette.disabled()
                    };
                    icons::paint(ui.painter(), icon, id, &pal);
                    if r.clicked() {
                        chosen = Some(id.clone());
                        ui.close();
                    }
                }
            });
        }
        if response.clicked() && !on_arrow && enabled {
            chosen = Some(shown);
        }
        response.on_hover_text(tooltip);
        if let Some(id) = chosen {
            self.flyouts.insert(key, id.clone());
            self.command(&id);
        }
    }
}

/// Visible toolbars on `dock`, grouped by row in row order.
fn docked<'a>(frame: &Frame<'a>, dock: Dock) -> Vec<Vec<&'a Toolbar>> {
    let mut toolbars: Vec<&Toolbar> = frame
        .spec
        .toolbars
        .toolbar
        .iter()
        .filter(|t| t.visible && t.dock == dock)
        .collect();
    toolbars.sort_by_key(|t| t.row);
    let mut rows: Vec<Vec<&Toolbar>> = Vec::new();
    for t in toolbars {
        match rows.last_mut() {
            Some(row) if row[0].row == t.row => row.push(t),
            _ => rows.push(vec![t]),
        }
    }
    rows
}

/// Dotted drag handle at the start of a toolbar.
fn paint_grip(ui: &egui::Ui, rect: Rect, vertical: bool) {
    let color = Color32::from_gray(160);
    let painter = ui.painter();
    let c = rect.center();
    for i in -4..=4 {
        let o = i as f32 * 3.0;
        let p = if vertical {
            pos2(c.x, c.y + o)
        } else {
            pos2(c.x + o, c.y)
        };
        painter.rect_filled(Rect::from_center_size(p, vec2(1.5, 1.5)), 0.0, color);
    }
}
