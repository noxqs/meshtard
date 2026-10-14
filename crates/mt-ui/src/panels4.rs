//! Tray panel parts of LEFTOVERS.md 29: favorite collections (Materials,
//! Components, Styles), the Components panel's thumbnail grid (View Options) and its
//! Statistics scope (the picked component, the whole model or the selection).
//!
//! From memory of SketchUp, unverified: the menu entries' names and places, the
//! favorites heading in the collection dropdown, the tile sizes and the scope
//! dropdown (SketchUp's Statistics tab has an "All geometry" choice).

use egui::{Color32, Painter, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::panels::{icon_button, paint_cube};
use crate::theme::Theme32;
use crate::tray_data::{Collection, ScenesView, StatsScope, TrayData};
use crate::{CollectionKind, Event, Shell};

/// Details menu entries Add Collection to Favorites / Remove Collection from
/// Favorites for the collection `shown` (none for In Model).
pub(crate) fn favorite_items(
    ui: &mut egui::Ui,
    pending: &mut Vec<Event>,
    kind: CollectionKind,
    collections: &[Collection],
    shown: Option<usize>,
) {
    let shown = shown.filter(|&c| c < collections.len());
    let fav = shown.is_some_and(|c| collections[c].favorite);
    ui.separator();
    for (label, add) in [
        ("Add Collection to Favorites", true),
        ("Remove Collection from Favorites", false),
    ] {
        let enabled = shown.is_some() && fav != add;
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked()
            && let Some(collection) = shown
        {
            pending.push(Event::CollectionFavorite {
                kind,
                collection,
                add,
            });
            ui.close();
        }
    }
}

/// View Options icon: a 2 x 2 grid of squares.
fn paint_grid(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.1, ink);
    let h = r.width() * 0.5;
    for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let cell = Rect::from_min_size(r.min + vec2(x * h, y * h), vec2(h, h)).shrink(1.0);
        p.rect_stroke(cell, 0.0, s, StrokeKind::Middle);
    }
}

/// The tile size of a thumbnail view (`None`: list).
fn tile(view: ScenesView) -> Option<f32> {
    match view {
        ScenesView::List => None,
        ScenesView::SmallThumbnails => Some(48.0),
        ScenesView::LargeThumbnails => Some(96.0),
    }
}

/// A grid of square tiles: thumbnail (or the cube icon) per entry, the name as the
/// tooltip. Returns each entry's response.
fn thumb_grid(
    ui: &mut egui::Ui,
    t: &Theme32,
    size: f32,
    cells: &[(String, Option<egui::TextureId>, bool)],
) -> Vec<egui::Response> {
    let gap = 4.0;
    let w = ui.available_width();
    let per_row = (((w + gap) / (size + gap)).floor() as usize).max(1);
    let mut out = Vec::with_capacity(cells.len());
    for row in cells.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (name, thumb, selected) in row {
                let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                ui.painter().rect_filled(rect, 0.0, t.field);
                match thumb {
                    Some(tex) => {
                        ui.painter().image(
                            *tex,
                            rect.shrink(3.0),
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    None => paint_cube(
                        ui.painter(),
                        Rect::from_center_size(rect.center(), vec2(size * 0.45, size * 0.45)),
                        t.text_disabled,
                    ),
                }
                let stroke = if *selected {
                    Some(Stroke::new(2.0, t.selection))
                } else if r.hovered() {
                    Some(Stroke::new(1.0, t.hover_border))
                } else {
                    None
                };
                if let Some(s) = stroke {
                    ui.painter().rect_stroke(rect, 0.0, s, StrokeKind::Inside);
                }
                out.push(r.on_hover_text(name));
            }
        });
        ui.add_space(gap - ui.spacing().item_spacing.y);
    }
    out
}

impl Shell {
    /// What the Components Statistics tab counts while it is shown with a scope other
    /// than the picked component (the app then fills [`TrayData::model_stats`]).
    pub fn stats_scope(&self) -> Option<StatsScope> {
        let scope = self.tray_ui.more.stats_scope;
        (self.panel_expanded("window.components")
            && self.tray_ui.component_tab == crate::panels::ComponentTab::Statistics
            && scope != StatsScope::Component)
            .then_some(scope)
    }

    /// The Components panel's View Options button (list, small or large thumbnails).
    pub(crate) fn component_view_button(&mut self, ui: &mut egui::Ui, t: &Theme32) {
        let r = icon_button(ui, t, "View Options", true, paint_grid);
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(150.0);
            let view = &mut self.tray_ui.more.component_view;
            for (v, label) in [
                (ScenesView::SmallThumbnails, "Small Thumbnails"),
                (ScenesView::LargeThumbnails, "Large Thumbnails"),
                (ScenesView::List, "List"),
            ] {
                if ui.selectable_label(*view == v, label).clicked() {
                    *view = v;
                    ui.close();
                }
            }
        });
    }

    /// The In Model components as a thumbnail grid when View Options says so.
    /// Returns false for the list view (the caller draws it).
    pub(crate) fn components_grid(
        &mut self,
        ui: &mut egui::Ui,
        data: &mut TrayData,
        current: Option<usize>,
        t: &Theme32,
    ) -> bool {
        let Some(size) = tile(self.tray_ui.more.component_view) else {
            return false;
        };
        let cells: Vec<_> = (data.components.iter().enumerate())
            .map(|(i, c)| {
                let thumb = data.component_thumbnails.get(i).copied().flatten();
                let tip = format!("{}\n{}", c.name, crate::instances_label(c.instances));
                (tip, thumb, current == Some(i))
            })
            .collect();
        for (i, r) in thumb_grid(ui, t, size, &cells).into_iter().enumerate() {
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
        true
    }

    /// Collection `c` of the Components panel as a thumbnail grid when View Options
    /// says so (false: list view).
    pub(crate) fn component_collection_grid(
        &mut self,
        ui: &mut egui::Ui,
        data: &TrayData,
        c: usize,
        t: &Theme32,
    ) -> bool {
        let Some(size) = tile(self.tray_ui.more.component_view) else {
            return false;
        };
        let cells: Vec<_> = (data.component_collections[c].items.iter())
            .map(|it| (it.name.clone(), it.thumbnail, false))
            .collect();
        for (k, r) in thumb_grid(ui, t, size, &cells).into_iter().enumerate() {
            if r.clicked() {
                self.pending.push(Event::ComponentFromCollection {
                    collection: c,
                    item: k,
                });
            }
        }
        true
    }

    /// The Statistics tab's scope dropdown.
    pub(crate) fn stats_scope_combo(&mut self, ui: &mut egui::Ui) {
        let scope = &mut self.tray_ui.more.stats_scope;
        let label = |s: StatsScope| match s {
            StatsScope::Component => "Selected Component",
            StatsScope::Model => "All Geometry",
            StatsScope::Selection => "Selection",
        };
        egui::ComboBox::from_id_salt("mt_ui_stats_scope")
            .selected_text(label(*scope))
            .show_ui(ui, |ui| {
                for s in [
                    StatsScope::Component,
                    StatsScope::Model,
                    StatsScope::Selection,
                ] {
                    ui.selectable_value(scope, s, label(s));
                }
            })
            .response
            .on_hover_text("Count the picked component, the whole model or the selection");
    }
}

/// The hint shown when a scope has nothing to count.
pub(crate) fn empty_hint(scope: StatsScope) -> RichText {
    RichText::new(match scope {
        StatsScope::Component => "Pick a component in the Select tab to see its statistics.",
        StatsScope::Model => "The model is empty.",
        StatsScope::Selection => "Nothing is selected.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_by_view() {
        assert_eq!(tile(ScenesView::List), None);
        assert!(tile(ScenesView::SmallThumbnails) < tile(ScenesView::LargeThumbnails));
    }
}
