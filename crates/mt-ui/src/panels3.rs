//! Tray panel parts of LEFTOVERS.md 10: library collections of the Materials,
//! Components and Styles panels, the Materials Sample button and thumbnail style,
//! the Components Statistics tab, the Styles Mix tab, the Tags panel's Apply Tags to
//! Scenes dialog, the Outliner's section plane icon and the scene tab search.
//!
//! Layouts follow SketchUp 2026 as remembered and are unverified: where each
//! command sits (a Details menu here), the icons (own drawings), the Statistics
//! rows and the Mix tab (SketchUp drags a style onto a setting slot; here a style
//! is picked, then a setting group's button is clicked).

use egui::{
    Align2, Color32, FontId, Painter, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2,
};

use crate::panels::{ROW, icon_button, paint_details, paint_plus};
use crate::theme::Theme32;
use crate::tray_data::{
    Collection, STYLE_PARTS, StyleSource, ThumbnailStyle, TrayData, cube_shades, search_names,
};
use crate::{CollectionKind, Event, Shell};

/// The collection dropdown: "In Model" (`None`) and `collections`. Returns the
/// collection just chosen, if any.
pub(crate) fn collection_combo(
    ui: &mut egui::Ui,
    id: &str,
    collections: &[Collection],
    shown: &mut Option<usize>,
) -> Option<usize> {
    if shown.is_some_and(|c| c >= collections.len()) {
        *shown = None;
    }
    let label = shown.map_or("In Model", |c| collections[c].name.as_str());
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width((ui.available_width() - 8.0).clamp(80.0, 200.0))
        .show_ui(ui, |ui| {
            if ui.selectable_label(shown.is_none(), "In Model").clicked() {
                *shown = None;
            }
            for (c, col) in collections.iter().enumerate() {
                if ui.selectable_label(*shown == Some(c), &col.name).clicked() {
                    *shown = Some(c);
                    picked = Some(c);
                }
            }
        });
    picked
}

/// The Materials thumbnail style choices (Details menu and swatch right-click).
pub(crate) fn thumbnail_menu(ui: &mut egui::Ui, style: &mut ThumbnailStyle) {
    for (s, label) in [
        (ThumbnailStyle::Auto, "Auto Thumbnails"),
        (ThumbnailStyle::Cube, "Cube Thumbnails"),
        (ThumbnailStyle::Flat, "Flat Thumbnails"),
    ] {
        if ui.selectable_label(*style == s, label).clicked() {
            *style = s;
            ui.close();
        }
    }
}

/// A material swatch drawn as a cube seen from above: lighter top, darker right.
pub(crate) fn paint_cube_swatch(p: &Painter, rect: Rect, color: [u8; 3]) {
    let [top, left, right] = cube_shades(color).map(|c| Color32::from_rgb(c[0], c[1], c[2]));
    let r = rect.shrink(1.0);
    let c = r.center();
    let w = r.width() * 0.5;
    let h = r.height() * 0.5;
    let up = pos2(c.x, r.top());
    let ul = pos2(r.left(), c.y - h * 0.5);
    let ur = pos2(r.right(), c.y - h * 0.5);
    let mid = pos2(c.x, c.y);
    let dl = pos2(r.left(), c.y + h * 0.5);
    let dr = pos2(r.right(), c.y + h * 0.5);
    let down = pos2(c.x, r.bottom());
    let _ = w;
    let edge = Stroke::new(0.6, Color32::from_black_alpha(90));
    p.add(egui::Shape::convex_polygon(
        vec![up, ur, mid, ul],
        top,
        edge,
    ));
    p.add(egui::Shape::convex_polygon(
        vec![ul, mid, down, dl],
        left,
        edge,
    ));
    p.add(egui::Shape::convex_polygon(
        vec![mid, ur, dr, down],
        right,
        edge,
    ));
}

/// Section plane icon for the Outliner: a square plane with an arrow; filled when it
/// is the active cut.
pub(crate) fn paint_section_icon(p: &Painter, r: Rect, ink: Color32, active: bool) {
    let s = Stroke::new(1.1, ink);
    let plane = Rect::from_min_max(r.min + vec2(1.0, 3.0), r.max - vec2(5.0, 1.0));
    if active {
        p.rect_filled(plane, 0.0, ink.gamma_multiply(0.35));
    }
    p.rect_stroke(plane, 0.0, s, StrokeKind::Middle);
    let a = pos2(plane.center().x, plane.center().y);
    let b = pos2(r.right(), r.top() + 1.0);
    p.line_segment([a, b], s);
    p.line_segment([b, b + vec2(-3.5, 0.0)], s);
    p.line_segment([b, b + vec2(0.0, 3.5)], s);
}

/// Eyedropper icon (Materials Sample Paint).
fn paint_eyedropper(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.3, ink);
    let tip = pos2(r.left() + 1.0, r.bottom() - 1.0);
    let neck = pos2(r.right() - 5.0, r.top() + 5.0);
    p.line_segment([tip, neck], Stroke::new(2.0, ink));
    p.circle_filled(pos2(r.right() - 3.5, r.top() + 3.5), 2.6, ink);
    p.line_segment(
        [neck + vec2(-2.5, -2.5), neck + vec2(2.5, 2.5)],
        Stroke::new(1.6, ink),
    );
    p.line_segment([tip, tip + vec2(1.0, -1.0)], s);
}

/// Magnifier icon (scene tab search).
fn paint_search(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.4, ink);
    let c = r.center() + vec2(-1.5, -1.5);
    p.circle_stroke(c, r.width() * 0.3, s);
    p.line_segment([c + vec2(2.8, 2.8), r.right_bottom() - vec2(0.5, 0.5)], s);
}

/// Component icon for collection entries without a thumbnail.
fn paint_box(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.0, ink);
    let r = r.shrink(1.5);
    let d = r.width() * 0.3;
    let front = Rect::from_min_max(pos2(r.left(), r.top() + d), pos2(r.right() - d, r.bottom()));
    p.rect_stroke(front, 0.0, s, StrokeKind::Middle);
    for c in [front.left_top(), front.right_top(), front.right_bottom()] {
        p.line_segment([c, c + vec2(d, -d)], s);
    }
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

/// A list row of a collection: icon or thumbnail and name; returns the response.
fn list_row(
    ui: &mut egui::Ui,
    t: &Theme32,
    name: &str,
    thumbnail: Option<egui::TextureId>,
    selected: bool,
    icon: fn(&Painter, Rect, Color32),
) -> egui::Response {
    let w = ui.available_width();
    let h = if thumbnail.is_some() { 36.0 } else { ROW + 4.0 };
    let (row, r) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let (fill, ink) = if selected {
        (t.selection, Color32::WHITE)
    } else if r.hovered() {
        (t.hover, t.text)
    } else {
        (t.field, t.text)
    };
    ui.painter().rect_filled(row, 0.0, fill);
    let icon_rect = Rect::from_center_size(
        pos2(row.left() + 4.0 + (h - 6.0) / 2.0, row.center().y),
        vec2(h - 6.0, h - 6.0),
    );
    match thumbnail {
        Some(tex) => {
            ui.painter().image(
                tex,
                icon_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => icon(ui.painter(), icon_rect.shrink(1.0), ink),
    }
    let label = Rect::from_min_max(pos2(icon_rect.right() + 6.0, row.top()), row.max);
    ui.painter().with_clip_rect(label).text(
        pos2(label.left(), row.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(t.sizes.font),
        ink,
    );
    r.on_hover_text(name)
}

fn no_icon(_: &Painter, _: Rect, _: Color32) {}

/// A frame around the list drawn since `top`.
fn list_frame(ui: &egui::Ui, t: &Theme32, top: f32, w: f32) {
    let area = Rect::from_x_y_ranges(
        ui.min_rect().left()..=ui.min_rect().left() + w,
        top..=ui.cursor().top(),
    );
    ui.painter()
        .rect_stroke(area, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
}

impl Shell {
    /// Whether the Scenes panel is open showing thumbnails (the app then renders
    /// [`TrayData::scene_thumbnails`]).
    pub fn scene_thumbnails_shown(&self) -> bool {
        self.panel_expanded("window.scenes")
            && self.tray_ui.more.scenes_view != crate::tray_data::ScenesView::List
    }

    /// The Materials Select tab's header: Sample Paint, the collection dropdown, a
    /// Details menu and Create Material. When a collection is shown, draws its
    /// swatches and returns true (the caller then skips the In Model grid).
    pub(crate) fn materials_select_header(&mut self, ui: &mut egui::Ui, data: &TrayData) -> bool {
        let t = self.theme.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, &t, "Sample Paint", true, paint_eyedropper).clicked() {
                self.pending
                    .push(Event::Command("tool.paint_bucket".to_owned()));
                self.pending.push(Event::MaterialSample);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, &t, "Create Material", true, paint_plus).clicked() {
                    self.pending.push(Event::MaterialCreate);
                }
                let r = icon_button(ui, &t, "Details", true, paint_details);
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(190.0);
                    if ui.button("Open or Create a Collection...").clicked() {
                        self.pending
                            .push(Event::CollectionAdd(CollectionKind::Materials));
                        ui.close();
                    }
                    ui.separator();
                    thumbnail_menu(ui, &mut self.tray_ui.more.thumbnail_style);
                });
                let pick = collection_combo(
                    ui,
                    "mt_ui_material_collection",
                    &data.material_collections,
                    &mut self.tray_ui.more.material_collection,
                );
                if let Some(c) = pick {
                    self.pending.push(Event::CollectionOpened {
                        kind: CollectionKind::Materials,
                        collection: c,
                    });
                }
            });
        });
        let Some(c) = self
            .tray_ui
            .more
            .material_collection
            .filter(|&c| c < data.material_collections.len())
        else {
            return false;
        };
        let col = &data.material_collections[c];
        if col.items.is_empty() {
            ui.label(RichText::new("No materials in this collection.").color(t.text_disabled));
            return true;
        }
        let style = self.tray_ui.more.thumbnail_style;
        let size = 40.0;
        let gap = 4.0;
        let w = ui.available_width();
        let per_row = (((w + gap) / (size + gap)).floor() as usize).max(1);
        let items: Vec<usize> = (0..col.items.len()).collect();
        for row in items.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for &k in row {
                    let item = &col.items[k];
                    let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                    let inner = rect.shrink(2.0);
                    match (item.thumbnail, item.color) {
                        (Some(tex), _) if style != ThumbnailStyle::Cube => {
                            ui.painter().image(
                                tex,
                                inner,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        (thumb, color) => {
                            let color = color.unwrap_or([200, 200, 200]);
                            if style.cube(thumb.is_some()) {
                                paint_cube_swatch(ui.painter(), inner, color);
                            } else {
                                ui.painter().rect_filled(
                                    inner,
                                    0.0,
                                    Color32::from_rgb(color[0], color[1], color[2]),
                                );
                            }
                        }
                    }
                    if r.hovered() {
                        ui.painter().rect_stroke(
                            rect,
                            0.0,
                            Stroke::new(1.0, t.hover_border),
                            StrokeKind::Inside,
                        );
                    }
                    if r.on_hover_text(&item.name).clicked() {
                        self.pending.push(Event::MaterialFromCollection {
                            collection: c,
                            item: k,
                        });
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
        true
    }

    /// The Components Select tab showing a collection: click an entry to load and
    /// place it.
    pub(crate) fn component_collection_list(
        &mut self,
        ui: &mut egui::Ui,
        data: &TrayData,
        c: usize,
    ) {
        let t = self.theme.clone();
        let col = &data.component_collections[c];
        if col.items.is_empty() {
            ui.label(
                RichText::new("No components (.skp files) in this collection.")
                    .color(t.text_disabled),
            );
            return;
        }
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        for (k, item) in col.items.iter().enumerate() {
            if list_row(ui, &t, &item.name, item.thumbnail, false, paint_box).clicked() {
                self.pending.push(Event::ComponentFromCollection {
                    collection: c,
                    item: k,
                });
            }
        }
        list_frame(ui, &t, top, w);
    }

    /// The Components Statistics tab: counts of what the current definition holds,
    /// with nested components counted in when Expand is checked.
    pub(crate) fn components_stats(
        &mut self,
        ui: &mut egui::Ui,
        data: &TrayData,
        current: Option<usize>,
    ) {
        let t = self.theme.clone();
        if current.is_none() {
            ui.label(
                RichText::new("Pick a component in the Select tab to see its statistics.")
                    .color(t.text_disabled),
            );
            return;
        }
        ui.checkbox(&mut self.tray_ui.more.stats_expand, "Expand")
            .on_hover_text("Count the contents of nested components too");
        let expand = self.tray_ui.more.stats_expand;
        egui::Grid::new("mt_ui_component_stats")
            .num_columns(2)
            .striped(true)
            .spacing(vec2(16.0, 3.0))
            .show(ui, |ui| {
                for (label, own, nested) in &data.component_stats {
                    ui.label(label);
                    let n = if expand { nested } else { own };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(n.to_string());
                    });
                    ui.end_row();
                }
            });
    }

    /// The Styles Select tab showing a collection: click a style to add it to the
    /// model and apply it.
    pub(crate) fn style_collection_list(&mut self, ui: &mut egui::Ui, data: &TrayData, c: usize) {
        let t = self.theme.clone();
        let col = &data.style_collections[c];
        if col.items.is_empty() {
            ui.label(
                RichText::new("No styles (.style files) in this collection.")
                    .color(t.text_disabled),
            );
            return;
        }
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        for (k, item) in col.items.iter().enumerate() {
            if list_row(ui, &t, &item.name, item.thumbnail, false, no_icon).clicked() {
                self.pending.push(Event::StyleFromCollection {
                    collection: c,
                    item: k,
                });
            }
        }
        list_frame(ui, &t, top, w);
    }

    /// The Styles Mix tab: pick a style in the browser below (In Model or a
    /// collection), then click a setting group to take that group from it.
    pub(crate) fn styles_mix(&mut self, ui: &mut egui::Ui, data: &TrayData) {
        let t = self.theme.clone();
        let more = &mut self.tray_ui.more;
        let source_name = |s: StyleSource| -> Option<String> {
            match s {
                StyleSource::InModel(i) => data.styles.get(i).cloned(),
                StyleSource::Collection { collection, item } => data
                    .style_collections
                    .get(collection)?
                    .items
                    .get(item)
                    .map(|x| x.name.clone()),
            }
        };
        let source = more.mix_source.filter(|&s| source_name(s).is_some());
        ui.label(
            RichText::new(match source.and_then(source_name) {
                Some(n) => format!("From \"{n}\": click a setting group to use it."),
                None => "Pick a style below, then click a setting group.".to_owned(),
            })
            .color(t.text_disabled),
        );
        for (k, part) in STYLE_PARTS.iter().enumerate() {
            let b = egui::Button::new(*part).min_size(vec2(ui.available_width(), 0.0));
            if ui.add_enabled(source.is_some(), b).clicked()
                && let Some(source) = source
            {
                self.pending.push(Event::StyleMix { source, part: k });
            }
        }
        ui.add_space(4.0);
        ui.separator();
        let more = &mut self.tray_ui.more;
        collection_combo(
            ui,
            "mt_ui_mix_collection",
            &data.style_collections,
            &mut more.mix_collection,
        );
        let names: Vec<(StyleSource, String)> = match more.mix_collection {
            None => (data.styles.iter().enumerate())
                .map(|(i, n)| (StyleSource::InModel(i), n.clone()))
                .collect(),
            Some(c) => data.style_collections[c]
                .items
                .iter()
                .enumerate()
                .map(|(k, x)| {
                    (
                        StyleSource::Collection {
                            collection: c,
                            item: k,
                        },
                        x.name.clone(),
                    )
                })
                .collect(),
        };
        if names.is_empty() {
            ui.label(RichText::new("No styles here.").color(t.text_disabled));
            return;
        }
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        for (s, name) in names {
            if list_row(ui, &t, &name, None, source == Some(s), no_icon).clicked() {
                self.tray_ui.more.mix_source = Some(s);
            }
        }
        list_frame(ui, &t, top, w);
    }

    /// Tags Details > Apply Tags to Scenes: a dialog with a check box per scene; the
    /// current visibility of the selected tags (all tags when none is selected) is
    /// written into the checked scenes.
    pub(crate) fn tag_apply_dialog(&mut self, ui: &mut egui::Ui, data: &TrayData, tags: &[usize]) {
        let Some(mut checked) = self.tray_ui.more.tag_apply.take() else {
            return;
        };
        checked.resize(data.scenes.len(), true);
        let tags: Vec<usize> = if tags.is_empty() {
            (1..data.tags.len()).collect()
        } else {
            tags.to_vec()
        };
        let mut open = true;
        egui::Window::new("Apply Tags to Scenes")
            .id(egui::Id::new("mt_ui_tag_apply"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                let what = if tags.len() == data.tags.len().saturating_sub(1) {
                    "every tag".to_owned()
                } else if tags.len() == 1 {
                    format!("\"{}\"", data.tags[tags[0]].name)
                } else {
                    format!("the {} selected tags", tags.len())
                };
                ui.label(format!("Apply the current visibility of {what} to:"));
                ui.horizontal(|ui| {
                    if ui.small_button("All").clicked() {
                        checked.iter_mut().for_each(|c| *c = true);
                    }
                    if ui.small_button("None").clicked() {
                        checked.iter_mut().for_each(|c| *c = false);
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for (k, name) in data.scenes.iter().enumerate() {
                            ui.checkbox(&mut checked[k], name);
                        }
                    });
                ui.horizontal(|ui| {
                    let scenes: Vec<usize> = (0..checked.len()).filter(|&k| checked[k]).collect();
                    if ui
                        .add_enabled(
                            !scenes.is_empty() && !tags.is_empty(),
                            egui::Button::new("Apply"),
                        )
                        .clicked()
                    {
                        self.pending.push(Event::TagsApplyToScenes {
                            tags: tags.clone(),
                            scenes,
                        });
                        open = false;
                    }
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                });
            });
        if open && !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.tray_ui.more.tag_apply = Some(checked);
        }
    }

    /// The search button at the start of the scene tabs: a popup with a field that
    /// filters the scenes by name; click (or Enter on the first match) goes there.
    pub(crate) fn scene_tab_search(&mut self, ui: &mut egui::Ui, data: &TrayData) {
        let t = self.theme.clone();
        let r = icon_button(ui, &t, "Search Scenes", true, paint_search);
        if r.clicked() {
            let s = &mut self.tray_ui.more.scene_search;
            *s = if s.is_some() {
                None
            } else {
                Some(String::new())
            };
        }
        let Some(mut query) = self.tray_ui.more.scene_search.take() else {
            return;
        };
        let matches = search_names(&data.scenes, &query);
        let mut keep = true;
        let area = egui::Area::new(egui::Id::new("mt_ui_scene_search"))
            .order(egui::Order::Foreground)
            .fixed_pos(r.rect.left_bottom() + vec2(0.0, 2.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(220.0);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text("Search scenes")
                            .desired_width(f32::INFINITY),
                    );
                    if !field.has_focus() && !field.lost_focus() {
                        field.request_focus();
                    }
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter && let Some(&i) = matches.first() {
                        self.pending.push(Event::SceneActivate(i));
                        keep = false;
                    }
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            if matches.is_empty() {
                                ui.label(
                                    RichText::new("No matching scenes.").color(t.text_disabled),
                                );
                            }
                            for &i in &matches {
                                let active = data.active_scene == Some(i);
                                if ui
                                    .add(egui::Button::selectable(active, &data.scenes[i]))
                                    .clicked()
                                {
                                    self.pending.push(Event::SceneActivate(i));
                                    keep = false;
                                }
                            }
                        });
                });
            });
        let clicked_outside = ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !area.response.rect.contains(p) && !r.rect.contains(p))
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) || clicked_outside {
            keep = false;
        }
        if keep {
            self.tray_ui.more.scene_search = Some(query);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray_data::{CollectionItem, TagRow};

    /// Runs a frame of the whole shell with `events`.
    fn run(shell: &mut Shell, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<Event> {
        run_mods(shell, ctx, events, egui::Modifiers::NONE)
    }

    /// [`run`] with modifier keys held.
    fn run_mods(
        shell: &mut Shell,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Vec<Event> {
        let spec = mt_spec::Spec::builtin().expect("spec");
        let info = crate::EntityInfo::no_selection();
        let frame = crate::Frame {
            spec: &spec,
            active_tool: "tool.select",
            status: "",
            measurements_label: "Measurements",
            measurements_value: "",
            is_enabled: &|_| true,
            is_checked: &|_| false,
            entity_info: &info,
        };
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0))),
            events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                .chain(events)
                .collect(),
            // A second apart, so clicks are not double clicks.
            time: Some(ctx.input(|i| i.time) + 1.0),
            ..Default::default()
        };
        let mut out = Vec::new();
        let mut o = ctx.run_ui(input, |ui| out = shell.show(ui, &frame, |_| {}));
        o.textures_delta.clear();
        out
    }

    fn click_at(p: egui::Pos2) -> Vec<egui::Event> {
        let b = egui::PointerButton::Primary;
        vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: b,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: p,
                button: b,
                pressed: false,
                modifiers: Default::default(),
            },
        ]
    }

    /// Clicks a column of points down the tray until `want` comes back; `reset`
    /// runs before each click (a click may collapse a panel or switch a tab).
    fn click_until(
        s: &mut Shell,
        ctx: &egui::Context,
        x: f32,
        reset: impl Fn(&mut Shell),
        want: impl Fn(&Event) -> bool,
    ) -> Option<Event> {
        for y in (40..880).step_by(3) {
            reset(s);
            let p = pos2(x, y as f32);
            run(s, ctx, vec![egui::Event::PointerMoved(p)]);
            for e in run(s, ctx, click_at(p)) {
                if want(&e) {
                    return Some(e);
                }
            }
        }
        None
    }

    fn only(s: &mut Shell, id: &str) {
        for p in &mut s.panels {
            p.shown = true;
            p.expanded = p.id == id;
        }
    }

    #[test]
    fn material_collection_swatch_and_sample_button() {
        let spec = mt_spec::Spec::builtin().expect("spec");
        let mut s = Shell::new(&spec);
        s.open_panel("window.materials");
        only(&mut s, "window.materials");
        s.tray_ui.more.material_collection = Some(0);
        s.set_tray_data(TrayData {
            material_collections: vec![Collection {
                name: "Colors".into(),
                items: vec![CollectionItem {
                    name: "Red".into(),
                    color: Some([200, 0, 0]),
                    thumbnail: None,
                }],
            }],
            ..TrayData::default()
        });
        let ctx = egui::Context::default();
        run(&mut s, &ctx, vec![]);
        run(&mut s, &ctx, vec![]);
        let rect = s.last_tray_rects.first().map(|r| r.1).expect("tray drawn");
        // The first swatch sits at the tray's left edge.
        let reset = |s: &mut Shell| {
            only(s, "window.materials");
            s.tray_ui.material_tab = crate::panels::MaterialTab::Select;
            s.tray_ui.more.material_collection = Some(0);
        };
        let got = click_until(&mut s, &ctx, rect.left() + 30.0, reset, |e| {
            matches!(e, Event::MaterialFromCollection { .. })
        });
        assert_eq!(
            got,
            Some(Event::MaterialFromCollection {
                collection: 0,
                item: 0
            })
        );
        // The Sample Paint button (left of the header) sends the paint tool, then
        // the sample request.
        let mut sampled = false;
        for y in (40..400).step_by(3) {
            reset(&mut s);
            let p = pos2(rect.left() + 20.0, y as f32);
            run(&mut s, &ctx, vec![egui::Event::PointerMoved(p)]);
            let ev = run(&mut s, &ctx, click_at(p));
            if let Some(k) = ev.iter().position(|e| *e == Event::MaterialSample) {
                assert!(k > 0);
                assert_eq!(ev[k - 1], Event::Command("tool.paint_bucket".into()));
                sampled = true;
                break;
            }
        }
        assert!(sampled, "no Sample Paint button found");
    }

    #[test]
    fn tags_ctrl_click_selects_several_and_one_eye_sets_all() {
        let spec = mt_spec::Spec::builtin().expect("spec");
        let mut s = Shell::new(&spec);
        s.open_panel("window.tags");
        only(&mut s, "window.tags");
        let mut data = TrayData::default();
        for n in ["A", "B", "C"] {
            data.tags.push(TagRow {
                name: n.into(),
                ..TagRow::default()
            });
        }
        s.set_tray_data(data);
        // Select tags 1 and 3 directly (clicking rows is covered by the layout test
        // in panels2); the eye of tag 3 then hides both.
        s.tray_ui.more.tag_set = vec![
            crate::tray_data::TagTreeItem::Tag(1),
            crate::tray_data::TagTreeItem::Tag(3),
        ];
        let ctx = egui::Context::default();
        run(&mut s, &ctx, vec![]);
        run(&mut s, &ctx, vec![]);
        let rect = s.last_tray_rects.first().map(|r| r.1).expect("tray drawn");
        // Eye column: left of the color (26) and dashes (70) columns, inside the
        // body margin (8) and the scroll bar.
        let mut hit = None;
        for dx in [8.0, 12.0, 16.0, 20.0] {
            let x = rect.right() - 8.0 - 70.0 - 26.0 - dx;
            let reset = |s: &mut Shell| {
                only(s, "window.tags");
                s.tray_ui.more.tag_sort = crate::tray_data::TagSort::Model;
                s.tray_ui.more.tag_set = vec![
                    crate::tray_data::TagTreeItem::Tag(1),
                    crate::tray_data::TagTreeItem::Tag(3),
                ];
            };
            hit = click_until(&mut s, &ctx, x, reset, |e| {
                matches!(e, Event::TagsVisible(..) | Event::TagVisible(..))
            });
            if hit.is_some() {
                break;
            }
        }
        match hit {
            Some(Event::TagVisible(i, false)) => assert_eq!(i, 2, "unselected row clicked"),
            Some(Event::TagsVisible(tags, false)) => assert_eq!(tags, vec![1, 3]),
            other => panic!("no eye click: {other:?}"),
        }
    }

    /// Plain click selects one tag row, Ctrl+click adds one, Shift+click selects the
    /// range from the last clicked row.
    #[test]
    fn tag_rows_select_with_ctrl_and_shift() {
        use crate::tray_data::TagTreeItem::Tag;
        let spec = mt_spec::Spec::builtin().expect("spec");
        let mut s = Shell::new(&spec);
        s.open_panel("window.tags");
        only(&mut s, "window.tags");
        let mut data = TrayData::default();
        for n in ["A", "B", "C"] {
            data.tags.push(TagRow {
                name: n.into(),
                ..TagRow::default()
            });
        }
        s.set_tray_data(data);
        let ctx = egui::Context::default();
        run(&mut s, &ctx, vec![]);
        run(&mut s, &ctx, vec![]);
        let x = s.last_tray_rects[0].1.left() + 40.0;
        let click = |s: &mut Shell, y: f32, m: egui::Modifiers| {
            let p = pos2(x, y);
            run_mods(s, &ctx, vec![egui::Event::PointerMoved(p)], m);
            let mut ev = click_at(p);
            for e in &mut ev {
                if let egui::Event::PointerButton { modifiers, .. } = e {
                    *modifiers = m;
                }
            }
            run_mods(s, &ctx, ev, m);
            run(s, &ctx, vec![]);
        };
        // Find row A by clicking down the name column.
        let mut row_a = None;
        for y in (60..600).step_by(2) {
            only(&mut s, "window.tags");
            // A click on the Name header sorts; keep model order.
            s.tray_ui.more.tag_sort = crate::tray_data::TagSort::Model;
            click(&mut s, y as f32, egui::Modifiers::NONE);
            if s.tray_ui.more.tag_set == [Tag(1)] {
                row_a = Some(y as f32);
                break;
            }
        }
        let y = row_a.expect("row A found");
        click(&mut s, y + 2.0 * ROW, egui::Modifiers::CTRL);
        assert_eq!(s.tray_ui.more.tag_set, [Tag(1), Tag(3)]);
        click(&mut s, y + ROW, egui::Modifiers::SHIFT);
        assert_eq!(
            s.tray_ui.more.tag_set,
            [Tag(2), Tag(3)],
            "range from C to B"
        );
        click(&mut s, y + ROW, egui::Modifiers::CTRL);
        assert_eq!(s.tray_ui.more.tag_set, [Tag(3)], "Ctrl+click takes B out");
    }

    #[test]
    fn cube_shades_lighten_top_and_darken_right() {
        let [top, left, right] = cube_shades([100, 100, 100]);
        assert!(top[0] > left[0] && right[0] < left[0]);
        assert_eq!(left, [100, 100, 100]);
        assert_eq!(cube_shades([255, 255, 255])[0], [255, 255, 255]);
    }
}
