//! Window > Manage Trays: named trays, each with its own set of panels.
//!
//! One tray is docked at a time (on the right, as the Default Tray); the others are
//! kept and can be switched to from the tray's title bar or the dialog. SketchUp shows
//! several trays at once, docked side by side or floating (unverified details); that
//! is not done here. The first tray is the Default Tray from `spec/tray.toml`; it can
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrayDef {
    pub name: String,
    pub panels: Vec<String>,
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

    /// Adds (`on`) or removes panel `id` in the active tray.
    pub(crate) fn set_panel_in_tray(&mut self, id: &str, on: bool) {
        let active = self.trays.active;
        if self.set_panel_in(active, id, on) {
            self.apply_active_tray();
        }
    }

    fn set_panel_in(&mut self, tray: usize, id: &str, on: bool) -> bool {
        let Some(t) = self.trays.trays.get_mut(tray) else {
            return false;
        };
        let has = t.panels.iter().any(|p| p == id);
        if has == on {
            return false;
        }
        if on {
            t.panels.push(id.to_owned());
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
        let mut open = true;
        let mut close = false;
        let panels: Vec<(String, String)> = self
            .panels
            .iter()
            .map(|p| (p.id.clone(), p.title.clone()))
            .collect();
        egui::Window::new("Manage Trays")
            .id(egui::Id::new("mt_ui_trays_dialog"))
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
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
                                    panels: Vec::new(),
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
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close = ui.button("Close").clicked();
                });
            });
        if open && !close {
            self.trays_dialog = Some(d);
        }
    }
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
}
