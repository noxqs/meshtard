//! The Environments and Instructor panels (FEATURES.md 66).
//!
//! Environments (help page "Adding an environment to models", [doc]): a preview of
//! the model's environment, the library of environments (thumbnails), and the Edit
//! settings: Use as Skydome, Skydome Exposure, Use for Reflections, Reflection
//! Exposure, Rotation and Set Sun Location (click the preview where the sun is; that
//! direction then overrides the Shadows panel's sun). The panel's layout, slider
//! ranges and button wording are from memory (unverified). The app owns the images
//! and passes [`EnvironmentData`] (thumbnails as egui textures) to
//! [`Shell::set_environment_data`]; changes come back as events.
//!
//! Instructor: the active tool's entry of `spec/instructor.toml`.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::{Event, Shell};

/// One environment of the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentRow {
    pub name: String,
    /// A small preview texture (2:1), if the app made one.
    pub thumbnail: Option<egui::TextureId>,
    /// Added by the user (Remove from library allowed); the bundled ones are not.
    pub removable: bool,
}

/// The Edit settings of the model's environment, in whole units for the widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvSettings {
    pub skydome: bool,
    pub reflections: bool,
    /// Degrees, 0..360.
    pub rotation_deg: i32,
    /// Percent (100 = as recorded).
    pub skydome_exposure: i32,
    pub reflection_exposure: i32,
    /// Set Sun Location: panorama point (u, v) in thousandths; `None` = the Shadows
    /// panel's sun.
    pub sun: Option<[u16; 2]>,
}

impl Default for EnvSettings {
    fn default() -> Self {
        Self {
            skydome: true,
            reflections: true,
            rotation_deg: 0,
            skydome_exposure: 100,
            reflection_exposure: 100,
            sun: None,
        }
    }
}

/// What the Environments panel shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvironmentData {
    pub rows: Vec<EnvironmentRow>,
    /// The model's environment (index into `rows`); `None` = no environment.
    pub current: Option<usize>,
    pub settings: EnvSettings,
    /// A larger preview of the current environment and its size in pixels.
    pub preview: Option<(egui::TextureId, [u32; 2])>,
}

/// Exposure slider range in percent (unverified; SketchUp shows a plain slider).
pub const EXPOSURE_RANGE: std::ops::RangeInclusive<i32> = 0..=400;

/// The panorama point (thousandths) under `pos` in the preview `rect`.
pub fn sun_from_click(rect: Rect, pos: egui::Pos2) -> [u16; 2] {
    let u = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
    let v = ((pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
    [(u * 1000.0).round() as u16, (v * 1000.0).round() as u16]
}

impl Shell {
    /// Sets what the Environments panel shows (call before [`Shell::show`]).
    pub fn set_environment_data(&mut self, data: EnvironmentData) {
        self.env_data = data;
    }

    pub(crate) fn environments_panel(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        let data = self.env_data.clone();
        let width = ui.available_width();
        // Preview: the current environment, 2:1, sun marker; click sets the sun
        // while Set Sun Location is on.
        let size = vec2(width, (width / 2.0).round());
        let sense = if self.env_sun_mode {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        let painter = ui.painter_at(rect);
        match data.preview {
            Some((tex, _)) => {
                painter.image(
                    tex,
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                painter.rect_filled(rect, 0.0, Color32::from_gray(225));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No environment",
                    egui::FontId::proportional(t.sizes.font),
                    t.text_disabled,
                );
            }
        }
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
        let mut s = data.settings;
        if let Some([u, v]) = s.sun.filter(|_| data.preview.is_some()) {
            let p = pos2(
                rect.left() + f32::from(u) / 1000.0 * rect.width(),
                rect.top() + f32::from(v) / 1000.0 * rect.height(),
            );
            painter.circle(
                p,
                5.0,
                Color32::from_rgb(255, 210, 0),
                Stroke::new(1.5, Color32::BLACK),
            );
        }
        if self.env_sun_mode {
            let response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
            if let Some(pos) = response
                .interact_pointer_pos()
                .filter(|_| response.clicked())
            {
                s.sun = Some(sun_from_click(rect, pos));
                self.env_sun_mode = false;
            }
        }
        if let Some(i) = data.current {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(&data.rows[i].name).strong());
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Add Environment...")
                .on_hover_text("Add an HDR, EXR, PNG or JPEG panorama (2:1) to the library")
                .clicked()
            {
                self.pending.push(Event::EnvironmentAdd);
            }
            if ui
                .add_enabled(data.current.is_some(), egui::Button::new("None"))
                .on_hover_text("Use no environment")
                .clicked()
            {
                self.env_sun_mode = false;
                self.pending.push(Event::EnvironmentSelect(None));
            }
            let removable = data.current.filter(|&i| data.rows[i].removable);
            if ui
                .add_enabled(removable.is_some(), egui::Button::new("Remove"))
                .on_hover_text("Remove the environment from the library")
                .clicked()
                && let Some(i) = removable
            {
                self.pending.push(Event::EnvironmentRemove(i));
            }
        });

        // Library: thumbnails in a wrapping grid.
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Library").color(t.text_disabled));
        let thumb = vec2(84.0, 42.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for (i, row) in data.rows.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(thumb, Sense::click());
                let p = ui.painter();
                match row.thumbnail {
                    Some(tex) => {
                        p.image(
                            tex,
                            r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    None => {
                        p.rect_filled(r, 0.0, Color32::from_gray(200));
                    }
                }
                let selected = data.current == Some(i);
                let stroke = if selected {
                    Stroke::new(2.0, t.hover_border)
                } else if resp.hovered() {
                    Stroke::new(1.0, t.hover_border)
                } else {
                    Stroke::new(1.0, t.border)
                };
                p.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
                if resp.on_hover_text(&row.name).clicked() && !selected {
                    self.env_sun_mode = false;
                    self.pending.push(Event::EnvironmentSelect(Some(i)));
                }
            }
        });

        // Edit.
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Edit").color(t.text_disabled));
        let on = data.current.is_some();
        // Narrow enough that label + slider + value fit: a wider grid would widen the tray.
        let slider_w = (ui.available_width() - 200.0).max(40.0);
        ui.add_enabled_ui(on, |ui| {
            ui.checkbox(&mut s.skydome, "Use as Skydome");
            egui::Grid::new("mt_ui_env_edit")
                .num_columns(2)
                .spacing(vec2(6.0, 4.0))
                .show(ui, |ui| {
                    ui.spacing_mut().slider_width = slider_w;
                    ui.label("Skydome Exposure");
                    ui.add_enabled(
                        s.skydome,
                        egui::Slider::new(&mut s.skydome_exposure, EXPOSURE_RANGE).suffix("%"),
                    );
                    ui.end_row();
                });
            ui.checkbox(&mut s.reflections, "Use for Reflections");
            egui::Grid::new("mt_ui_env_edit2")
                .num_columns(2)
                .spacing(vec2(6.0, 4.0))
                .show(ui, |ui| {
                    ui.spacing_mut().slider_width = slider_w;
                    ui.label("Reflection Exposure");
                    ui.add_enabled(
                        s.reflections,
                        egui::Slider::new(&mut s.reflection_exposure, EXPOSURE_RANGE).suffix("%"),
                    );
                    ui.end_row();
                    ui.label("Rotation");
                    ui.add(egui::Slider::new(&mut s.rotation_deg, 0..=359).suffix("\u{b0}"));
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                let label = if self.env_sun_mode {
                    "Click the preview..."
                } else {
                    "Set Sun Location"
                };
                if ui
                    .selectable_label(self.env_sun_mode, label)
                    .on_hover_text(
                        "Click where the sun is in the preview above; the shadows then come \
                         from there",
                    )
                    .clicked()
                {
                    self.env_sun_mode = !self.env_sun_mode;
                }
                if ui
                    .add_enabled(s.sun.is_some(), egui::Button::new("Clear Sun"))
                    .on_hover_text("Use the Shadows panel's sun again")
                    .clicked()
                {
                    s.sun = None;
                }
            });
        });
        if s != data.settings {
            self.env_data.settings = s;
            self.pending.push(Event::EnvironmentChanged(s));
        }
    }

    /// Instructor: help for the active tool from `spec/instructor.toml`.
    pub(crate) fn instructor_panel(&mut self, ui: &mut egui::Ui, active_tool: &str) {
        let t = self.theme.clone();
        let Some(help) = self.instructor.as_ref().and_then(|i| i.get(active_tool)) else {
            ui.label(egui::RichText::new("No help for this tool yet.").color(t.text_disabled));
            return;
        };
        ui.label(
            egui::RichText::new(&help.title)
                .strong()
                .size(t.sizes.font + 2.0),
        );
        ui.label(&help.summary);
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Tool operation").strong());
        for (i, step) in help.steps.iter().enumerate() {
            ui.label(format!("{}. {step}", i + 1));
        }
        if !help.modifiers.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Modifier keys").strong());
            for m in &help.modifiers {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(&m.key).strong());
                    ui.label(&m.action);
                });
            }
        }
        if let Some(tip) = &help.tip {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("Tip: {tip}")).italics());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_click_maps_to_thousandths() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 100.0));
        assert_eq!(sun_from_click(r, pos2(10.0, 20.0)), [0, 0]);
        assert_eq!(sun_from_click(r, pos2(110.0, 45.0)), [500, 250]);
        assert_eq!(sun_from_click(r, pos2(500.0, -5.0)), [1000, 0]);
    }

    #[test]
    fn instructor_is_loaded_for_tools() {
        let s = Shell::new(&mt_spec::Spec::builtin().unwrap());
        let i = s.instructor.as_ref().unwrap();
        assert!(i.get("tool.line").is_some());
    }
}
