//! Contents of the tray panels (Tags, Materials, Outliner, Scenes, Styles, Shadows)
//! and the scene tab strip above the viewport.
//!
//! Layouts follow SketchUp 2026 on Windows as remembered, simplified; details that
//! are unverified are noted on each function. Every [`Event`] here is pushed only in
//! response to a click, drag or typed text.

use egui::{
    Align2, Color32, FontId, Painter, Rect, Response, RichText, Sense, Stroke, StrokeKind, pos2,
    vec2,
};

use crate::theme::Theme32;
use crate::tray_data::{
    FaceStyle, TextureAction, TrayData, component_edit, date_label, hex_color, hsl_to_rgb,
    instances_label, parse_hex_color, rgb_to_hsl, time_label,
};
use crate::{Event, Shell};

/// Height of a list row in the panels.
pub(crate) const ROW: f32 = 20.0;
/// Size of the small flat icon buttons above lists.
const ICON_BUTTON: f32 = 22.0;

/// Panel-local UI state that is not model data.
#[derive(Debug, Default)]
pub(crate) struct TrayUi {
    material_tab: MaterialTab,
    /// Name field of the Materials panel and the material it was loaded from.
    material_name: String,
    material_name_of: Option<usize>,
    styles_page: StylesPage,
    component_tab: ComponentTab,
    /// Edit tab fields of the Components panel and the definition they were loaded
    /// from.
    component_name: String,
    component_description: String,
    component_fields_of: Option<usize>,
    /// Entity Info text field being edited: field key and text.
    pub(crate) info_edit: Option<(String, String)>,
    /// Entity Info number field being dragged: field key and value.
    pub(crate) info_drag: Option<(String, i64)>,
    /// Materials Edit tab: color picker page.
    color_mode: ColorMode,
    /// The HSL fields' values and the color they were last set to (keeps the hue of
    /// grays while editing).
    material_hsl: Option<([u8; 3], [f64; 3])>,
    /// HEX field text.
    material_hex: String,
    /// Texture width and height fields and the material they were loaded from.
    texture_w: String,
    texture_h: String,
    texture_of: Option<usize>,
    /// Texture size aspect lock (on by default).
    texture_unlocked: bool,
    /// State of the panel parts in `panels2` (tag folders, Outliner editing, scene
    /// details, In Model styles).
    pub(crate) more: crate::panels2::MoreUi,
}

/// Color picker pages of the Materials Edit tab (SketchUp: Color Wheel, HLS, HSB,
/// RGB; here Wheel is egui's saturation/value square with a hue bar).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    #[default]
    Wheel,
    Rgb,
    Hsl,
    Hex,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ComponentTab {
    #[default]
    Select,
    Edit,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum MaterialTab {
    #[default]
    Select,
    Edit,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum StylesPage {
    #[default]
    Edge,
    Face,
    Background,
    Watermark,
    Modeling,
}

impl Shell {
    /// Draws the body of tray panel `id` if it is one of the panels in this module.
    /// Returns false for panels without content.
    pub(crate) fn panel_body(&mut self, ui: &mut egui::Ui, id: &str) -> bool {
        let mut data = std::mem::take(&mut self.data);
        let known = match id {
            "window.tags" => {
                self.tags_panel2(ui, &mut data);
                true
            }
            "window.materials" => {
                self.materials_panel(ui, &mut data);
                true
            }
            "window.components" => {
                self.components_panel(ui, &mut data);
                true
            }
            "window.outliner" => {
                self.outliner_panel2(ui, &data);
                true
            }
            "window.scenes" => {
                self.scenes_panel2(ui, &data);
                true
            }
            "window.styles" => {
                self.styles_panel2(ui, &mut data);
                true
            }
            "window.shadows" => {
                self.shadows_panel(ui, &mut data);
                true
            }
            "window.fog" => {
                self.fog_panel(ui, &mut data);
                true
            }
            "window.soften_edges" => {
                self.soften_panel(ui, &mut data);
                true
            }
            _ => false,
        };
        self.data = data;
        known
    }

    /// Materials panel: preview swatch and name of the current material, then the
    /// Select tab ("In Model" swatch grid) and the Edit tab (color, opacity).
    ///
    /// Unverified: swatch size and spacing, the Edit tab's color picker (SketchUp
    /// has Color Wheel / HLS / HSB / RGB pickers; egui's picker is used), texture
    /// controls (left out), collection dropdown (only "In Model" exists here).
    fn materials_panel(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let t = self.theme.clone();
        let current = data.current_material.filter(|&i| i < data.materials.len());

        // Header: big swatch + name.
        let name_id = ui.id().with("mt_ui_material_name");
        let name_focused = ui.memory(|m| m.has_focus(name_id));
        if !name_focused || self.tray_ui.material_name_of != current {
            self.tray_ui.material_name_of = current;
            self.tray_ui.material_name = current
                .map(|i| data.materials[i].name.clone())
                .unwrap_or_else(|| "Default".to_owned());
        }
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            match current {
                Some(i) => paint_swatch(
                    ui.painter(),
                    rect,
                    data.materials[i].color,
                    data.materials[i].opacity,
                    &t,
                ),
                None => paint_default_swatch(ui.painter(), rect, &t),
            }
            ui.vertical(|ui| {
                let edit = egui::TextEdit::singleline(&mut self.tray_ui.material_name)
                    .id(name_id)
                    .desired_width(ui.available_width())
                    .interactive(current.is_some());
                let r = ui.add(edit);
                if r.lost_focus()
                    && let Some(i) = current
                {
                    let name = self.tray_ui.material_name.trim().to_owned();
                    let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if !cancel && !name.is_empty() && name != data.materials[i].name {
                        data.materials[i].name.clone_from(&name);
                        let m = &data.materials[i];
                        self.pending.push(Event::MaterialEdit {
                            index: i,
                            name,
                            color: m.color,
                            opacity: m.opacity,
                        });
                    }
                }
            });
        });
        ui.add_space(4.0);

        // Tabs.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (tab, label) in [(MaterialTab::Select, "Select"), (MaterialTab::Edit, "Edit")] {
                if tab_button(ui, &t, label, self.tray_ui.material_tab == tab).clicked() {
                    self.tray_ui.material_tab = tab;
                }
            }
        });

        match self.tray_ui.material_tab {
            MaterialTab::Select => {
                ui.horizontal(|ui| {
                    ui.label("In Model");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, &t, "Create Material", true, paint_plus).clicked() {
                            self.pending.push(Event::MaterialCreate);
                        }
                    });
                });
                // The Default material (no material) first, then the model's.
                let cells: Vec<Option<usize>> = std::iter::once(None)
                    .chain((0..data.materials.len()).map(Some))
                    .collect();
                let size = 40.0;
                let gap = 4.0;
                let w = ui.available_width();
                let per_row = (((w + gap) / (size + gap)).floor() as usize).max(1);
                for row in cells.chunks(per_row) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for &cell in row {
                            let (rect, r) =
                                ui.allocate_exact_size(vec2(size, size), Sense::click());
                            let name = match cell {
                                Some(i) => {
                                    let m = &data.materials[i];
                                    paint_swatch(
                                        ui.painter(),
                                        rect.shrink(2.0),
                                        m.color,
                                        m.opacity,
                                        &t,
                                    );
                                    m.name.as_str()
                                }
                                None => {
                                    paint_default_swatch(ui.painter(), rect.shrink(2.0), &t);
                                    "Default"
                                }
                            };
                            if current == cell {
                                ui.painter().rect_stroke(
                                    rect,
                                    0.0,
                                    Stroke::new(2.0, t.selection),
                                    StrokeKind::Inside,
                                );
                            } else if r.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    0.0,
                                    Stroke::new(1.0, t.hover_border),
                                    StrokeKind::Inside,
                                );
                            }
                            let r = r.on_hover_text(name);
                            if let Some(i) = cell {
                                r.context_menu(|ui| {
                                    if ui.button("Duplicate").clicked() {
                                        self.pending.push(Event::MaterialDuplicate(i));
                                        ui.close();
                                    }
                                    if ui.button("Area").clicked() {
                                        self.pending.push(Event::MaterialArea(i));
                                        ui.close();
                                    }
                                });
                            }
                            if r.clicked() {
                                data.current_material = cell;
                                self.pending.push(match cell {
                                    Some(i) => Event::MaterialPick(i),
                                    None => Event::MaterialDefault,
                                });
                            }
                        }
                    });
                    ui.add_space(gap - ui.spacing().item_spacing.y);
                }
            }
            MaterialTab::Edit => {
                let Some(i) = current else {
                    ui.label(
                        RichText::new("Pick a material in the Select tab to edit it.")
                            .color(t.text_disabled),
                    );
                    return;
                };
                let before = data.materials[i].clone();
                let m = &mut data.materials[i];
                ui.horizontal(|ui| {
                    ui.label("Color");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Back to the image's own colors (textured materials only).
                        if ui
                            .add_enabled(m.texture.is_some(), egui::Button::new("Reset Color"))
                            .clicked()
                        {
                            self.pending.push(Event::MaterialTexture {
                                index: i,
                                action: TextureAction::ResetColor,
                            });
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (mode, label) in [
                        (ColorMode::Wheel, "Wheel"),
                        (ColorMode::Rgb, "RGB"),
                        (ColorMode::Hsl, "HSL"),
                        (ColorMode::Hex, "HEX"),
                    ] {
                        if tab_button(ui, &t, label, self.tray_ui.color_mode == mode).clicked() {
                            self.tray_ui.color_mode = mode;
                        }
                    }
                });
                self.color_editor(ui, &mut m.color);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Opacity");
                    ui.spacing_mut().slider_width = (ui.available_width() - 50.0).max(60.0);
                    ui.add(egui::Slider::new(&mut m.opacity, 0..=100));
                });
                if *m != before {
                    self.pending.push(Event::MaterialEdit {
                        index: i,
                        name: m.name.clone(),
                        color: m.color,
                        opacity: m.opacity,
                    });
                }
                ui.add_space(4.0);
                self.texture_editor(ui, i, &*data);
            }
        }
    }

    /// The color picker page of the Materials Edit tab. Unverified: SketchUp's pages
    /// are Color Wheel, HLS, HSB and RGB; HEX is a field on them.
    fn color_editor(&mut self, ui: &mut egui::Ui, color: &mut [u8; 3]) {
        let slider_w = (ui.available_width() - 70.0).clamp(60.0, 220.0);
        match self.tray_ui.color_mode {
            ColorMode::Wheel => {
                let mut c = Color32::from_rgb(color[0], color[1], color[2]);
                ui.spacing_mut().slider_width = (ui.available_width() - 20.0).clamp(80.0, 180.0);
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut c,
                    egui::color_picker::Alpha::Opaque,
                ) {
                    *color = [c.r(), c.g(), c.b()];
                }
            }
            ColorMode::Rgb => {
                egui::Grid::new("mt_ui_material_rgb")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.spacing_mut().slider_width = slider_w;
                        for (k, label) in ["R", "G", "B"].iter().enumerate() {
                            ui.label(*label);
                            ui.add(egui::Slider::new(&mut color[k], 0..=255));
                            ui.end_row();
                        }
                    });
            }
            ColorMode::Hsl => {
                let mut hsl = match self.tray_ui.material_hsl {
                    Some((c, hsl)) if c == *color => hsl,
                    _ => rgb_to_hsl(*color),
                };
                // Whole degrees and percent; only a slider the user moved changes the
                // color (rounding alone must not).
                let mut changed = false;
                egui::Grid::new("mt_ui_material_hsl")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.spacing_mut().slider_width = slider_w;
                        for (k, (label, max, suffix)) in
                            [("H", 360, "°"), ("S", 100, "%"), ("L", 100, "%")]
                                .into_iter()
                                .enumerate()
                        {
                            ui.label(label);
                            let mut v = hsl[k].round() as i32;
                            let r = ui.add(egui::Slider::new(&mut v, 0..=max).suffix(suffix));
                            if r.changed() && v != hsl[k].round() as i32 {
                                hsl[k] = f64::from(v);
                                changed = true;
                            }
                            ui.end_row();
                        }
                    });
                if changed {
                    *color = hsl_to_rgb(hsl);
                }
                self.tray_ui.material_hsl = Some((*color, hsl));
            }
            ColorMode::Hex => {
                let id = ui.id().with("mt_ui_material_hex");
                if !ui.memory(|m| m.has_focus(id)) {
                    self.tray_ui.material_hex = hex_color(*color);
                }
                ui.horizontal(|ui| {
                    ui.label("HEX");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.tray_ui.material_hex)
                            .id(id)
                            .desired_width(80.0),
                    );
                    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        Color32::from_rgb(color[0], color[1], color[2]),
                    );
                    if r.lost_focus()
                        && !ui.input(|i| i.key_pressed(egui::Key::Escape))
                        && let Some(c) = parse_hex_color(&self.tray_ui.material_hex)
                    {
                        *color = c;
                    }
                });
            }
        }
    }

    /// Texture controls of the Materials Edit tab: "Use texture image", the file with
    /// Browse and external editing, size with aspect lock, Colorize, and Position
    /// Texture for the selected face. Unverified: SketchUp's layout and that the size
    /// fields' lock is a chain icon (a check box here).
    fn texture_editor(&mut self, ui: &mut egui::Ui, i: usize, data: &TrayData) {
        let m = &data.materials[i];
        let mut use_texture = m.texture.is_some();
        if ui.checkbox(&mut use_texture, "Use texture image").changed() {
            self.pending.push(Event::MaterialTexture {
                index: i,
                action: if use_texture {
                    TextureAction::Load
                } else {
                    TextureAction::Remove
                },
            });
        }
        let Some(tex) = &m.texture else {
            return;
        };
        ui.horizontal(|ui| {
            let file = std::path::Path::new(&tex.file)
                .file_name()
                .map_or_else(|| tex.file.clone(), |n| n.to_string_lossy().into_owned());
            ui.add(egui::Label::new(file).truncate())
                .on_hover_text(&tex.file);
        });
        ui.horizontal(|ui| {
            if ui
                .button("Browse...")
                .on_hover_text("Load a different image")
                .clicked()
            {
                self.pending.push(Event::MaterialTexture {
                    index: i,
                    action: TextureAction::Load,
                });
            }
            if ui
                .button("Edit Externally")
                .on_hover_text("Edit texture image in external editor")
                .clicked()
            {
                self.pending.push(Event::MaterialTexture {
                    index: i,
                    action: TextureAction::EditExternally,
                });
            }
        });
        let w_id = ui.id().with("mt_ui_texture_w");
        let h_id = ui.id().with("mt_ui_texture_h");
        let focused = ui.memory(|mem| mem.has_focus(w_id) || mem.has_focus(h_id));
        if !focused || self.tray_ui.texture_of != Some(i) {
            self.tray_ui.texture_of = Some(i);
            self.tray_ui.texture_w.clone_from(&tex.width);
            self.tray_ui.texture_h.clone_from(&tex.height);
        }
        let lock = !self.tray_ui.texture_unlocked;
        let mut size = None;
        egui::Grid::new("mt_ui_texture_size")
            .num_columns(2)
            .show(ui, |ui| {
                for (label, id, is_w) in [("Width", w_id, true), ("Height", h_id, false)] {
                    ui.label(label);
                    let buf = if is_w {
                        &mut self.tray_ui.texture_w
                    } else {
                        &mut self.tray_ui.texture_h
                    };
                    let r = ui.add(egui::TextEdit::singleline(buf).id(id).desired_width(90.0));
                    let old = if is_w { &tex.width } else { &tex.height };
                    if r.lost_focus()
                        && !ui.input(|inp| inp.key_pressed(egui::Key::Escape))
                        && buf.trim() != old.trim()
                    {
                        let text = Some(buf.trim().to_owned());
                        size = Some(if is_w { (text, None) } else { (None, text) });
                    }
                    ui.end_row();
                }
            });
        if let Some((width, height)) = size {
            self.pending.push(Event::MaterialTexture {
                index: i,
                action: TextureAction::Size {
                    width,
                    height,
                    lock,
                },
            });
        }
        let mut locked = lock;
        if ui.checkbox(&mut locked, "Lock aspect ratio").changed() {
            self.tray_ui.texture_unlocked = !locked;
        }
        let mut colorize = tex.colorize;
        if ui.checkbox(&mut colorize, "Colorize").changed() {
            self.pending.push(Event::MaterialTexture {
                index: i,
                action: TextureAction::Colorize(colorize),
            });
        }
        if ui
            .add_enabled(
                data.can_position_texture,
                egui::Button::new("Position Texture"),
            )
            .on_hover_text("Position the texture on the selected face")
            .on_disabled_hover_text("Select one face painted with a texture")
            .clicked()
        {
            self.pending
                .push(Event::Command("tool.texture_position".to_owned()));
        }
    }

    /// Components panel: header with an icon, the name and the description of the
    /// current definition; Select tab with the "In Model" list (click = place an
    /// instance) and a Details menu (Purge Unused); Edit tab with the name and
    /// description fields.
    ///
    /// Unverified against SketchUp 2026: SketchUp edits name and description in the
    /// header fields and its Edit tab holds the component behaviors (Glue to, Cut
    /// opening, Always face camera, Shadows face sun, axes), which are left out; the
    /// list shows rendered thumbnails (a drawn icon here) in an icon grid by default
    /// (the Details menu's List view is used); the instance count is shown here on
    /// each row and in the tooltip, SketchUp shows it in the Statistics tab (left
    /// out); the 3D Warehouse search field and collection navigation are left out.
    fn components_panel(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let t = self.theme.clone();
        let current = data
            .current_component
            .filter(|&i| i < data.components.len());

        // Header: icon, name, description.
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            ui.painter().rect(
                rect,
                0.0,
                t.field,
                Stroke::new(1.0, t.border),
                StrokeKind::Inside,
            );
            let ink = if current.is_some() {
                t.text
            } else {
                t.text_disabled
            };
            paint_cube(ui.painter(), rect.shrink(8.0), ink);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                match current {
                    Some(i) => {
                        let c = &data.components[i];
                        ui.label(RichText::new(&c.name).strong());
                        if !c.description.is_empty() {
                            ui.label(RichText::new(&c.description).color(t.text_disabled));
                        }
                    }
                    None => {
                        ui.label(RichText::new("No component").color(t.text_disabled));
                    }
                }
            });
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (tab, label) in [
                (ComponentTab::Select, "Select"),
                (ComponentTab::Edit, "Edit"),
            ] {
                if tab_button(ui, &t, label, self.tray_ui.component_tab == tab).clicked() {
                    self.tray_ui.component_tab = tab;
                }
            }
        });

        match self.tray_ui.component_tab {
            ComponentTab::Select => self.components_select(ui, data, current, &t),
            ComponentTab::Edit => self.components_edit(ui, data, current, &t),
        }
    }

    fn components_select(
        &mut self,
        ui: &mut egui::Ui,
        data: &mut TrayData,
        current: Option<usize>,
        t: &Theme32,
    ) {
        ui.horizontal(|ui| {
            ui.label("In Model");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = icon_button(ui, t, "Details", true, paint_details);
                let any_unused = data.components.iter().any(|c| c.instances == 0);
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(140.0);
                    if ui
                        .add_enabled(any_unused, egui::Button::new("Purge Unused"))
                        .clicked()
                    {
                        self.pending.push(Event::ComponentPurge);
                        ui.close();
                    }
                });
            });
        });
        if data.components.is_empty() {
            ui.label(RichText::new("No components in the model.").color(t.text_disabled));
            return;
        }
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        let font = FontId::proportional(t.sizes.font);
        for (i, c) in data.components.iter().enumerate() {
            let (row, r) = ui.allocate_exact_size(vec2(w, ROW + 4.0), Sense::click());
            let (fill, ink, sub) = if current == Some(i) {
                (t.selection, Color32::WHITE, Color32::WHITE)
            } else if r.hovered() {
                (t.hover, t.text, t.text_disabled)
            } else {
                (t.field, t.text, t.text_disabled)
            };
            ui.painter().rect_filled(row, 0.0, fill);
            let icon =
                Rect::from_center_size(pos2(row.left() + 13.0, row.center().y), vec2(16.0, 16.0));
            paint_cube(ui.painter(), icon, ink);
            let count = c.instances.to_string();
            let count_w = ui
                .painter()
                .layout_no_wrap(count.clone(), font.clone(), sub)
                .size()
                .x;
            let name_clip = Rect::from_min_max(
                pos2(icon.right() + 5.0, row.top()),
                pos2(row.right() - count_w - 12.0, row.bottom()),
            );
            ui.painter().with_clip_rect(name_clip).text(
                pos2(name_clip.left(), row.center().y),
                Align2::LEFT_CENTER,
                &c.name,
                font.clone(),
                ink,
            );
            ui.painter().text(
                pos2(row.right() - 6.0, row.center().y),
                Align2::RIGHT_CENTER,
                count,
                font.clone(),
                sub,
            );
            let mut tip = format!("{}\n{}", c.name, instances_label(c.instances));
            if !c.description.is_empty() {
                tip = format!("{tip}\n{}", c.description);
            }
            let r = r.on_hover_text(tip);
            r.context_menu(|ui| {
                if ui.button("Select Instances").clicked() {
                    self.pending.push(Event::ComponentSelectInstances(i));
                    ui.close();
                }
                if ui
                    .add_enabled(
                        data.can_replace_component,
                        egui::Button::new("Replace Selected"),
                    )
                    .clicked()
                {
                    self.pending.push(Event::ComponentReplaceSelected(i));
                    ui.close();
                }
            });
            if r.clicked() {
                data.current_component = Some(i);
                self.pending.push(Event::ComponentPick(i));
            }
        }
        let area = Rect::from_x_y_ranges(
            ui.min_rect().left()..=ui.min_rect().left() + w,
            top..=ui.cursor().top(),
        );
        ui.painter()
            .rect_stroke(area, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }

    fn components_edit(
        &mut self,
        ui: &mut egui::Ui,
        data: &mut TrayData,
        current: Option<usize>,
        t: &Theme32,
    ) {
        let Some(i) = current else {
            ui.label(
                RichText::new("Pick a component in the Select tab to edit it.")
                    .color(t.text_disabled),
            );
            return;
        };
        let name_id = ui.id().with("mt_ui_component_name");
        let desc_id = ui.id().with("mt_ui_component_description");
        let focused = ui.memory(|m| m.has_focus(name_id) || m.has_focus(desc_id));
        if !focused || self.tray_ui.component_fields_of != current {
            self.tray_ui.component_fields_of = current;
            self.tray_ui.component_name = data.components[i].name.clone();
            self.tray_ui.component_description = data.components[i].description.clone();
        }
        let width = ui.available_width();
        ui.label("Name");
        let name = ui.add(
            egui::TextEdit::singleline(&mut self.tray_ui.component_name)
                .id(name_id)
                .desired_width(width),
        );
        ui.label("Description");
        let desc = ui.add(
            egui::TextEdit::multiline(&mut self.tray_ui.component_description)
                .id(desc_id)
                .desired_rows(3)
                .desired_width(width),
        );
        ui.label(
            RichText::new(instances_label(data.components[i].instances)).color(t.text_disabled),
        );
        // Component behaviors (Glue to, Cut opening, Always face camera, Shadows face
        // sun); SketchUp's layout of this group is unverified.
        ui.add_space(4.0);
        ui.label(RichText::new("Component Axes and Behaviors").strong());
        let before = data.components[i].behavior;
        let b = &mut data.components[i].behavior;
        ui.horizontal(|ui| {
            ui.label("Glue to");
            egui::ComboBox::from_id_salt("mt_ui_component_glue")
                .selected_text(crate::tray_data::GLUE_NAMES[b.glue.min(4)])
                .show_ui(ui, |ui| {
                    for (k, name) in crate::tray_data::GLUE_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut b.glue, k, *name);
                    }
                });
        });
        ui.add_enabled(
            b.glue != 0,
            egui::Checkbox::new(&mut b.cut_opening, "Cut opening"),
        );
        ui.checkbox(&mut b.always_face_camera, "Always face camera");
        ui.add_enabled(
            b.always_face_camera,
            egui::Checkbox::new(&mut b.shadows_face_sun, "Shadows face sun"),
        );
        if *b != before {
            self.pending.push(Event::ComponentBehavior {
                index: i,
                behavior: *b,
            });
        }
        if name.lost_focus() || desc.lost_focus() {
            if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                // Cancel: reload from the model next frame.
                self.tray_ui.component_fields_of = None;
                return;
            }
            let row = &mut data.components[i];
            if let Some((name, description)) = component_edit(
                row,
                &self.tray_ui.component_name,
                &self.tray_ui.component_description,
            ) {
                row.name.clone_from(&name);
                row.description.clone_from(&description);
                self.pending.push(Event::ComponentEdit {
                    index: i,
                    name,
                    description,
                });
            }
        }
    }

    /// Styles panel, Edit tab: Edge Settings (Edges, Profiles + width), Face
    /// Settings (face style, X-ray) and Modeling Settings (Section Cut Width), plus
    /// the rows of `style_panel` (and its Background and Watermark pages).
    /// Unverified: SketchUp shows these pages behind icon buttons and the face
    /// styles as icon buttons; text is used here.
    pub(crate) fn styles_edit(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let t = self.theme.clone();
        let before = data.style;
        let s = &mut data.style;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (page, label) in [
                (StylesPage::Edge, "Edge"),
                (StylesPage::Face, "Face"),
                (StylesPage::Background, "Background"),
                (StylesPage::Watermark, "Watermark"),
                (StylesPage::Modeling, "Modeling"),
            ] {
                if tab_button(ui, &t, label, self.tray_ui.styles_page == page).clicked() {
                    self.tray_ui.styles_page = page;
                }
            }
        });
        match self.tray_ui.styles_page {
            StylesPage::Edge => {
                ui.label(RichText::new("Edge Settings").strong());
                ui.checkbox(&mut s.edges, "Edges");
                ui.checkbox(&mut s.back_edges, "Back Edges");
                for (on, value, label) in [
                    (&mut s.profiles, &mut s.profile_width, "Profiles"),
                    (&mut s.depth_cue, &mut s.depth_cue_width, "Depth Cue"),
                    (&mut s.extension, &mut s.extension_length, "Extension"),
                ] {
                    ui.horizontal(|ui| {
                        ui.checkbox(on, label);
                        ui.add_enabled(*on, egui::DragValue::new(value).range(1..=99));
                    });
                }
            }
            StylesPage::Background | StylesPage::Watermark => {}
            StylesPage::Face => {
                ui.label(RichText::new("Face Settings").strong());
                ui.label("Style");
                for style in FaceStyle::ALL {
                    ui.radio_value(&mut s.face_style, style, style.label());
                }
                ui.checkbox(&mut s.x_ray, "X-ray");
            }
            StylesPage::Modeling => {
                ui.label(RichText::new("Modeling Settings").strong());
                ui.horizontal(|ui| {
                    ui.label("Section Cut Width");
                    ui.add(egui::DragValue::new(&mut s.section_cut_width).range(1..=99));
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.section_fill, "Section Fill");
                    ui.color_edit_button_srgb(&mut s.section_fill_color);
                });
            }
        }
        if data.style != before {
            self.pending.push(Event::StyleChanged(data.style));
        }
        use crate::style_panel::ExtrasPage;
        let page = match self.tray_ui.styles_page {
            StylesPage::Edge => ExtrasPage::Edge,
            StylesPage::Face => ExtrasPage::Face,
            StylesPage::Background => ExtrasPage::Background,
            StylesPage::Watermark => ExtrasPage::Watermark,
            StylesPage::Modeling => ExtrasPage::Modeling,
        };
        self.style_extras_ui(ui, data, page);
    }

    /// Shadows panel: show/hide, Time and Date sliders, Light and Dark sliders and
    /// the Display checkboxes. Unverified: UTC offset, "Use sun for shading" and
    /// "From edges" are left out; SketchUp's time slider spans sunrise to sunset
    /// rather than the whole day.
    fn shadows_panel(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let before = data.shadows;
        let s = &mut data.shadows;
        ui.checkbox(&mut s.enabled, "Show Shadows");
        let slider_w = (ui.available_width() - 110.0).max(60.0);
        egui::Grid::new("mt_ui_shadows")
            .num_columns(3)
            .spacing(vec2(6.0, 4.0))
            .show(ui, |ui| {
                ui.spacing_mut().slider_width = slider_w;
                ui.label("UTC");
                egui::ComboBox::from_id_salt("mt_ui_shadows_utc")
                    .selected_text(crate::tray_data::utc_label(s.utc_offset_minutes))
                    .show_ui(ui, |ui| {
                        // Whole and half hours from UTC-12:00 to UTC+14:00.
                        for k in -24..=28 {
                            let m = k * 30;
                            ui.selectable_value(
                                &mut s.utc_offset_minutes,
                                m,
                                crate::tray_data::utc_label(m),
                            );
                        }
                    });
                ui.label("");
                ui.end_row();
                ui.label("Time");
                ui.add(egui::Slider::new(&mut s.minute_of_day, 0..=24 * 60 - 1).show_value(false));
                ui.label(time_label(s.minute_of_day));
                ui.end_row();
                ui.label("Date");
                ui.add(egui::Slider::new(&mut s.day_of_year, 1..=365).show_value(false));
                ui.label(date_label(s.day_of_year));
                ui.end_row();
                ui.label("Light");
                ui.add(egui::Slider::new(&mut s.light, 0..=100).show_value(false));
                ui.label(s.light.to_string());
                ui.end_row();
                ui.label("Dark");
                ui.add(egui::Slider::new(&mut s.dark, 0..=100).show_value(false));
                ui.label(s.dark.to_string());
                ui.end_row();
            });
        ui.checkbox(&mut s.use_sun_for_shading, "Use sun for shading");
        ui.horizontal_wrapped(|ui| {
            ui.label("Display:");
            ui.checkbox(&mut s.on_faces, "On faces");
            ui.checkbox(&mut s.on_ground, "On ground");
            ui.checkbox(&mut s.from_edges, "From edges");
        });
        if data.shadows != before {
            self.pending.push(Event::ShadowsChanged(data.shadows));
        }
    }

    /// Fog panel: Display Fog, the Distance sliders (start and end, in percent of the
    /// model's depth) and the color (background or own). Unverified: SketchUp draws
    /// one two-handled Distance slider labeled 0 and infinity.
    fn fog_panel(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let before = data.fog;
        let f = &mut data.fog;
        ui.checkbox(&mut f.enabled, "Display Fog");
        let slider_w = (ui.available_width() - 90.0).max(60.0);
        egui::Grid::new("mt_ui_fog")
            .num_columns(2)
            .spacing(vec2(6.0, 4.0))
            .show(ui, |ui| {
                ui.spacing_mut().slider_width = slider_w;
                ui.label("Start");
                ui.add(egui::Slider::new(&mut f.start, 0..=100).suffix("%"));
                ui.end_row();
                ui.label("End");
                ui.add(egui::Slider::new(&mut f.end, 0..=100).suffix("%"));
                ui.end_row();
            });
        // The end never comes before the start: the slider being dragged wins.
        if f.start != before.start {
            f.end = f.end.max(f.start);
        } else {
            f.start = f.start.min(f.end);
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut f.use_background, "Use Background Color");
            ui.add_enabled_ui(!f.use_background, |ui| {
                ui.color_edit_button_srgb(&mut f.color);
            });
        });
        if data.fog != before {
            self.pending.push(Event::FogChanged(data.fog));
        }
    }

    /// Soften Edges panel: the "Angle between normals" slider (degrees) and the Smooth
    /// normals and Soften coplanar boxes. Unverified: SketchUp's slider shows no
    /// number field and is labeled "Angle between normals" above it.
    fn soften_panel(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let before = data.soften;
        let s = &mut data.soften;
        ui.label("Angle between normals");
        ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(60.0);
        ui.add(egui::Slider::new(&mut s.angle, 0..=180).suffix("°"));
        ui.checkbox(&mut s.smooth, "Smooth normals");
        ui.checkbox(&mut s.coplanar, "Soften coplanar");
        if data.soften != before {
            self.pending.push(Event::SoftenChanged(data.soften));
        }
    }

    /// Scene tabs above the viewport (View > Scene Tabs). Click activates a scene;
    /// right-click offers Update, Add and Delete. Unverified: tab shape and colors,
    /// and the full context menu (SketchUp also has Move Left/Right, Scene Manager).
    pub(crate) fn scene_tabs(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        let data = std::mem::take(&mut self.data);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(4.0);
            for (i, name) in data.scenes.iter().enumerate() {
                let active = data.active_scene == Some(i);
                // Scenes left out of the animation in parentheses (Scenes panel).
                let name = &if data
                    .scene_details
                    .get(i)
                    .is_none_or(|d| d.include_in_animation)
                {
                    name.clone()
                } else {
                    format!("({name})")
                };
                let font = FontId::proportional(t.sizes.font);
                let text_w = ui
                    .painter()
                    .layout_no_wrap(name.clone(), font.clone(), t.text)
                    .size()
                    .x;
                let (rect, r) = ui.allocate_exact_size(
                    vec2(text_w + 20.0, ui.available_height() - 3.0),
                    Sense::click(),
                );
                let fill = if active {
                    t.field
                } else if r.hovered() {
                    t.hover
                } else {
                    t.panel_header
                };
                let rect = rect.translate(vec2(0.0, 2.0));
                ui.painter().rect(
                    rect,
                    egui::CornerRadius {
                        nw: 3,
                        ne: 3,
                        sw: 0,
                        se: 0,
                    },
                    fill,
                    Stroke::new(1.0, t.border),
                    StrokeKind::Inside,
                );
                ui.painter()
                    .text(rect.center(), Align2::CENTER_CENTER, name, font, t.text);
                if r.clicked() {
                    self.pending.push(Event::SceneActivate(i));
                }
                r.context_menu(|ui| {
                    if ui.button("Update").clicked() {
                        self.pending.push(Event::SceneUpdate(i));
                        ui.close();
                    }
                    if ui.button("Add...").clicked() {
                        self.pending.push(Event::SceneAdd);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        self.pending.push(Event::SceneDelete(i));
                        ui.close();
                    }
                });
            }
        });
        self.data = data;
    }
}

/// Flat icon button (hover/pressed look of the toolbars) with a painted icon.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    t: &Theme32,
    tooltip: &str,
    enabled: bool,
    paint: fn(&Painter, Rect, Color32),
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(vec2(ICON_BUTTON, ICON_BUTTON), sense);
    if enabled && r.hovered() {
        let (fill, stroke) = if r.is_pointer_button_down_on() {
            (t.pressed, t.pressed_border)
        } else {
            (t.hover, t.hover_border)
        };
        ui.painter().rect(
            rect,
            0.0,
            fill,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        );
    }
    let ink = if enabled { t.text } else { t.text_disabled };
    paint(ui.painter(), rect.shrink(4.0), ink);
    r.on_hover_text(tooltip)
}

/// Tab header button; the selected tab is drawn like the panel body.
pub(crate) fn tab_button(ui: &mut egui::Ui, t: &Theme32, label: &str, selected: bool) -> Response {
    let font = FontId::proportional(t.sizes.font);
    let w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), t.text)
        .size()
        .x;
    let (rect, r) = ui.allocate_exact_size(vec2(w + 18.0, ROW), Sense::click());
    let fill = if selected {
        t.field
    } else if r.hovered() {
        t.hover
    } else {
        t.panel_header
    };
    ui.painter().rect(
        rect,
        0.0,
        fill,
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, label, font, t.text);
    r
}

/// Material swatch; translucent colors are drawn over a checkerboard.
fn paint_swatch(p: &Painter, rect: Rect, color: [u8; 3], opacity: u8, t: &Theme32) {
    if opacity < 100 {
        let n = 4;
        let cell = rect.size() / n as f32;
        for y in 0..n {
            for x in 0..n {
                let c = if (x + y) % 2 == 0 {
                    Color32::WHITE
                } else {
                    Color32::from_gray(200)
                };
                let min = rect.min + vec2(x as f32 * cell.x, y as f32 * cell.y);
                p.rect_filled(Rect::from_min_size(min, cell), 0.0, c);
            }
        }
    }
    let a = (f32::from(opacity.min(100)) / 100.0 * 255.0).round() as u8;
    p.rect_filled(
        rect,
        0.0,
        Color32::from_rgba_unmultiplied(color[0], color[1], color[2], a),
    );
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
}

/// SketchUp's default material swatch: front (white) and back (blue-grey) halves.
fn paint_default_swatch(p: &Painter, rect: Rect, t: &Theme32) {
    p.rect_filled(rect, 0.0, Color32::WHITE);
    p.add(egui::Shape::convex_polygon(
        vec![rect.right_top(), rect.right_bottom(), rect.left_bottom()],
        Color32::from_rgb(164, 178, 187),
        Stroke::NONE,
    ));
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
}

pub(crate) fn paint_plus(p: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    let s = Stroke::new(1.6, ink);
    p.circle_stroke(c, r.width() / 2.0, Stroke::new(1.0, ink));
    p.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], s);
    p.line_segment([c - vec2(0.0, 4.0), c + vec2(0.0, 4.0)], s);
}

pub(crate) fn paint_minus(p: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    p.circle_stroke(c, r.width() / 2.0, Stroke::new(1.0, ink));
    p.line_segment(
        [c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)],
        Stroke::new(1.6, ink),
    );
}

pub(crate) fn paint_refresh(p: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    let rad = r.width() / 2.0 - 1.0;
    let pts: Vec<_> = (0..=20)
        .map(|i| {
            let a = 0.6 + i as f32 / 20.0 * 5.0;
            c + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    let end = *pts.last().expect("points");
    p.add(egui::Shape::line(pts, Stroke::new(1.4, ink)));
    p.add(egui::Shape::convex_polygon(
        vec![
            end + vec2(-3.5, -1.0),
            end + vec2(3.0, -2.5),
            end + vec2(0.5, 3.5),
        ],
        ink,
        Stroke::NONE,
    ));
}

/// Eye icon; a closed eye (line with lashes) when not visible.
pub(crate) fn paint_eye(p: &Painter, r: Rect, ink: Color32, open: bool) {
    let c = r.center();
    let s = Stroke::new(1.1, ink);
    let curve = |sign: f32| -> Vec<egui::Pos2> {
        (0..=12)
            .map(|i| {
                let x = -6.5 + i as f32 * 13.0 / 12.0;
                let y = sign * 4.0 * (1.0 - (x / 6.5).powi(2));
                c + vec2(x, y)
            })
            .collect()
    };
    if open {
        p.add(egui::Shape::line(curve(-1.0), s));
        p.add(egui::Shape::line(curve(1.0), s));
        p.circle_filled(c, 2.0, ink);
    } else {
        p.add(egui::Shape::line(curve(1.0), s));
        for x in [-4.0, 0.0, 4.0] {
            let base = c + vec2(x, 4.0 * (1.0 - (x / 6.5f32).powi(2)));
            p.line_segment([base, base + vec2(x * 0.2, 2.5)], s);
        }
    }
}

pub(crate) fn paint_pencil(p: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    let a = c + vec2(4.5, -4.5);
    let b = c + vec2(-3.0, 3.0);
    p.line_segment([a, b], Stroke::new(3.0, ink));
    p.add(egui::Shape::convex_polygon(
        vec![
            b + vec2(-1.5, -1.5),
            b + vec2(1.5, 1.5),
            c + vec2(-5.0, 5.0),
        ],
        ink,
        Stroke::NONE,
    ));
}

/// Expand arrow: right-pointing when closed, down-pointing when open.
pub(crate) fn paint_arrow(p: &Painter, c: egui::Pos2, open: bool, ink: Color32) {
    let tri = if open {
        vec![
            pos2(c.x - 3.5, c.y - 2.0),
            pos2(c.x + 3.5, c.y - 2.0),
            pos2(c.x, c.y + 2.5),
        ]
    } else {
        vec![
            pos2(c.x - 2.0, c.y - 3.5),
            pos2(c.x + 2.5, c.y),
            pos2(c.x - 2.0, c.y + 3.5),
        ]
    };
    p.add(egui::Shape::convex_polygon(tri, ink, Stroke::NONE));
}

/// Model root icon (a small house); SketchUp's actual icon is unverified.
pub(crate) fn paint_model_icon(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.1, ink);
    let roof = r.top() + r.height() * 0.45;
    p.add(egui::Shape::closed_line(
        vec![
            pos2(r.left() + 2.0, roof),
            pos2(r.center().x, r.top() + 1.0),
            pos2(r.right() - 2.0, roof),
            pos2(r.right() - 2.0, r.bottom() - 1.0),
            pos2(r.left() + 2.0, r.bottom() - 1.0),
        ],
        s,
    ));
}

/// Group icon: a box outline (unverified).
pub(crate) fn paint_group_icon(p: &Painter, r: Rect, ink: Color32) {
    let r = r.shrink(1.5);
    let s = Stroke::new(1.1, ink);
    let d = 3.0;
    let front = Rect::from_min_max(pos2(r.left(), r.top() + d), pos2(r.right() - d, r.bottom()));
    p.rect_stroke(front, 0.0, s, StrokeKind::Middle);
    p.line_segment([front.left_top(), front.left_top() + vec2(d, -d)], s);
    p.line_segment([front.right_top(), front.right_top() + vec2(d, -d)], s);
    p.line_segment(
        [front.right_bottom(), front.right_bottom() + vec2(d, -d)],
        s,
    );
    p.line_segment(
        [
            front.left_top() + vec2(d, -d),
            front.right_top() + vec2(d, -d),
        ],
        s,
    );
    p.line_segment(
        [
            front.right_top() + vec2(d, -d),
            front.right_bottom() + vec2(d, -d),
        ],
        s,
    );
}

/// Component icon: four small boxes (unverified).
pub(crate) fn paint_component_icon(p: &Painter, r: Rect, ink: Color32) {
    let r = r.shrink(1.0);
    let h = r.width() / 2.0;
    for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let cell = Rect::from_min_size(r.min + vec2(x * h, y * h), vec2(h, h)).shrink(1.0);
        p.rect_filled(cell, 0.0, ink);
    }
}

/// Isometric cube outline: icon of a component definition (SketchUp shows a
/// rendered thumbnail instead).
fn paint_cube(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.2, ink);
    let c = r.center();
    let a = r.width().min(r.height()) / 2.0;
    let dx = a * 0.866;
    let top = pos2(c.x, c.y - a);
    let ul = pos2(c.x - dx, c.y - a / 2.0);
    let ur = pos2(c.x + dx, c.y - a / 2.0);
    let ll = pos2(c.x - dx, c.y + a / 2.0);
    let lr = pos2(c.x + dx, c.y + a / 2.0);
    let bottom = pos2(c.x, c.y + a);
    p.add(egui::Shape::closed_line(
        vec![top, ur, lr, bottom, ll, ul],
        s,
    ));
    p.line_segment([ul, c], s);
    p.line_segment([ur, c], s);
    p.line_segment([c, bottom], s);
}

/// Details menu button: a right-pointing arrow head over three lines (unverified;
/// SketchUp uses a small arrow icon).
pub(crate) fn paint_details(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.2, ink);
    for k in 0..3 {
        let y = r.top() + 3.0 + k as f32 * 4.0;
        p.hline(r.left()..=r.right() - 6.0, y, s);
    }
    let x = r.right() - 3.0;
    let c = r.center().y;
    p.add(egui::Shape::convex_polygon(
        vec![
            pos2(x - 3.0, c - 4.0),
            pos2(x + 1.0, c),
            pos2(x - 3.0, c + 4.0),
        ],
        ink,
        Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentRow, EntityInfo, Frame};
    use mt_spec::Spec;

    /// Runs one 1600x900 frame of the whole shell with `events`.
    fn run(
        ctx: &egui::Context,
        s: &mut Shell,
        spec: &Spec,
        events: Vec<egui::Event>,
    ) -> Vec<Event> {
        let info = EntityInfo::no_selection();
        let frame = Frame {
            spec,
            active_tool: "tool.select",
            status: "",
            measurements_label: "Measurements",
            measurements_value: "",
            is_enabled: &|_| true,
            is_checked: &|_| false,
            entity_info: &info,
        };
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 900.0))),
            events,
            ..Default::default()
        };
        let mut out_events = Vec::new();
        let mut out = ctx.run_ui(input, |ui| out_events = s.show(ui, &frame, |_| {}));
        out.textures_delta.clear();
        out_events
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

    /// The Fog panel is not in the tray until Window > Fog; then Display Fog turns
    /// fog on.
    #[test]
    fn window_fog_opens_panel_and_turns_fog_on() {
        let spec = Spec::builtin().unwrap();
        let ctx = egui::Context::default();
        let mut s = Shell::new(&spec);
        assert!(!s.panel_expanded("window.fog"));
        assert!(s.panels.iter().any(|p| p.id == "window.fog" && !p.shown));
        s.command("window.fog");
        assert!(s.panel_expanded("window.fog"));
        let mut changed = None;
        for y in (40..600).step_by(4) {
            // Only the Fog panel, expanded (a click may hit its header).
            for p in &mut s.panels {
                p.shown = p.id == "window.fog";
                p.expanded = p.shown;
            }
            let ev = run(&ctx, &mut s, &spec, click(pos2(1360.0, y as f32)));
            if let Some(Event::FogChanged(f)) =
                ev.into_iter().find(|e| matches!(e, Event::FogChanged(_)))
            {
                changed = Some(f);
                break;
            }
        }
        let f = changed.expect("a click in the panel changes the fog");
        assert!(f.enabled, "Display Fog is the first control: {f:?}");
        assert_eq!((f.start, f.end), (0, 100));
    }

    /// Window > Soften Edges shows the panel; a click on it changes the settings.
    #[test]
    fn window_soften_edges_opens_panel() {
        let spec = Spec::builtin().unwrap();
        let ctx = egui::Context::default();
        let mut s = Shell::new(&spec);
        assert!(
            s.panels
                .iter()
                .any(|p| p.id == "window.soften_edges" && !p.shown)
        );
        s.command("window.soften_edges");
        assert!(s.panel_expanded("window.soften_edges"));
        let mut changed = None;
        for y in (40..600).step_by(4) {
            for p in &mut s.panels {
                p.shown = p.id == "window.soften_edges";
                p.expanded = p.shown;
            }
            let ev = run(&ctx, &mut s, &spec, click(pos2(1360.0, y as f32)));
            if let Some(Event::SoftenChanged(f)) = ev
                .into_iter()
                .find(|e| matches!(e, Event::SoftenChanged(_)))
            {
                changed = Some(f);
                break;
            }
        }
        let f = changed.expect("a click in the panel changes the settings");
        assert_ne!(f, crate::SoftenSettings::default());
    }

    /// The Materials Edit tab sends nothing by itself in any color mode (HSL rounding
    /// must not recolor the material), and clicking down it finds the Colorize box and
    /// the Position Texture button.
    #[test]
    fn material_edit_tab_is_quiet_until_used() {
        let spec = Spec::builtin().unwrap();
        let ctx = egui::Context::default();
        let mut s = Shell::new(&spec);
        let only_materials = |s: &mut Shell| {
            for p in &mut s.panels {
                p.shown = true;
                p.expanded = p.id == "window.materials";
            }
        };
        s.set_tray_data(TrayData {
            materials: vec![crate::MaterialRow {
                name: "Bricks".into(),
                color: [191, 127, 63],
                opacity: 100,
                texture: Some(crate::TextureRow {
                    file: "bricks.png".into(),
                    width: "1m".into(),
                    height: "0.5m".into(),
                    colorize: false,
                }),
            }],
            current_material: Some(0),
            can_position_texture: true,
            ..TrayData::default()
        });
        s.tray_ui.material_tab = MaterialTab::Edit;
        for mode in [
            ColorMode::Wheel,
            ColorMode::Rgb,
            ColorMode::Hsl,
            ColorMode::Hex,
        ] {
            s.tray_ui.color_mode = mode;
            only_materials(&mut s);
            for _ in 0..3 {
                assert_eq!(run(&ctx, &mut s, &spec, vec![]), vec![], "{mode:?}");
            }
        }
        let mut seen = Vec::new();
        for x in [1320.0, 1380.0] {
            for y in (300..880).step_by(4) {
                only_materials(&mut s);
                s.tray_ui.material_tab = MaterialTab::Edit;
                s.tray_ui.color_mode = ColorMode::Hex;
                seen.extend(run(&ctx, &mut s, &spec, click(pos2(x, y as f32))));
            }
        }
        assert!(
            seen.contains(&Event::MaterialTexture {
                index: 0,
                action: TextureAction::Colorize(true)
            }),
            "{seen:?}"
        );
        assert!(
            seen.contains(&Event::Command("tool.texture_position".into())),
            "{seen:?}"
        );
    }

    /// Expands only the Components panel (clicks while scanning may toggle headers).
    fn only_components(s: &mut Shell) {
        for p in &mut s.panels {
            p.shown = true;
            p.expanded = p.id == "window.components";
        }
    }

    /// Clicks down the tray (so the test does not depend on exact pixel layout) to
    /// pick a definition, then to focus the Edit tab's name field and rename it.
    #[test]
    fn pick_and_edit_component() {
        let spec = Spec::builtin().unwrap();
        let ctx = egui::Context::default();
        let mut s = Shell::new(&spec);
        only_components(&mut s);
        let row = |name: &str, instances| ComponentRow {
            name: name.into(),
            description: String::new(),
            instances,
            ..ComponentRow::default()
        };
        s.set_tray_data(TrayData {
            components: vec![row("Door", 1), row("Window", 0)],
            ..TrayData::default()
        });
        assert!(run(&ctx, &mut s, &spec, vec![]).is_empty());

        // Click down the tray until a list row is hit: picks a definition.
        let x = 1450.0;
        let mut picked = None;
        for y in (150..600).step_by(6) {
            only_components(&mut s);
            let ev = run(&ctx, &mut s, &spec, click(pos2(x, y as f32)));
            if let Some(Event::ComponentPick(i)) = ev.first() {
                picked = Some(*i);
                break;
            }
            // Leave the Edit tab if a click switched to it.
            s.tray_ui.component_tab = ComponentTab::Select;
        }
        let i = picked.expect("a click on the list picks a definition");
        assert_eq!(s.tray_data().current_component, Some(i));

        // Edit tab: focus the name field by clicking down the panel, type, Enter.
        s.tray_ui.component_tab = ComponentTab::Edit;
        run(&ctx, &mut s, &spec, vec![]);
        let mut edited = None;
        for y in (150..600).step_by(6) {
            only_components(&mut s);
            run(&ctx, &mut s, &spec, click(pos2(x, y as f32)));
            if !ctx.egui_wants_keyboard_input() {
                continue;
            }
            let key = |key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let typed = vec![key(egui::Key::End), egui::Event::Text("X".into())];
            run(&ctx, &mut s, &spec, typed);
            let ev = run(&ctx, &mut s, &spec, vec![key(egui::Key::Enter)]);
            edited = ev.into_iter().next();
            break;
        }
        let name = format!("{}X", ["Door", "Window"][i]);
        assert_eq!(
            edited,
            Some(Event::ComponentEdit {
                index: i,
                name: name.clone(),
                description: String::new(),
            })
        );
        assert_eq!(s.tray_data().components[i].name, name);
    }
}
