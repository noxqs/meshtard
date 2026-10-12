//! Styles panel, Edit tab extras (FEATURES.md item 65): edge color, endpoints and
//! jitter; face front/back colors, transparency and ambient occlusion; the Background
//! and Watermark pages; modeling colors. Layout and wording follow SketchUp's
//! Styles help pages as remembered (unverified); SketchUp uses icon buttons for the
//! pages, text tabs are used here.

use egui::RichText;

use crate::tray_data::TrayData;
use crate::{Event, Shell};

/// Edge color modes, in [`StyleExtras::edge_color_mode`] order.
pub const EDGE_COLOR_MODES: [&str; 3] = ["All same", "By material", "By axis"];

/// Watermark placements, in [`WatermarkRow::mode`] order.
pub const WATERMARK_MODES: [&str; 3] = ["Stretched", "Tiled", "Positioned"];

/// Positioned watermark anchors, in [`WatermarkRow::anchor`] order (row by row from
/// the top left).
pub const WATERMARK_ANCHORS: [&str; 9] = [
    "Top left",
    "Top",
    "Top right",
    "Left",
    "Center",
    "Right",
    "Bottom left",
    "Bottom",
    "Bottom right",
];

/// The Styles panel settings beyond [`crate::StyleSettings`]. Percentages are 0..=100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleExtras {
    /// Index into [`EDGE_COLOR_MODES`].
    pub edge_color_mode: usize,
    pub edge_color: [u8; 3],
    pub endpoints: bool,
    pub endpoint_length: u8,
    pub jitter: bool,
    pub front_color: [u8; 3],
    pub back_color: [u8; 3],
    pub transparency: bool,
    /// Transparency quality: Nicer (else Faster).
    pub nicer: bool,
    pub ambient_occlusion: bool,
    pub ao_intensity: u8,
    /// Occluder distance in whole inches.
    pub ao_distance: u16,
    pub ao_use_color: bool,
    pub ao_color: [u8; 3],
    pub background: [u8; 3],
    pub sky: bool,
    pub sky_color: [u8; 3],
    pub horizon_color: [u8; 3],
    /// Horizon blend, 1..=100 (how high the horizon color reaches; 50 default).
    pub horizon_blend: u8,
    pub ground: bool,
    pub ground_color: [u8; 3],
    pub ground_transparency: u8,
    pub ground_from_below: bool,
    pub selected: [u8; 3],
    pub locked: [u8; 3],
    pub guides: [u8; 3],
    pub section_active: [u8; 3],
    pub section_inactive: [u8; 3],
    pub section_cut: [u8; 3],
    /// Hidden geometry color.
    pub hidden_geometry: [u8; 3],
    /// Inference marker colors: endpoint, midpoint, on edge, on face.
    pub inference: [[u8; 3]; 4],
}

/// The inference colors of [`StyleExtras::inference`], in order.
pub const INFERENCE_COLORS: [&str; 4] = ["Endpoint", "Midpoint", "On Edge", "On Face"];

impl Default for StyleExtras {
    fn default() -> Self {
        Self {
            edge_color_mode: 0,
            edge_color: [0, 0, 0],
            endpoints: false,
            endpoint_length: 7,
            jitter: false,
            front_color: [255, 255, 255],
            back_color: [164, 178, 187],
            transparency: true,
            nicer: true,
            ambient_occlusion: false,
            ao_intensity: 50,
            ao_distance: 12,
            ao_use_color: false,
            ao_color: [0, 0, 0],
            background: [255, 255, 255],
            sky: true,
            sky_color: [163, 199, 235],
            horizon_color: [237, 242, 247],
            horizon_blend: 50,
            ground: true,
            ground_color: [212, 209, 199],
            ground_transparency: 0,
            ground_from_below: false,
            selected: [0, 0, 255],
            locked: [255, 0, 0],
            guides: [64, 64, 64],
            section_active: [242, 102, 13],
            section_inactive: [217, 140, 51],
            section_cut: [0, 0, 0],
            hidden_geometry: [115, 115, 115],
            inference: [[0, 200, 0], [0, 200, 220], [230, 0, 0], [0, 0, 230]],
        }
    }
}

/// One watermark of the Watermark page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatermarkRow {
    pub name: String,
    /// Overlay (else background).
    pub overlay: bool,
    /// Blend: opacity percentage.
    pub opacity: u8,
    /// Index into [`WATERMARK_MODES`].
    pub mode: usize,
    /// Stretched: lock aspect ratio.
    pub lock_aspect: bool,
    /// Tiled and Positioned: scale percentage (10..=400).
    pub scale: u16,
    /// Positioned: index into [`WATERMARK_ANCHORS`].
    pub anchor: usize,
    /// Create Mask: the image's brightness is the opacity of the background color.
    pub mask: bool,
}

/// Which extra rows the current Edit tab page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtrasPage {
    Edge,
    Face,
    Background,
    Watermark,
    Modeling,
}

fn color_row(ui: &mut egui::Ui, label: &str, c: &mut [u8; 3]) {
    ui.horizontal(|ui| {
        ui.color_edit_button_srgb(c);
        ui.label(label);
    });
}

fn checked_color(ui: &mut egui::Ui, label: &str, on: &mut bool, c: &mut [u8; 3]) {
    ui.horizontal(|ui| {
        ui.checkbox(on, label);
        ui.add_enabled_ui(*on, |ui| ui.color_edit_button_srgb(c));
    });
}

impl Shell {
    /// The rows of `page` beyond the View-menu toggles; pushes
    /// [`Event::StyleExtrasChanged`] (and the watermark events) on a change.
    pub(crate) fn style_extras_ui(
        &mut self,
        ui: &mut egui::Ui,
        data: &mut TrayData,
        page: ExtrasPage,
    ) {
        let before = data.style_extras;
        let s = &mut data.style_extras;
        match page {
            ExtrasPage::Edge => {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.endpoints, "Endpoints");
                    ui.add_enabled(
                        s.endpoints,
                        egui::DragValue::new(&mut s.endpoint_length).range(1..=99),
                    );
                });
                ui.checkbox(&mut s.jitter, "Jitter");
                ui.horizontal(|ui| {
                    ui.label("Color:");
                    egui::ComboBox::from_id_salt("mt_ui_edge_color_mode")
                        .selected_text(EDGE_COLOR_MODES[s.edge_color_mode.min(2)])
                        .show_ui(ui, |ui| {
                            for (i, name) in EDGE_COLOR_MODES.iter().enumerate() {
                                ui.selectable_value(&mut s.edge_color_mode, i, *name);
                            }
                        });
                    ui.color_edit_button_srgb(&mut s.edge_color);
                });
            }
            ExtrasPage::Face => {
                color_row(ui, "Front color", &mut s.front_color);
                color_row(ui, "Back color", &mut s.back_color);
                ui.checkbox(&mut s.transparency, "Enable transparency");
                ui.add_enabled_ui(s.transparency, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Transparency quality:");
                        ui.radio_value(&mut s.nicer, false, "Faster");
                        ui.radio_value(&mut s.nicer, true, "Nicer");
                    });
                });
                ui.checkbox(&mut s.ambient_occlusion, "Ambient Occlusion");
                ui.add_enabled_ui(s.ambient_occlusion, |ui| {
                    egui::Grid::new("mt_ui_ao")
                        .num_columns(2)
                        .spacing([6.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Intensity");
                            ui.add(egui::Slider::new(&mut s.ao_intensity, 0..=100).suffix("%"));
                            ui.end_row();
                            ui.label("Distance");
                            ui.add(
                                egui::DragValue::new(&mut s.ao_distance)
                                    .range(1..=9999)
                                    .suffix("\""),
                            );
                            ui.end_row();
                        });
                    checked_color(ui, "Color", &mut s.ao_use_color, &mut s.ao_color);
                });
            }
            ExtrasPage::Background => {
                ui.label(RichText::new("Background Settings").strong());
                color_row(ui, "Background", &mut s.background);
                checked_color(ui, "Sky", &mut s.sky, &mut s.sky_color);
                ui.add_enabled_ui(s.sky, |ui| {
                    color_row(ui, "Horizon", &mut s.horizon_color);
                    ui.horizontal(|ui| {
                        ui.label("Horizon blend");
                        ui.add(egui::Slider::new(&mut s.horizon_blend, 1..=100));
                    });
                });
                checked_color(ui, "Ground", &mut s.ground, &mut s.ground_color);
                ui.add_enabled_ui(s.ground, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Transparency");
                        ui.add(egui::Slider::new(&mut s.ground_transparency, 0..=100));
                    });
                    ui.checkbox(&mut s.ground_from_below, "Show ground from below");
                });
            }
            ExtrasPage::Watermark => self.watermarks_ui(ui, data),
            ExtrasPage::Modeling => {
                for (label, c) in [
                    ("Selected", &mut s.selected),
                    ("Locked", &mut s.locked),
                    ("Guides", &mut s.guides),
                    ("Active section", &mut s.section_active),
                    ("Inactive section", &mut s.section_inactive),
                    ("Section cuts", &mut s.section_cut),
                    ("Hidden geometry", &mut s.hidden_geometry),
                ] {
                    color_row(ui, label, c);
                }
                ui.label(RichText::new("Inferences").strong());
                for (label, c) in INFERENCE_COLORS.iter().zip(&mut s.inference) {
                    color_row(ui, label, c);
                }
            }
        }
        if data.style_extras != before {
            self.pending
                .push(Event::StyleExtrasChanged(data.style_extras));
        }
    }

    /// The Watermark page: the list (overlays first, top one first, as SketchUp lists
    /// them above the model space, then the backgrounds), add, remove and move
    /// buttons, and the selected watermark's settings.
    fn watermarks_ui(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        ui.label(RichText::new("Watermark Settings").strong());
        ui.horizontal(|ui| {
            if ui.button("Add...").clicked() {
                self.pending.push(Event::WatermarkAdd);
            }
            let sel = self
                .tray_ui
                .more
                .watermark_selected
                .filter(|&i| i < data.watermarks.len());
            if ui
                .add_enabled(sel.is_some(), egui::Button::new("Remove"))
                .clicked()
                && let Some(i) = sel
            {
                self.pending.push(Event::WatermarkRemove(i));
                self.tray_ui.more.watermark_selected = None;
            }
            // Move within the overlays or within the backgrounds (the app answers with
            // the new list; the selection follows the moved watermark).
            for (up, label, tip) in [
                (true, "⏶", "Move watermark up"),
                (false, "⏷", "Move watermark down"),
            ] {
                let target = sel.and_then(|i| watermark_neighbor(&data.watermarks, i, up));
                if ui
                    .add_enabled(target.is_some(), egui::Button::new(label))
                    .on_hover_text(tip)
                    .clicked()
                    && let (Some(i), Some(j)) = (sel, target)
                {
                    self.pending.push(Event::WatermarkMove(i, up));
                    self.tray_ui.more.watermark_selected = Some(j);
                }
            }
        });
        if data.watermarks.is_empty() {
            ui.weak("No watermarks.");
            return;
        }
        for overlay in [true, false] {
            if !overlay {
                ui.weak("Model Space");
            }
            for (i, w) in data.watermarks.iter().enumerate().rev() {
                if w.overlay != overlay {
                    continue;
                }
                let label = format!(
                    "{} ({})",
                    w.name,
                    if w.overlay { "overlay" } else { "background" }
                );
                let on = self.tray_ui.more.watermark_selected == Some(i);
                if ui.selectable_label(on, label).clicked() {
                    self.tray_ui.more.watermark_selected = Some(i);
                }
            }
        }
        let Some(i) = self
            .tray_ui
            .more
            .watermark_selected
            .filter(|&i| i < data.watermarks.len())
        else {
            return;
        };
        ui.separator();
        let w = &mut data.watermarks[i];
        let before = w.clone();
        ui.horizontal(|ui| {
            ui.radio_value(&mut w.overlay, false, "Background");
            ui.radio_value(&mut w.overlay, true, "Overlay");
        });
        ui.checkbox(&mut w.mask, "Create Mask")
            .on_hover_text("Use the image's brightness as the opacity of the background color");
        ui.horizontal(|ui| {
            ui.label("Blend");
            ui.add(egui::Slider::new(&mut w.opacity, 0..=100).suffix("%"));
        });
        ui.horizontal(|ui| {
            for (k, name) in WATERMARK_MODES.iter().enumerate() {
                ui.radio_value(&mut w.mode, k, *name);
            }
        });
        match w.mode {
            0 => {
                ui.checkbox(&mut w.lock_aspect, "Lock aspect ratio");
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.label("Scale");
                    ui.add(egui::Slider::new(&mut w.scale, 10..=400).suffix("%"));
                });
                if w.mode == 2 {
                    egui::ComboBox::from_id_salt("mt_ui_wm_anchor")
                        .selected_text(WATERMARK_ANCHORS[w.anchor.min(8)])
                        .show_ui(ui, |ui| {
                            for (k, name) in WATERMARK_ANCHORS.iter().enumerate() {
                                ui.selectable_value(&mut w.anchor, k, *name);
                            }
                        });
                }
            }
        }
        if *w != before {
            self.pending.push(Event::WatermarkChanged(i, w.clone()));
        }
    }
}

/// The watermark that watermark `i` swaps with when moved up (drawn later) or down:
/// the next one of its own kind (overlay or background) in that direction.
pub(crate) fn watermark_neighbor(list: &[WatermarkRow], i: usize, up: bool) -> Option<usize> {
    let overlay = list.get(i)?.overlay;
    if up {
        (i + 1..list.len()).find(|&j| list[j].overlay == overlay)
    } else {
        (0..i).rev().find(|&j| list[j].overlay == overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbors_stay_within_overlays_or_backgrounds() {
        let row = |overlay| WatermarkRow {
            name: String::new(),
            overlay,
            opacity: 50,
            mode: 0,
            lock_aspect: true,
            scale: 100,
            anchor: 4,
            mask: false,
        };
        let l = [row(false), row(true), row(false), row(true)];
        assert_eq!(watermark_neighbor(&l, 0, true), Some(2));
        assert_eq!(watermark_neighbor(&l, 1, true), Some(3));
        assert_eq!(watermark_neighbor(&l, 3, true), None);
        assert_eq!(watermark_neighbor(&l, 2, false), Some(0));
        assert_eq!(watermark_neighbor(&l, 1, false), None);
        assert_eq!(watermark_neighbor(&l, 9, false), None);
    }
}
