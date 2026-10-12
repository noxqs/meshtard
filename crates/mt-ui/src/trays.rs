//! Window > Manage Trays: named trays, each with its own set of panels.
//!
//! One tray is docked at a time (on the right, as the Default Tray); the others are
//! kept and can be switched to from the tray's title bar or the dialog, or float as
//! windows of their own (SketchUp 2026 release notes: panels move between trays and
//! float). Right-click a tray's title or a panel's header for its panels (show and
//! hide), Float / Dock, Move To another tray and Make Tray; drag a panel's header onto
//! another tray to move it there, or out over the viewport to make a floating tray of
//! it; double-click a floating tray's title to dock it. SketchUp docks several trays
//! side by side; here one is docked at a time (unverified details). The first tray is the Default Tray from `spec/tray.toml`; it can
//! be renamed and its panels changed but not deleted. Panels are listed in spec order
//! in every tray. [`TrayLayout`] is serde-serializable; [`crate::Event::TrayLayoutChanged`]
//! tells when to save it, [`Shell::set_tray_layout`] restores it.

use serde::{Deserialize, Serialize};

use crate::{Event, Shell};

/// The trays and which one is shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrayLayout {
    pub trays: Vec<TrayDef>,
    /// Index into `trays` of the docked tray.
    pub active: usize,
}

/// A tray: its name and the ids of its panels (Window menu commands).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrayDef {
    pub name: String,
    pub panels: Vec<String>,
    /// Shown as a floating window (never the docked tray).
    #[serde(default)]
    pub floating: bool,
    /// Where the floating window was put (points, top left).
    #[serde(default)]
    pub pos: Option<[f32; 2]>,
}

impl TrayLayout {
    /// The Default Tray of the spec: the panels shown at first.
    pub(crate) fn from_spec(spec: &mt_spec::Spec) -> Self {
        Self {
            trays: vec![TrayDef {
                name: spec.tray.name.clone(),
                panels: spec
                    .tray
                    .panel
                    .iter()
                    .filter(|p| p.shown)
                    .map(|p| p.id.clone())
                    .collect(),
                ..Default::default()
            }],
            active: 0,
        }
    }

    /// `self` cleaned up for the panels `known`: unknown ids and empty names
    /// dropped, at least one tray, `active` in range.
    pub(crate) fn validated(mut self, known: &[String], default: &TrayLayout) -> Self {
        self.trays.retain(|t| !t.name.trim().is_empty());
        for t in &mut self.trays {
            t.panels.retain(|p| known.contains(p));
            let mut seen = std::collections::HashSet::new();
            t.panels.retain(|p| seen.insert(p.clone()));
        }
        if self.trays.is_empty() {
            return default.clone();
        }
        if self.active >= self.trays.len() {
            self.active = 0;
        }
        // The docked tray never floats, and the Default Tray always docks.
        self.trays[0].floating = false;
        if self.trays[self.active].floating {
            self.active = 0;
        }
        // A panel in a floating tray is in no other tray (named docked trays may
        // share panels: only one of them shows at a time).
        let floating: Vec<String> = self
            .trays
            .iter()
            .filter(|t| t.floating)
            .flat_map(|t| t.panels.clone())
            .collect();
        let mut seen = std::collections::HashSet::new();
        for t in &mut self.trays {
            t.panels
                .retain(|p| !floating.contains(p) || (t.floating && seen.insert(p.clone())));
        }
        self
    }

    /// A name not used by another tray: `base`, or `base 2`, `base 3`, ...
    pub(crate) fn unique_name(&self, base: &str) -> String {
        let used = |n: &str| self.trays.iter().any(|t| t.name == n);
        if !used(base) {
            return base.to_owned();
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !used(n))
            .expect("some number is free")
    }
}

/// The Manage Trays dialog's state.
#[derive(Debug, Clone, Default)]
pub(crate) struct TraysDialog {
    /// The tray being edited.
    selected: usize,
    /// Name field (new tray name or rename).
    name: String,
}

impl Shell {
    /// The trays and their panels; save it when [`Event::TrayLayoutChanged`] comes.
    pub fn tray_layout(&self) -> &TrayLayout {
        &self.trays
    }

    /// Restores trays saved from [`Shell::tray_layout`]. Panels no longer in the spec
    /// are dropped.
    pub fn set_tray_layout(&mut self, layout: TrayLayout) {
        let known: Vec<String> = self.panels.iter().map(|p| p.id.clone()).collect();
        self.trays = layout.validated(&known, &self.tray_default);
        self.apply_active_tray();
    }

    /// Name of the docked tray.
    pub fn active_tray_name(&self) -> &str {
        &self.trays.trays[self.trays.active].name
    }

    /// Opens Window > Manage Trays (the shell does this itself for the
    /// `window.manage_trays` command).
    pub fn open_trays_dialog(&mut self) {
        self.trays_dialog = Some(TraysDialog {
            selected: self.trays.active,
            name: String::new(),
        });
    }

    /// Shows the panels of the active tray.
    pub(crate) fn apply_active_tray(&mut self) {
        let tray = &self.trays.trays[self.trays.active];
        for p in &mut self.panels {
            p.shown = tray.panels.contains(&p.id);
        }
    }

    /// Docks tray `i`.
    pub(crate) fn switch_tray(&mut self, i: usize) {
        if i < self.trays.trays.len() && i != self.trays.active {
            self.trays.active = i;
            self.apply_active_tray();
            self.tray_visible = true;
            self.pending.push(Event::TrayLayoutChanged);
        }
    }

    /// The tray holding panel `id`.
    #[cfg(test)]
    pub(crate) fn tray_of(&self, id: &str) -> Option<usize> {
        self.trays
            .trays
            .iter()
            .position(|t| t.panels.iter().any(|p| p == id))
    }

    /// Moves panel `id` out of its tray into tray `to`, or into a new floating tray at
    /// `pos` when `to` is `None` (Make Tray).
    pub(crate) fn move_panel(&mut self, id: &str, to: Option<usize>, pos: Option<[f32; 2]>) {
        let to = match to {
            Some(t) if t < self.trays.trays.len() => t,
            Some(_) => return,
            None => self.make_tray(pos),
        };
        for (i, t) in self.trays.trays.iter_mut().enumerate() {
            if i != to {
                t.panels.retain(|p| p != id);
            }
        }
        let t = &mut self.trays.trays[to];
        if !t.panels.iter().any(|p| p == id) {
            t.panels.push(id.to_owned());
        }
        if let Some(p) = self.panels.iter_mut().find(|p| p.id == id) {
            p.expanded = true;
        }
        self.apply_active_tray();
        self.pending.push(Event::TrayLayoutChanged);
    }

    /// Make Tray: a new, empty floating tray at `pos`; returns its index.
    pub(crate) fn make_tray(&mut self, pos: Option<[f32; 2]>) -> usize {
        let name = self.trays.unique_name("Tray");
        self.trays.trays.push(TrayDef {
            name,
            panels: Vec::new(),
            floating: true,
            pos,
        });
        self.pending.push(Event::TrayLayoutChanged);
        self.trays.trays.len() - 1
    }

    /// Floats (`on`) or docks tray `i`. Docking makes it the docked tray; the Default
    /// Tray does not float; floating the docked tray docks the Default Tray.
    pub(crate) fn set_tray_floating(&mut self, i: usize, on: bool) {
        if i == 0 && on || i >= self.trays.trays.len() {
            return;
        }
        if on {
            self.trays.trays[i].floating = true;
            if self.trays.active == i {
                self.trays.active = 0;
                self.apply_active_tray();
            }
        } else {
            self.trays.trays[i].floating = false;
            self.switch_tray(i);
        }
        self.pending.push(Event::TrayLayoutChanged);
    }

    /// The right-click menu of tray `tray`'s title: its panels, Float / Dock, Make
    /// Tray.
    pub(crate) fn tray_menu(&mut self, response: &egui::Response, tray: usize) {
        let panels: Vec<(String, String)> = self
            .panels
            .iter()
            .map(|p| (p.id.clone(), p.title.clone()))
            .collect();
        response.context_menu(|ui| {
            self.panel_checks(ui, tray, &panels);
            ui.separator();
            self.float_dock_items(ui, tray);
        });
    }

    /// The right-click menu of panel `id`'s header in tray `tray`.
    pub(crate) fn panel_menu(&mut self, response: &egui::Response, id: &str, tray: usize) {
        let panels: Vec<(String, String)> = self
            .panels
            .iter()
            .map(|p| (p.id.clone(), p.title.clone()))
            .collect();
        let names: Vec<String> = self.trays.trays.iter().map(|t| t.name.clone()).collect();
        response.context_menu(|ui| {
            ui.menu_button("Move To", |ui| {
                for (i, n) in names.iter().enumerate() {
                    if i != tray && ui.button(n).clicked() {
                        self.move_panel(id, Some(i), None);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("New Floating Tray").clicked() {
                    self.move_panel(id, None, None);
                    ui.close();
                }
            });
            ui.menu_button("Panels", |ui| self.panel_checks(ui, tray, &panels));
            ui.separator();
            self.float_dock_items(ui, tray);
        });
    }

    fn panel_checks(&mut self, ui: &mut egui::Ui, tray: usize, panels: &[(String, String)]) {
        for (id, title) in panels {
            let mut on = self.trays.trays[tray].panels.contains(id);
            if ui.checkbox(&mut on, title).changed() {
                if on {
                    self.move_panel(id, Some(tray), None);
                } else if self.set_panel_in(tray, id, false) {
                    self.apply_active_tray();
                }
            }
        }
    }

    fn float_dock_items(&mut self, ui: &mut egui::Ui, tray: usize) {
        let floating = self.trays.trays[tray].floating;
        if floating {
            if ui.button("Dock").clicked() {
                self.set_tray_floating(tray, false);
                ui.close();
            }
        } else if ui
            .add_enabled(tray != 0, egui::Button::new("Float"))
            .on_disabled_hover_text("The Default Tray stays docked")
            .clicked()
        {
            self.set_tray_floating(tray, true);
            ui.close();
        }
        if ui.button("Make Tray").clicked() {
            self.make_tray(None);
            ui.close();
        }
        if ui.button("Manage Trays...").clicked() {
            self.open_trays_dialog();
            ui.close();
        }
    }

    /// Dragging a panel's header: a ghost follows the pointer; on release the panel
    /// moves to the tray under the pointer, or into a new floating tray when dropped
    /// outside every tray.
    pub(crate) fn panel_header_drag(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        id: &str,
        tray: usize,
    ) {
        if response.drag_started() {
            self.panel_drag = Some((id.to_owned(), tray));
        }
        let Some(pos) = ui.ctx().pointer_latest_pos() else {
            return;
        };
        if response.dragged() {
            let title = self
                .panels
                .iter()
                .find(|p| p.id == id)
                .map_or(id, |p| p.title.as_str());
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("mt_ui_panel_drag"),
            ));
            let r = egui::Rect::from_min_size(pos + egui::vec2(8.0, 4.0), egui::vec2(160.0, 22.0));
            painter.rect(
                r,
                2.0,
                self.theme.panel_header.gamma_multiply(0.9),
                egui::Stroke::new(1.0, self.theme.hover_border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                r.left_center() + egui::vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(self.theme.sizes.font),
                self.theme.text,
            );
        }
        if response.drag_stopped()
            && let Some((id, from)) = self.panel_drag.take()
        {
            match drop_tray(&self.last_tray_rects, pos) {
                Some(t) if t == from => {}
                Some(t) => self.move_panel(&id, Some(t), None),
                None => self.move_panel(&id, None, Some([pos.x - 20.0, pos.y - 10.0])),
            }
        }
    }

    /// The floating trays, as windows.
    pub(crate) fn floating_trays(&mut self, ctx: &egui::Context, frame: &crate::Frame) {
        let t = self.theme.clone();
        for ti in 0..self.trays.trays.len() {
            if !self.trays.trays[ti].floating {
                continue;
            }
            let name = self.trays.trays[ti].name.clone();
            let mut area = egui::Window::new(&name)
                .id(egui::Id::new(("mt_ui_floating_tray", ti)))
                .title_bar(false)
                .resizable(true)
                .default_width(frame.spec.tray.width)
                .default_height(360.0)
                .frame(
                    egui::Frame::window(&ctx.global_style())
                        .fill(t.tray)
                        .inner_margin(egui::Margin::ZERO),
                );
            let [x, y] = self.trays.trays[ti].pos.unwrap_or([320.0, 180.0]);
            area = area.default_pos(egui::pos2(x, y));
            let mut close = false;
            let shown = area.show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                let h = t.sizes.panel_header_height;
                let (bar, bar_response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), h + 2.0),
                    egui::Sense::click(),
                );
                ui.painter().rect_filled(bar, 0.0, t.tray_title);
                ui.painter().text(
                    egui::pos2(bar.left() + 8.0, bar.center().y),
                    egui::Align2::LEFT_CENTER,
                    &name,
                    egui::FontId::proportional(t.sizes.font),
                    t.text,
                );
                let x = egui::Rect::from_center_size(
                    egui::pos2(bar.right() - 12.0, bar.center().y),
                    egui::vec2(16.0, 16.0),
                );
                close = crate::tray::close_button(ui, x, ("mt_ui_ftray_close", ti), &t).clicked();
                if bar_response.double_clicked() {
                    self.set_tray_floating(ti, false);
                }
                self.tray_menu(&bar_response, ti);
                egui::ScrollArea::vertical()
                    .id_salt(("mt_ui_ftray_scroll", ti))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        for i in 0..self.panels.len() {
                            let id = self.panels[i].id.clone();
                            if self
                                .trays
                                .trays
                                .get(ti)
                                .is_some_and(|t| t.panels.contains(&id))
                            {
                                self.panel(ui, frame, i, ti);
                            }
                        }
                        if self
                            .trays
                            .trays
                            .get(ti)
                            .is_some_and(|t| t.panels.is_empty())
                        {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("  Drag panels here.").color(t.text_disabled),
                            );
                            ui.add_space(6.0);
                        }
                    });
            });
            if let Some(r) = shown {
                let rect = r.response.rect;
                self.tray_rects.push((ti, rect));
                let pos = [rect.left(), rect.top()];
                if let Some(tray) = self.trays.trays.get_mut(ti)
                    && tray.floating
                    && tray.pos != Some(pos)
                {
                    tray.pos = Some(pos);
                    if r.response.drag_stopped() {
                        self.pending.push(Event::TrayLayoutChanged);
                    }
                }
            }
            if close && let Some(tray) = self.trays.trays.get_mut(ti) {
                // Closing a floating tray hides it: it docks hidden (Manage Trays or
                // Window > a panel shows it again).
                tray.floating = false;
                self.pending.push(Event::TrayLayoutChanged);
            }
        }
    }

    /// Adds (`on`) or removes panel `id` in the active tray.
    pub(crate) fn set_panel_in_tray(&mut self, id: &str, on: bool) {
        let active = self.trays.active;
        if self.set_panel_in(active, id, on) {
            self.apply_active_tray();
        }
    }

    pub(crate) fn set_panel_in(&mut self, tray: usize, id: &str, on: bool) -> bool {
        let Some(t) = self.trays.trays.get_mut(tray) else {
            return false;
        };
        let has = t.panels.iter().any(|p| p == id);
        if has == on {
            return false;
        }
        if on {
            t.panels.push(id.to_owned());
            // A panel in a floating tray is in no other tray.
            let floating = t.floating;
            for (i, other) in self.trays.trays.iter_mut().enumerate() {
                if i != tray && (floating || other.floating) {
                    other.panels.retain(|p| p != id);
                }
            }
        } else {
            t.panels.retain(|p| p != id);
        }
        self.pending.push(Event::TrayLayoutChanged);
        true
    }

    pub(crate) fn trays_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.trays_dialog.take() else {
            return;
        };
        d.selected = d.selected.min(self.trays.trays.len() - 1);
        let panels: Vec<(String, String)> = self
            .panels
            .iter()
            .map(|p| (p.id.clone(), p.title.clone()))
            .collect();
        let r = crate::dialog::Dialog::new("manage_trays", "Manage Trays")
            .width(380.0)
            .show(ctx, |ui| {
                // The shell draws flat widgets; the name field needs a border.
                ui.visuals_mut().widgets.inactive.bg_stroke =
                    egui::Stroke::new(1.0, egui::Color32::from_gray(150));
                ui.visuals_mut().extreme_bg_color = egui::Color32::WHITE;
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(180.0);
                        ui.strong("Trays");
                        for (i, t) in self.trays.trays.iter().enumerate() {
                            let label = if i == self.trays.active {
                                format!("{}  (shown)", t.name)
                            } else {
                                t.name.clone()
                            };
                            if ui.selectable_label(d.selected == i, label).clicked() {
                                d.selected = i;
                            }
                        }
                        ui.add_space(6.0);
                        ui.label("Name:");
                        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(170.0));
                        ui.horizontal(|ui| {
                            let name = d.name.trim().to_owned();
                            if ui
                                .add_enabled(!name.is_empty(), egui::Button::new("New"))
                                .on_hover_text("A new tray with the name above")
                                .clicked()
                            {
                                let name = self.trays.unique_name(&name);
                                self.trays.trays.push(TrayDef {
                                    name,
                                    ..Default::default()
                                });
                                d.selected = self.trays.trays.len() - 1;
                                d.name.clear();
                                self.pending.push(Event::TrayLayoutChanged);
                            }
                            if ui
                                .add_enabled(!name.is_empty(), egui::Button::new("Rename"))
                                .clicked()
                                && self.trays.trays[d.selected].name != name
                            {
                                let name = self.trays.unique_name(&name);
                                self.trays.trays[d.selected].name = name;
                                d.name.clear();
                                self.pending.push(Event::TrayLayoutChanged);
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(d.selected != 0, egui::Button::new("Delete"))
                                .on_hover_text("The Default Tray cannot be deleted")
                                .clicked()
                            {
                                self.trays.trays.remove(d.selected);
                                if self.trays.active == d.selected {
                                    self.trays.active = 0;
                                } else if self.trays.active > d.selected {
                                    self.trays.active -= 1;
                                }
                                d.selected = d.selected.saturating_sub(1);
                                self.apply_active_tray();
                                self.pending.push(Event::TrayLayoutChanged);
                            }
                            if ui
                                .add_enabled(
                                    d.selected != self.trays.active,
                                    egui::Button::new("Show"),
                                )
                                .clicked()
                            {
                                self.switch_tray(d.selected);
                            }
                        });
                    });
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(180.0);
                        ui.strong(format!("Panels of {}", self.trays.trays[d.selected].name));
                        for (id, title) in &panels {
                            let mut on = self.trays.trays[d.selected].panels.contains(id);
                            if ui.checkbox(&mut on, title).changed()
                                && self.set_panel_in(d.selected, id, on)
                                && d.selected == self.trays.active
                            {
                                self.apply_active_tray();
                            }
                        }
                    });
                });
                crate::dialog::buttons(ui, &["Close"], None)
            });
        if !r.closed && r.inner.is_none() {
            self.trays_dialog = Some(d);
        }
    }
}

/// The tray whose rect (latest drawn last, so floating windows win) holds `pos`.
pub(crate) fn drop_tray(rects: &[(usize, egui::Rect)], pos: egui::Pos2) -> Option<usize> {
    rects
        .iter()
        .rev()
        .find(|(_, r)| r.contains(pos))
        .map(|(i, _)| *i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> Shell {
        Shell::new(&mt_spec::Spec::builtin().unwrap())
    }

    #[test]
    fn trays_switch_panels_and_validate() {
        let mut s = shell();
        assert_eq!(s.tray_layout().trays.len(), 1);
        assert!(s.panel_expanded("window.entity_info"));
        let mut layout = s.tray_layout().clone();
        layout.trays.push(TrayDef {
            name: "Drawing".into(),
            panels: vec!["window.materials".into(), "window.nope".into()],
            ..Default::default()
        });
        layout.active = 1;
        s.set_tray_layout(layout);
        assert_eq!(s.active_tray_name(), "Drawing");
        assert_eq!(s.tray_layout().trays[1].panels, ["window.materials"]);
        assert!(
            !s.panels
                .iter()
                .any(|p| p.shown && p.id == "window.entity_info")
        );
        // Window > Entity Info adds it to the shown tray.
        s.command("window.entity_info");
        assert!(
            s.tray_layout().trays[1]
                .panels
                .contains(&"window.entity_info".into())
        );
        assert!(s.pending.contains(&Event::TrayLayoutChanged));
        s.switch_tray(0);
        assert!(s.panel_expanded("window.entity_info"));
        // Bad input falls back to the default.
        s.set_tray_layout(TrayLayout {
            trays: Vec::new(),
            active: 5,
        });
        assert_eq!(s.tray_layout(), &s.tray_default.clone());
        // Round trip through TOML.
        let text = toml::to_string(s.tray_layout()).unwrap();
        let back: TrayLayout = toml::from_str(&text).unwrap();
        assert_eq!(&back, s.tray_layout());
        assert_eq!(
            s.tray_layout().unique_name(&back.trays[0].name),
            format!("{} 2", back.trays[0].name)
        );
    }

    #[test]
    fn panels_move_between_trays_and_float() {
        let mut s = shell();
        s.pending.clear();
        // Make Tray by moving Materials out: a new floating tray holds it alone.
        s.move_panel("window.materials", None, Some([100.0, 120.0]));
        let t = s.tray_of("window.materials").unwrap();
        assert_ne!(t, 0);
        assert!(s.trays.trays[t].floating);
        assert_eq!(s.trays.trays[t].pos, Some([100.0, 120.0]));
        assert!(!s.trays.trays[0].panels.contains(&"window.materials".into()));
        assert!(
            s.panel_expanded("window.materials"),
            "floating panels count as open"
        );
        assert!(s.pending.contains(&Event::TrayLayoutChanged));
        // Back into the Default Tray (a panel is in one tray at a time).
        s.move_panel("window.materials", Some(0), None);
        assert_eq!(s.tray_of("window.materials"), Some(0));
        assert!(s.trays.trays[t].panels.is_empty());
        // Window > Materials expands it where it is.
        s.set_panel_in(t, "window.tags", true);
        assert_eq!(s.tray_of("window.tags"), Some(t));
        assert!(s.open_panel("window.tags"));
        assert_eq!(s.tray_of("window.tags"), Some(t));
        // Docking the floating tray makes it the docked one; the Default Tray never floats.
        s.set_tray_floating(t, false);
        assert_eq!(s.trays.active, t);
        assert!(!s.trays.trays[t].floating);
        s.set_tray_floating(t, true);
        assert_eq!(s.trays.active, 0);
        s.set_tray_floating(0, true);
        assert!(!s.trays.trays[0].floating);
        // Saved layouts keep floating trays; bad ones are fixed.
        let mut bad = s.tray_layout().clone();
        bad.trays[0].floating = true;
        bad.active = t;
        bad.trays[t].panels.push("window.entity_info".into());
        s.set_tray_layout(bad);
        assert!(!s.trays.trays[0].floating);
        assert_eq!(s.trays.active, 0);
        // The floating tray keeps a panel listed twice.
        assert_eq!(s.tray_of("window.entity_info"), Some(t));
        assert!(
            !s.trays.trays[0]
                .panels
                .contains(&"window.entity_info".into())
        );
        let text = toml::to_string(s.tray_layout()).unwrap();
        let back: TrayLayout = toml::from_str(&text).unwrap();
        assert_eq!(&back, s.tray_layout());
        // Old layouts without the new fields load.
        let old: TrayLayout =
            toml::from_str("active = 0\n[[trays]]\nname = \"A\"\npanels = []\n").unwrap();
        assert!(!old.trays[0].floating);
        // Drops go to the topmost tray under the pointer.
        let r = |x: f32| egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(100.0, 100.0));
        let rects = [(0, r(0.0)), (2, r(50.0))];
        assert_eq!(drop_tray(&rects, egui::pos2(60.0, 10.0)), Some(2));
        assert_eq!(drop_tray(&rects, egui::pos2(10.0, 10.0)), Some(0));
        assert_eq!(drop_tray(&rects, egui::pos2(500.0, 10.0)), None);
    }

    #[test]
    fn reset_workspace_and_icon_size() {
        let spec = mt_spec::Spec::builtin().unwrap();
        let mut s = Shell::new(&spec);
        let mut layout = s.tray_layout().clone();
        layout.trays.push(TrayDef {
            name: "Mine".into(),
            panels: vec!["window.materials".into()],
            ..Default::default()
        });
        layout.active = 1;
        s.set_tray_layout(layout);
        s.set_toolbar_visible("standard", true);
        s.set_tray_visible(false);
        s.pending.clear();
        s.reset_workspace();
        assert_eq!(s.tray_layout(), &s.tray_default.clone());
        assert!(s.tray_visible());
        assert_eq!(s.toolbar_layout(), &s.toolbar_defaults.clone());
        assert!(s.pending.contains(&Event::TrayLayoutChanged));
        assert!(s.pending.contains(&Event::ToolbarLayoutChanged));
        assert!(s.large_icons(&spec));
        s.set_large_icons(&spec, false);
        assert!(!s.large_icons(&spec));
        assert_eq!(s.theme.sizes.toolbar_icon, crate::SMALL_ICON);
        s.set_large_icons(&spec, true);
        assert!(s.large_icons(&spec));
    }
}
