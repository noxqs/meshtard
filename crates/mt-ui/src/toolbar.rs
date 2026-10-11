//! Toolbars from `spec/toolbars.toml`: docked on the window edges or floating, moved
//! by dragging their grip (docked) or title bar (floating). The layout logic is in
//! `dock.rs`.
//!
//! Unverified details (SketchUp 2026 on Windows not checked): the move cursor over the
//! grip, the drag showing only an outline until the drop (classic Windows dock bars;
//! the toolbar itself stays put meanwhile), double-clicking a floating toolbar's title
//! to dock it again, the right-click toolbar list on grips and title bars, and the
//! look of the floating window's title bar.

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, Vec2,
    pos2, vec2,
};
use mt_spec::{Toolbar, ToolbarButton, ToolbarItem};

use crate::dock::{
    self, Cell, Cells, DockEdge, DockLine, DropTarget, ROW_GAP, TITLE, ToolbarPosition,
    toolbar_cells,
};
use crate::menu::shortcut_text;
use crate::{Event, Frame, Shell, icons};

/// A toolbar being dragged by its grip or title bar.
pub(crate) struct ToolbarDrag {
    id: String,
    /// Pointer position minus the top-left corner of the floating window it would be.
    grab: Vec2,
}

/// Toolbar `id` from the spec.
fn find<'a>(frame: &Frame<'a>, id: &str) -> Option<&'a Toolbar> {
    frame.spec.toolbars.toolbar.iter().find(|t| t.id == id)
}

/// Font of a floating toolbar's title.
fn title_font(sizes: &mt_spec::UiSizes) -> FontId {
    FontId::proportional(sizes.font - 1.0)
}

/// Size of floating toolbar `tb` laid out as `cells`: 1 px border, the title bar
/// (wide enough for the name and close button) and the buttons.
fn float_size(ctx: &egui::Context, tb: &Toolbar, cells: &Cells, sizes: &mt_spec::UiSizes) -> Vec2 {
    let name = ctx.fonts_mut(|f| {
        f.layout_no_wrap(tb.name.clone(), title_font(sizes), Color32::BLACK)
            .size()
            .x
    });
    vec2(
        cells.size.x.max(name + 26.0).ceil() + 2.0,
        cells.size.y + TITLE + 2.0,
    )
}

impl Shell {
    /// Draws the toolbars docked on `edge` as a panel on that side of `ui` and
    /// records their lines for drop targets. Nothing when the edge is empty.
    pub(crate) fn docked_toolbars(&mut self, ui: &mut egui::Ui, frame: &Frame, edge: DockEdge) {
        let t = self.theme.clone();
        let vertical = edge.is_vertical();
        let available = if vertical {
            ui.available_rect_before_wrap().height()
        } else {
            ui.available_width() - 4.0
        };
        let mut lines: Vec<(u32, Vec<(&Toolbar, Cells)>)> = Vec::new();
        for (row, ids) in self.toolbar_layout.rows(edge, true) {
            let toolbars: Vec<(&Toolbar, Cells)> = ids
                .iter()
                .filter_map(|id| find(frame, id))
                .map(|tb| (tb, toolbar_cells(tb, vertical, true, &t.sizes)))
                .collect();
            let lengths: Vec<f32> = toolbars
                .iter()
                .map(|(_, c)| {
                    if vertical {
                        c.size.y
                    } else {
                        c.size.x + ROW_GAP
                    }
                })
                .collect();
            let mut toolbars = toolbars.into_iter();
            for range in dock::wrap(&lengths, available) {
                lines.push((row, toolbars.by_ref().take(range.len()).collect()));
            }
        }
        if lines.is_empty() {
            return;
        }
        // Row 0 is nearest the window edge.
        if matches!(edge, DockEdge::Bottom | DockEdge::Right) {
            lines.reverse();
        }

        if vertical {
            let width: f32 = lines
                .iter()
                .map(|(_, tbs)| tbs.iter().map(|(_, c)| c.size.x).fold(0.0, f32::max))
                .sum();
            let panel = if edge == DockEdge::Left {
                egui::Panel::left("mt_ui_toolbars_left")
            } else {
                egui::Panel::right("mt_ui_toolbars_right")
            };
            panel
                .frame(egui::Frame::NONE.fill(t.toolbar))
                .exact_size(width)
                .resizable(false)
                .show_separator_line(true)
                .show(ui, |ui| {
                    let full = ui.max_rect().y_range();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for (row, tbs) in lines {
                            let mut items = Vec::new();
                            let r = ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                                for (tb, cells) in &tbs {
                                    let rect = self.toolbar_body(ui, frame, tb, cells, true);
                                    items.push((tb.id.clone(), rect));
                                }
                            });
                            self.dock_lines.push(DockLine {
                                edge,
                                row,
                                rect: Rect::from_x_y_ranges(r.response.rect.x_range(), full),
                                items,
                            });
                        }
                    });
                });
        } else {
            let panel = if edge == DockEdge::Top {
                egui::Panel::top("mt_ui_toolbars_top")
            } else {
                egui::Panel::bottom("mt_ui_toolbars_bottom")
            };
            panel
                .frame(
                    egui::Frame::NONE
                        .fill(t.toolbar)
                        .inner_margin(egui::Margin::symmetric(2, 2)),
                )
                .resizable(false)
                .show_separator_line(true)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                    let full = ui.max_rect().x_range().expand(2.0);
                    for (row, tbs) in lines {
                        let mut items = Vec::new();
                        let r = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                            for (tb, cells) in &tbs {
                                let rect = self.toolbar_body(ui, frame, tb, cells, false);
                                items.push((tb.id.clone(), rect));
                                ui.add_space(ROW_GAP);
                            }
                        });
                        // Half the 2 px row spacing and the margin belong to the line.
                        self.dock_lines.push(DockLine {
                            edge,
                            row,
                            rect: Rect::from_x_y_ranges(
                                full,
                                r.response.rect.y_range().expand(1.0),
                            ),
                            items,
                        });
                    }
                });
        }
    }

    /// Draws `toolbar` laid out as `cells` at the cursor of `ui`; returns its rect.
    fn toolbar_body(
        &mut self,
        ui: &mut egui::Ui,
        frame: &Frame,
        toolbar: &Toolbar,
        cells: &Cells,
        vertical: bool,
    ) -> Rect {
        let pad = self.theme.sizes.toolbar_padding;
        let (rect, _) = ui.allocate_exact_size(cells.size, Sense::hover());
        for &(r, cell) in &cells.cells {
            let r = r.translate(rect.min.to_vec2());
            match cell {
                Cell::Grip => {
                    if vertical {
                        paint_grip(ui, r.shrink2(vec2(pad + 2.0, 0.0)), false);
                    } else {
                        paint_grip(ui, r, true);
                    }
                    let id = Id::new(("mt_ui_toolbar_grip", &toolbar.id));
                    let response = ui.interact(r, id, Sense::click_and_drag());
                    if response.hovered() || response.dragged() {
                        ui.ctx().set_cursor_icon(CursorIcon::Move);
                    }
                    if response.drag_started() {
                        self.toolbar_drag = Some(ToolbarDrag {
                            id: toolbar.id.clone(),
                            grab: vec2(12.0, TITLE / 2.0),
                        });
                    }
                    response.context_menu(|ui| self.toolbar_menu(ui));
                }
                Cell::Button(i) => {
                    if let ToolbarItem::Button(button) = &toolbar.items[i] {
                        let id = Id::new(("mt_ui_toolbar_button", &toolbar.id, i));
                        self.tool_button(ui, frame, &toolbar.id, button, r, id);
                    }
                }
                Cell::Separator => {
                    let stroke = Stroke::new(1.0, self.theme.separator);
                    if vertical {
                        let y = r.center().y.round() + 0.5;
                        ui.painter().hline(r.x_range().shrink(pad + 3.0), y, stroke);
                    } else {
                        let x = r.center().x.round() + 0.5;
                        ui.painter().vline(x, r.y_range().shrink(4.0), stroke);
                    }
                }
            }
        }
        rect
    }

    /// Floating toolbar windows, kept inside the app window.
    pub(crate) fn floating_toolbars(&mut self, ctx: &egui::Context, frame: &Frame) {
        let t = self.theme.clone();
        let bounds = ctx.content_rect();
        let floating: Vec<(String, egui::Pos2)> = self
            .toolbar_layout
            .floating()
            .into_iter()
            .map(|(id, p)| (id.to_owned(), p))
            .collect();
        for (id, pos) in floating {
            let Some(tb) = find(frame, &id) else {
                continue;
            };
            let vertical = dock::floats_vertical(tb);
            let cells = toolbar_cells(tb, vertical, false, &t.sizes);
            let size = float_size(ctx, tb, &cells, &t.sizes);
            let pos = dock::clamp_float(pos, size, bounds);
            egui::Area::new(Id::new(("mt_ui_float_toolbar", &id)))
                .order(Order::Middle)
                .fixed_pos(pos)
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    let outer = Rect::from_min_size(pos, size);
                    ui.allocate_rect(outer, Sense::hover());
                    let painter = ui.painter();
                    painter.rect(
                        outer,
                        0.0,
                        t.toolbar,
                        Stroke::new(1.0, t.border),
                        StrokeKind::Inside,
                    );
                    let title =
                        Rect::from_min_size(pos + vec2(1.0, 1.0), vec2(size.x - 2.0, TITLE));
                    painter.rect_filled(title, 0.0, t.tray_title);
                    painter.text(
                        pos2(title.left() + 4.0, title.center().y),
                        Align2::LEFT_CENTER,
                        &tb.name,
                        title_font(&t.sizes),
                        t.text,
                    );
                    let close = Rect::from_center_size(
                        pos2(title.right() - 9.0, title.center().y),
                        vec2(14.0, 14.0),
                    );
                    let bar = Rect::from_min_max(title.min, pos2(close.left(), title.bottom()));
                    let response = ui.interact(
                        bar,
                        Id::new(("mt_ui_float_title", &id)),
                        Sense::click_and_drag(),
                    );
                    if response.drag_started()
                        && let Some(p) = response.interact_pointer_pos()
                    {
                        self.toolbar_drag = Some(ToolbarDrag {
                            id: id.clone(),
                            grab: p - pos,
                        });
                    }
                    if response.double_clicked() {
                        self.redock(&id);
                    }
                    response.context_menu(|ui| self.toolbar_menu(ui));
                    if crate::tray::close_button(ui, close, ("mt_ui_float_close", &id), &t)
                        .on_hover_text("Close")
                        .clicked()
                    {
                        self.toolbar_layout.set_visible(&id, false);
                        self.pending.push(Event::ToolbarLayoutChanged);
                    }
                    // Buttons centered under a title bar wider than them.
                    let left = ((size.x - cells.size.x) / 2.0).floor();
                    let body = Rect::from_min_size(pos + vec2(left, TITLE + 1.0), cells.size);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                        self.toolbar_body(ui, frame, tb, &cells, vertical);
                    });
                });
        }
    }

    /// Follows a toolbar drag: outlines where it would land and moves it there when
    /// the mouse button is released. Escape cancels.
    pub(crate) fn toolbar_drag(&mut self, ctx: &egui::Context, frame: &Frame) {
        let Some(drag) = &self.toolbar_drag else {
            return;
        };
        let (pointer, down, escape) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.primary_down(),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let toolbar = find(frame, &drag.id);
        let (Some(p), Some(tb), false) = (pointer, toolbar, escape) else {
            self.toolbar_drag = None;
            return;
        };
        let sizes = &self.theme.sizes;
        let cells = toolbar_cells(tb, dock::floats_vertical(tb), false, sizes);
        let float = float_size(ctx, tb, &cells, sizes);
        let float_pos = dock::clamp_float(p - drag.grab, float, ctx.content_rect());
        let target = dock::drop_target(p, self.dock_area, &self.dock_lines, &drag.id)
            .unwrap_or(DropTarget::Float { pos: float_pos });
        let preview = match &target {
            DropTarget::Row { edge, .. } | DropTarget::NewRow { edge, .. } => {
                let size = toolbar_cells(tb, edge.is_vertical(), true, sizes).size;
                dock::dock_preview(&target, self.dock_area, &self.dock_lines, size)
            }
            DropTarget::Float { .. } => Rect::from_min_size(float_pos, float),
        };
        let painter =
            ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("mt_ui_toolbar_drag")));
        painter.rect_stroke(
            preview,
            0.0,
            Stroke::new(2.0, Color32::from_gray(80)),
            StrokeKind::Inside,
        );
        ctx.set_cursor_icon(CursorIcon::Move);
        if !down {
            let id = drag.id.clone();
            self.toolbar_drag = None;
            self.move_toolbar(&id, &target);
        }
    }

    /// Moves toolbar `id` to `target` as the user's action.
    fn move_toolbar(&mut self, id: &str, target: &DropTarget) {
        let before = self.toolbar_layout.clone();
        if let (Some(p), DropTarget::Float { .. }) = (self.toolbar_layout.get(id), target)
            && matches!(p.position, ToolbarPosition::Docked { .. })
        {
            self.last_dock.insert(id.to_owned(), p.position);
        }
        self.toolbar_layout.apply_drop(id, target);
        if self.toolbar_layout != before {
            self.pending.push(Event::ToolbarLayoutChanged);
        }
    }

    /// Docks floating toolbar `id` where it was docked last (its default place if it
    /// never was).
    fn redock(&mut self, id: &str) {
        let last = self
            .last_dock
            .get(id)
            .copied()
            .or_else(|| self.toolbar_defaults.get(id).map(|p| p.position));
        let Some(ToolbarPosition::Docked { edge, row, index }) = last else {
            return;
        };
        let rows = self.toolbar_layout.rows(edge, false);
        let target = match rows.iter().find(|(r, _)| *r == row) {
            Some((_, ids)) => DropTarget::Row {
                edge,
                row,
                before: ids.get(index as usize).map(|s| (*s).to_owned()),
            },
            None => DropTarget::NewRow {
                edge,
                row: rows.len() as u32,
            },
        };
        self.move_toolbar(id, &target);
    }

    /// Checkable list of all toolbars (right-click on a grip or title bar).
    fn toolbar_menu(&mut self, ui: &mut egui::Ui) {
        for (id, name) in self.toolbar_names.clone() {
            let mut visible = self.toolbar_layout.get(&id).is_some_and(|p| p.visible);
            if ui.checkbox(&mut visible, name).changed() {
                self.set_toolbar_visible(&id, visible);
                self.pending.push(Event::ToolbarLayoutChanged);
            }
        }
    }

    /// View > Toolbars...: a dialog with a check box per toolbar.
    /// Unverified: SketchUp's dialog also has New/Rename/Delete/Reset and an Options tab.
    pub(crate) fn toolbars_dialog(&mut self, ctx: &egui::Context) {
        if !self.toolbars_dialog_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Toolbars")
            .id(Id::new("mt_ui_toolbars_dialog"))
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(220.0);
                self.toolbar_menu(ui);
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close = ui.button("Close").clicked();
                });
            });
        if !open || close {
            self.toolbars_dialog_open = false;
        }
    }

    fn tool_button(
        &mut self,
        ui: &mut egui::Ui,
        frame: &Frame,
        toolbar_id: &str,
        button: &ToolbarButton,
        rect: Rect,
        id: Id,
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
        let response = ui.interact(rect, id, Sense::click());
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
