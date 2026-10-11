//! Tray panels of FEATURES.md 60: Tags (folders, filter, dashes, Purge, Select All
//! Tagged, Color by Tag), Outliner (rename, eye, drag to reparent, status icons),
//! Scenes (details, properties to save, Update dialog, reorder) and Styles (In Model
//! styles: Create, Update, Delete, Purge, apply).
//!
//! Layouts follow SketchUp 2026 as remembered and are unverified: column order and
//! widths, the icons (own drawings), where the commands sit (a Details menu here),
//! and the Update dialog's wording.

use std::collections::HashSet;

use egui::{
    Align2, Color32, FontId, Painter, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2,
};

use crate::panels::{
    ROW, icon_button, paint_arrow, paint_component_icon, paint_details, paint_eye,
    paint_group_icon, paint_minus, paint_model_icon, paint_pencil, paint_plus, paint_refresh,
    tab_button,
};
use crate::theme::Theme32;
use crate::tray_data::{
    DASH_NAMES, OutlinerKind, SCENE_PROPS, SelectMode, TagTreeItem, TrayData, outliner_can_move,
    outliner_node, outliner_rows, tag_tree_rows,
};
use crate::{Event, Shell};

/// State of the panels in this module.
#[derive(Debug, Default)]
pub(crate) struct MoreUi {
    /// Styles Edit tab, Watermark page: the selected watermark.
    pub(crate) watermark_selected: Option<usize>,
    tag_filter: String,
    /// Row selected in the Tags list (target of Delete and Add Folder).
    tag_selected: Option<TagTreeItem>,
    /// Tag or folder being renamed, the text, and whether focus was requested.
    tag_rename: Option<(TagTreeItem, String, bool)>,
    folders_collapsed: HashSet<usize>,
    /// A tag being dragged onto a folder.
    tag_drag: Option<usize>,
    outliner_expanded: HashSet<Vec<usize>>,
    outliner_root_collapsed: bool,
    outliner_filter: String,
    outliner_rename: Option<(Vec<usize>, String, bool)>,
    /// An Outliner row being dragged onto another.
    outliner_drag: Option<Vec<usize>>,
    /// Scene detail fields and the scene they were loaded from.
    scene_fields_of: Option<usize>,
    scene_name: String,
    scene_description: String,
    /// The Update dialog: scene and the properties to update.
    scene_update: Option<(usize, [bool; 7])>,
    /// Scenes panel details shown (SketchUp's Show Details toggle).
    scene_details_hidden: bool,
    styles_edit: bool,
    style_selected: Option<usize>,
}

/// Width of the Dashes column in the Tags panel.
const DASH_COL: f32 = 70.0;

/// Columns of a Tags row, right to left: dashes, color swatch, eye, pencil, name.
struct Columns {
    name: Rect,
    pencil: Rect,
    eye: Rect,
    color: Rect,
    dashes: Rect,
}

impl Columns {
    fn new(row: Rect) -> Self {
        let dashes = Rect::from_min_max(pos2(row.right() - DASH_COL, row.top()), row.max);
        let color = Rect::from_min_max(
            pos2(dashes.left() - 26.0, row.top()),
            pos2(dashes.left(), row.bottom()),
        );
        let eye = Rect::from_min_max(
            pos2(color.left() - 24.0, row.top()),
            pos2(color.left(), row.bottom()),
        );
        let pencil = Rect::from_min_max(
            pos2(eye.left() - 20.0, row.top()),
            pos2(eye.left(), row.bottom()),
        );
        let name = Rect::from_min_max(row.min, pos2(pencil.left(), row.bottom()));
        Self {
            name,
            pencil,
            eye,
            color,
            dashes,
        }
    }
}

impl Shell {
    /// Tags panel: Add Tag, Add Folder, Delete and a Details menu (Purge, Select All
    /// Tagged, Color by Tag), a filter field, then the tree of folders and tags with
    /// columns name (pencil on the active tag), eye, color and dashes. Double-click
    /// renames; right-click moves a tag into a folder; tags can also be dragged onto
    /// a folder.
    pub(crate) fn tags_panel2(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let t = self.theme.clone();
        let rows = tag_tree_rows(
            &data.tags,
            &data.tag_folders,
            &self.tray_ui.more.folders_collapsed,
            &self.tray_ui.more.tag_filter,
        );
        let valid = |item: TagTreeItem| match item {
            TagTreeItem::Tag(i) => i < data.tags.len(),
            TagTreeItem::Folder(f) => f < data.tag_folders.len(),
        };
        let selected = self.tray_ui.more.tag_selected.filter(|&s| valid(s));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, &t, "Add Tag", true, paint_plus).clicked() {
                self.pending.push(Event::TagAdd);
            }
            if icon_button(ui, &t, "Add Tag Folder", true, paint_folder_plus).clicked() {
                let (parent, tag) = match selected {
                    Some(TagTreeItem::Folder(f)) => (Some(f), None),
                    Some(TagTreeItem::Tag(i)) if i > 0 => (data.tags[i].folder, Some(i)),
                    _ => (None, None),
                };
                self.pending.push(Event::TagFolderAdd { parent, tag });
            }
            let can_delete = matches!(selected, Some(TagTreeItem::Folder(_)))
                || matches!(selected, Some(TagTreeItem::Tag(i)) if i > 0);
            if icon_button(ui, &t, "Delete", can_delete, paint_minus).clicked() {
                match selected {
                    Some(TagTreeItem::Tag(i)) if i > 0 => self.pending.push(Event::TagDelete(i)),
                    Some(TagTreeItem::Folder(f)) => self.pending.push(Event::TagFolderDelete(f)),
                    _ => {}
                }
                self.tray_ui.more.tag_selected = None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = icon_button(ui, &t, "Details", true, paint_details);
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(160.0);
                    if ui.button("Purge").clicked() {
                        self.pending.push(Event::TagPurge);
                        ui.close();
                    }
                    let tag = match selected {
                        Some(TagTreeItem::Tag(i)) => Some(i),
                        _ => None,
                    };
                    if ui
                        .add_enabled(tag.is_some(), egui::Button::new("Select All Tagged"))
                        .clicked()
                        && let Some(i) = tag
                    {
                        self.pending.push(Event::TagSelectAll(i));
                        ui.close();
                    }
                });
                let on = data.color_by_tag;
                let r = color_by_tag_button(ui, &t, on);
                if r.clicked() {
                    data.color_by_tag = !on;
                    self.pending.push(Event::ColorByTag(!on));
                }
            });
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.tray_ui.more.tag_filter)
                .hint_text("Filter")
                .desired_width(ui.available_width()),
        );
        ui.add_space(2.0);

        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;
        let (head, _) = ui.allocate_exact_size(vec2(w, ROW), Sense::hover());
        let cols = Columns::new(head);
        let p = ui.painter();
        p.rect_filled(head, 0.0, t.panel_header);
        let font = FontId::proportional(t.sizes.font);
        p.text(
            pos2(cols.name.left() + 4.0, head.center().y),
            Align2::LEFT_CENTER,
            "Name",
            font.clone(),
            t.text,
        );
        paint_eye(p, cols.eye, t.text, true);
        p.text(
            pos2(cols.dashes.left() + 4.0, head.center().y),
            Align2::LEFT_CENTER,
            "Dashes",
            font.clone(),
            t.text,
        );

        let released = ui.input(|i| i.pointer.any_released());
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let mut drop_on: Option<Option<usize>> = None;
        for (item, depth) in rows {
            let (row, resp) = ui.allocate_exact_size(vec2(w, ROW), Sense::click_and_drag());
            let cols = Columns::new(row);
            let is_sel = selected == Some(item);
            let dragging_here = self.tray_ui.more.tag_drag.is_some()
                && matches!(item, TagTreeItem::Folder(_))
                && pointer.is_some_and(|q| row.contains(q));
            let fill = if is_sel {
                t.pressed
            } else if resp.hovered() || dragging_here {
                t.hover
            } else {
                t.field
            };
            ui.painter().rect_filled(row, 0.0, fill);
            if resp.clicked() {
                self.tray_ui.more.tag_selected = Some(item);
            }
            let indent = 14.0 * depth as f32;
            let name_rect = Rect::from_min_max(
                pos2(cols.name.left() + indent, row.top()),
                cols.name.right_bottom(),
            );
            match item {
                TagTreeItem::Folder(f) => {
                    let folder = &data.tag_folders[f];
                    let arrow = Rect::from_center_size(
                        pos2(name_rect.left() + 7.0, row.center().y),
                        vec2(14.0, ROW),
                    );
                    let open = !self.tray_ui.more.folders_collapsed.contains(&f);
                    let a =
                        ui.interact(arrow, ui.id().with(("mt_ui_tag_folder", f)), Sense::click());
                    if a.clicked() {
                        if open {
                            self.tray_ui.more.folders_collapsed.insert(f);
                        } else {
                            self.tray_ui.more.folders_collapsed.remove(&f);
                        }
                    }
                    paint_arrow(ui.painter(), arrow.center(), open, t.text);
                    let icon = Rect::from_center_size(
                        pos2(arrow.right() + 9.0, row.center().y),
                        vec2(14.0, 12.0),
                    );
                    paint_folder(ui.painter(), icon, t.text);
                    let label = Rect::from_min_max(
                        pos2(icon.right() + 4.0, row.top()),
                        name_rect.right_bottom(),
                    );
                    self.tag_name(ui, item, label, &folder.name, &resp, &t);
                    let eye = ui.interact(
                        cols.eye,
                        ui.id().with(("mt_ui_folder_eye", f)),
                        Sense::click(),
                    );
                    paint_eye(ui.painter(), cols.eye, t.text, folder.visible);
                    if eye.clicked() {
                        self.pending
                            .push(Event::TagFolderVisible(f, !folder.visible));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Add Subfolder").clicked() {
                            self.pending.push(Event::TagFolderAdd {
                                parent: Some(f),
                                tag: None,
                            });
                            ui.close();
                        }
                        if ui.button("Delete Folder").clicked() {
                            self.pending.push(Event::TagFolderDelete(f));
                            ui.close();
                        }
                    });
                    if dragging_here && released {
                        drop_on = Some(Some(f));
                    }
                }
                TagTreeItem::Tag(i) => {
                    let tag = data.tags[i].clone();
                    let label = Rect::from_min_max(
                        pos2(name_rect.left() + 4.0, row.top()),
                        name_rect.right_bottom(),
                    );
                    self.tag_name(ui, item, label, &tag.name, &resp, &t);
                    if resp.drag_started() && i > 0 {
                        self.tray_ui.more.tag_drag = Some(i);
                    }
                    // Pencil: shown on the active tag, click sets the active tag.
                    let pencil = ui
                        .interact(
                            cols.pencil,
                            ui.id().with(("mt_ui_tag_pencil", i)),
                            Sense::click(),
                        )
                        .on_hover_text("Active tag");
                    if i == data.active_tag {
                        paint_pencil(ui.painter(), cols.pencil, t.text);
                    } else if pencil.hovered() {
                        paint_pencil(ui.painter(), cols.pencil, t.text_disabled);
                    }
                    if pencil.clicked() && i != data.active_tag {
                        self.pending.push(Event::TagActive(i));
                    }
                    // The active tag cannot be hidden (as remembered of SketchUp).
                    let can_hide = i != data.active_tag;
                    let eye = ui.interact(
                        cols.eye,
                        ui.id().with(("mt_ui_tag_eye", i)),
                        if can_hide {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    let ink = if can_hide { t.text } else { t.text_disabled };
                    paint_eye(ui.painter(), cols.eye, ink, tag.visible);
                    if eye.clicked() {
                        self.pending.push(Event::TagVisible(i, !tag.visible));
                    }
                    let mut color = tag.color;
                    let swatch = cols.color.shrink2(vec2(4.0, 3.0));
                    let r = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(swatch), |ui| {
                            ui.spacing_mut().interact_size = swatch.size();
                            egui::color_picker::color_edit_button_srgb(ui, &mut color)
                        })
                        .inner;
                    ui.painter().rect_stroke(
                        swatch,
                        0.0,
                        Stroke::new(1.0, t.border),
                        StrokeKind::Inside,
                    );
                    if r.changed() && color != tag.color {
                        self.pending.push(Event::TagColor(i, color));
                    }
                    let mut dashes = tag.dashes.min(DASH_NAMES.len() - 1);
                    // A child that does not grow the panel when the combo box is wider.
                    let mut cell = ui.new_child(
                        egui::UiBuilder::new().max_rect(cols.dashes.shrink2(vec2(2.0, 1.0))),
                    );
                    egui::ComboBox::from_id_salt(("mt_ui_tag_dashes", i))
                        .width(cols.dashes.width() - 30.0)
                        .selected_text(DASH_NAMES[dashes])
                        .show_ui(&mut cell, |ui| {
                            for (k, name) in DASH_NAMES.iter().enumerate() {
                                ui.selectable_value(&mut dashes, k, *name);
                            }
                        });
                    if dashes != tag.dashes {
                        data.tags[i].dashes = dashes;
                        self.pending.push(Event::TagDashes(i, dashes));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Select All Tagged").clicked() {
                            self.pending.push(Event::TagSelectAll(i));
                            ui.close();
                        }
                        if i > 0 {
                            ui.menu_button("Move to Folder", |ui| {
                                if ui
                                    .add_enabled(
                                        tag.folder.is_some(),
                                        egui::Button::new("(No Folder)"),
                                    )
                                    .clicked()
                                {
                                    self.pending.push(Event::TagMove {
                                        tag: i,
                                        folder: None,
                                    });
                                    ui.close();
                                }
                                for (f, folder) in data.tag_folders.iter().enumerate() {
                                    if ui
                                        .add_enabled(
                                            tag.folder != Some(f),
                                            egui::Button::new(&folder.name),
                                        )
                                        .clicked()
                                    {
                                        self.pending.push(Event::TagMove {
                                            tag: i,
                                            folder: Some(f),
                                        });
                                        ui.close();
                                    }
                                }
                            });
                        }
                    });
                }
            }
        }
        // Released outside every folder row: a tag dragged into empty space leaves its
        // folder.
        if released && let Some(tag) = self.tray_ui.more.tag_drag.take() {
            let target = match drop_on {
                Some(f) => Some(f),
                None => pointer.filter(|q| ui.min_rect().contains(*q)).map(|_| None),
            };
            if let Some(folder) = target
                && data.tags.get(tag).is_some_and(|t| t.folder != folder)
            {
                self.pending.push(Event::TagMove { tag, folder });
            }
        }
        let (bottom, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
        ui.painter()
            .hline(bottom.x_range(), bottom.top(), Stroke::new(1.0, t.border));
    }

    /// A tag's or folder's name in `rect`, or its rename field after a double click.
    fn tag_name(
        &mut self,
        ui: &mut egui::Ui,
        item: TagTreeItem,
        rect: Rect,
        name: &str,
        row: &egui::Response,
        t: &Theme32,
    ) {
        let more = &mut self.tray_ui.more;
        let renaming = matches!(&more.tag_rename, Some((r, _, _)) if *r == item);
        if renaming {
            let (_, text, focused) = more.tag_rename.as_mut().expect("renaming");
            let edit = egui::TextEdit::singleline(text)
                .margin(egui::Margin::symmetric(3, 1))
                .frame(
                    egui::Frame::NONE
                        .fill(t.field)
                        .stroke(Stroke::new(1.0, t.hover_border)),
                );
            let r = ui.put(rect.shrink2(vec2(1.0, 1.0)), edit);
            if !*focused {
                r.request_focus();
                *focused = true;
            }
            if r.lost_focus() {
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                let (_, text, _) = more.tag_rename.take().expect("renaming");
                let text = text.trim().to_owned();
                if !cancel && !text.is_empty() && text != name {
                    self.pending.push(match item {
                        TagTreeItem::Tag(i) => Event::TagRename(i, text),
                        TagTreeItem::Folder(f) => Event::TagFolderRename(f, text),
                    });
                }
            }
        } else {
            ui.painter().with_clip_rect(rect).text(
                pos2(rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(t.sizes.font),
                t.text,
            );
            if row.double_clicked() && item != TagTreeItem::Tag(0) {
                more.tag_rename = Some((item, name.to_owned(), false));
            }
        }
    }

    /// Outliner: filter field, then the tree with the model root, expand arrows,
    /// group/component icons, the eye per row, lock and open status icons, greyed
    /// hidden entries and highlighted selection. Double-click renames; dragging a row
    /// onto another group or component (or the root) moves it there.
    ///
    /// Unverified: the icon drawings, that the eye sits at the right end, the root's
    /// label (the app may pass the file name), and that Ctrl/Shift follow the Select
    /// tool's add/toggle keys.
    pub(crate) fn outliner_panel2(&mut self, ui: &mut egui::Ui, data: &TrayData) {
        let t = self.theme.clone();
        ui.add(
            egui::TextEdit::singleline(&mut self.tray_ui.more.outliner_filter)
                .hint_text("Filter")
                .desired_width(ui.available_width()),
        );
        ui.add_space(2.0);
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;
        let frame_top = ui.cursor().top();
        let released = ui.input(|i| i.pointer.any_released());
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let font = FontId::proportional(t.sizes.font);

        // Root row (a drop target: moves to the model root).
        let (row, _) = ui.allocate_exact_size(vec2(w, ROW), Sense::hover());
        let over_root =
            self.tray_ui.more.outliner_drag.is_some() && pointer.is_some_and(|q| row.contains(q));
        ui.painter()
            .rect_filled(row, 0.0, if over_root { t.hover } else { t.field });
        let arrow = Rect::from_center_size(pos2(row.left() + 8.0, row.center().y), vec2(12.0, ROW));
        if !data.outliner.is_empty() {
            let r = ui.interact(arrow, ui.id().with("mt_ui_outliner_root"), Sense::click());
            if r.clicked() {
                self.tray_ui.more.outliner_root_collapsed ^= true;
            }
            paint_arrow(
                ui.painter(),
                arrow.center(),
                !self.tray_ui.more.outliner_root_collapsed,
                t.text,
            );
        }
        let icon =
            Rect::from_center_size(pos2(row.left() + 24.0, row.center().y), vec2(14.0, 14.0));
        paint_model_icon(ui.painter(), icon, t.text);
        ui.painter().text(
            pos2(icon.right() + 5.0, row.center().y),
            Align2::LEFT_CENTER,
            &data.outliner_root,
            font.clone(),
            t.text,
        );
        let mut drop: Option<Vec<usize>> = over_root.then(Vec::new);

        if !self.tray_ui.more.outliner_root_collapsed {
            let rows = outliner_rows(
                &data.outliner,
                &self.tray_ui.more.outliner_expanded,
                &self.tray_ui.more.outliner_filter,
            );
            let mods = ui.input(|i| i.modifiers);
            for r in rows {
                let Some(node) = outliner_node(&data.outliner, &r.path) else {
                    continue;
                };
                let (row, resp) = ui.allocate_exact_size(vec2(w, ROW), Sense::click_and_drag());
                let over = self
                    .tray_ui
                    .more
                    .outliner_drag
                    .as_ref()
                    .is_some_and(|from| {
                        outliner_can_move(from, &r.path) && pointer.is_some_and(|q| row.contains(q))
                    });
                if over {
                    drop = Some(r.path.clone());
                }
                let fill = if node.selected {
                    t.selection
                } else if resp.hovered() || over {
                    t.hover
                } else {
                    t.field
                };
                ui.painter().rect_filled(row, 0.0, fill);
                let x = row.left() + 16.0 + 14.0 * r.depth as f32;
                let ink = match (node.selected, node.hidden) {
                    (true, _) => Color32::WHITE,
                    (false, true) => t.text_disabled,
                    (false, false) => t.text,
                };
                if r.has_children {
                    let arrow =
                        Rect::from_center_size(pos2(x + 8.0, row.center().y), vec2(14.0, ROW));
                    let a = ui.interact(
                        arrow,
                        ui.id().with(("mt_ui_outliner_arrow", &r.path)),
                        Sense::click(),
                    );
                    if a.clicked() {
                        let exp = &mut self.tray_ui.more.outliner_expanded;
                        if r.open {
                            exp.remove(&r.path);
                        } else {
                            exp.insert(r.path.clone());
                        }
                    }
                    paint_arrow(ui.painter(), arrow.center(), r.open, ink);
                }
                let icon = Rect::from_center_size(pos2(x + 24.0, row.center().y), vec2(14.0, 14.0));
                match node.kind {
                    OutlinerKind::Group => paint_group_icon(ui.painter(), icon, ink),
                    OutlinerKind::Component { .. } => paint_component_icon(ui.painter(), icon, ink),
                }
                // Status icons at the right: eye, lock, open.
                let eye = Rect::from_min_max(pos2(row.right() - 22.0, row.top()), row.max);
                let lock = eye.translate(vec2(-16.0, 0.0)).shrink2(vec2(3.0, 0.0));
                let open = lock.translate(vec2(-16.0, 0.0));
                let e = ui.interact(
                    eye,
                    ui.id().with(("mt_ui_outliner_eye", &r.path)),
                    Sense::click(),
                );
                paint_eye(ui.painter(), eye, ink, !node.instance_hidden);
                if e.on_hover_text("Hide / Unhide").clicked() {
                    self.pending.push(Event::OutlinerVisible {
                        path: r.path.clone(),
                        visible: node.instance_hidden,
                    });
                }
                if node.locked {
                    paint_lock(ui.painter(), lock, ink);
                }
                if node.open {
                    paint_open(ui.painter(), open, ink);
                }
                let label = Rect::from_min_max(
                    pos2(icon.right() + 5.0, row.top()),
                    pos2(open.left() - 2.0, row.bottom()),
                );
                let more = &mut self.tray_ui.more;
                let renaming = matches!(&more.outliner_rename, Some((p, _, _)) if *p == r.path);
                if renaming {
                    let (_, text, focused) = more.outliner_rename.as_mut().expect("renaming");
                    let field = ui.put(
                        label.shrink2(vec2(0.0, 1.0)),
                        egui::TextEdit::singleline(text).margin(egui::Margin::symmetric(2, 0)),
                    );
                    if !*focused {
                        field.request_focus();
                        *focused = true;
                    }
                    if field.lost_focus() {
                        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        let (path, text, _) = more.outliner_rename.take().expect("renaming");
                        let text = text.trim().to_owned();
                        if !cancel && text != node.name {
                            self.pending
                                .push(Event::OutlinerRename { path, name: text });
                        }
                    }
                } else {
                    ui.painter().with_clip_rect(label).text(
                        pos2(label.left(), row.center().y),
                        Align2::LEFT_CENTER,
                        node.label(),
                        font.clone(),
                        ink,
                    );
                    if resp.double_clicked() {
                        more.outliner_rename = Some((r.path.clone(), node.name.clone(), false));
                    }
                }
                if resp.drag_started() {
                    self.tray_ui.more.outliner_drag = Some(r.path.clone());
                }
                if resp.clicked() {
                    self.pending.push(Event::OutlinerSelect {
                        path: r.path.clone(),
                        mode: SelectMode::from_modifiers(mods.command || mods.ctrl, mods.shift),
                    });
                }
            }
        }
        if released
            && let Some(from) = self.tray_ui.more.outliner_drag.take()
            && let Some(to) = drop.filter(|to| outliner_can_move(&from, to))
        {
            self.pending.push(Event::OutlinerMove { from, to });
        }
        let area = Rect::from_min_max(
            pos2(ui.min_rect().left(), frame_top),
            pos2(ui.min_rect().left() + w, ui.cursor().top()),
        );
        ui.painter()
            .rect_stroke(area, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }

    /// Scenes panel: Update (asks which properties), Add, Delete, Move Up, Move Down,
    /// the scene list (scenes left out of the animation in parentheses) and the
    /// selected scene's details: name, description, Include in Animation and the
    /// properties it saves. Thumbnails are left out.
    pub(crate) fn scenes_panel2(&mut self, ui: &mut egui::Ui, data: &TrayData) {
        let t = self.theme.clone();
        let n = data.scenes.len();
        let active = data.active_scene.filter(|&i| i < n);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, &t, "Update Scene", active.is_some(), paint_refresh).clicked()
                && let Some(i) = active
            {
                let props = data.scene_details.get(i).map_or([true; 7], |d| d.props);
                self.tray_ui.more.scene_update = Some((i, props));
            }
            if icon_button(ui, &t, "Add Scene", true, paint_plus).clicked() {
                self.pending.push(Event::SceneAdd);
            }
            if icon_button(ui, &t, "Delete Scene", active.is_some(), paint_minus).clicked()
                && let Some(i) = active
            {
                self.pending.push(Event::SceneDelete(i));
            }
            let up = active.is_some_and(|i| i > 0);
            if icon_button(ui, &t, "Move Scene Up", up, paint_up).clicked()
                && let Some(i) = active
            {
                self.pending.push(Event::SceneMove { index: i, up: true });
            }
            let down = active.is_some_and(|i| i + 1 < n);
            if icon_button(ui, &t, "Move Scene Down", down, paint_down).clicked()
                && let Some(i) = active
            {
                self.pending.push(Event::SceneMove {
                    index: i,
                    up: false,
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let hidden = &mut self.tray_ui.more.scene_details_hidden;
                let label = if *hidden {
                    "Show Details"
                } else {
                    "Hide Details"
                };
                if ui.small_button(label).clicked() {
                    *hidden ^= true;
                }
            });
        });
        self.scene_update_dialog(ui, data);
        let w = ui.available_width();
        if data.scenes.is_empty() {
            ui.label(RichText::new("No scenes. Click + to add one.").color(t.text_disabled));
            return;
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        for (i, name) in data.scenes.iter().enumerate() {
            let (row, r) = ui.allocate_exact_size(vec2(w, ROW + 4.0), Sense::click());
            let (fill, ink) = if active == Some(i) {
                (t.selection, Color32::WHITE)
            } else if r.hovered() {
                (t.hover, t.text)
            } else {
                (t.field, t.text)
            };
            ui.painter().rect_filled(row, 0.0, fill);
            let included = data
                .scene_details
                .get(i)
                .is_none_or(|d| d.include_in_animation);
            let text = if included {
                name.clone()
            } else {
                format!("({name})")
            };
            ui.painter().with_clip_rect(row).text(
                pos2(row.left() + 8.0, row.center().y),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(t.sizes.font),
                ink,
            );
            if r.clicked() {
                self.pending.push(Event::SceneActivate(i));
            }
        }
        let area = Rect::from_x_y_ranges(
            ui.min_rect().left()..=ui.min_rect().left() + w,
            top..=ui.cursor().top(),
        );
        ui.painter()
            .rect_stroke(area, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.add_space(6.0);
        if self.tray_ui.more.scene_details_hidden {
            return;
        }
        let Some(i) = active else {
            return;
        };
        let details = data.scene_details.get(i).cloned().unwrap_or_default();
        let name_id = ui.id().with("mt_ui_scene_name");
        let desc_id = ui.id().with("mt_ui_scene_description");
        let focused = ui.memory(|m| m.has_focus(name_id) || m.has_focus(desc_id));
        let more = &mut self.tray_ui.more;
        if !focused || more.scene_fields_of != Some(i) {
            more.scene_fields_of = Some(i);
            more.scene_name.clone_from(&data.scenes[i]);
            more.scene_description.clone_from(&details.description);
        }
        ui.label("Name");
        let name = ui.add(
            egui::TextEdit::singleline(&mut more.scene_name)
                .id(name_id)
                .desired_width(w),
        );
        ui.label("Description");
        let desc = ui.add(
            egui::TextEdit::multiline(&mut more.scene_description)
                .id(desc_id)
                .desired_rows(2)
                .desired_width(w),
        );
        let mut include = details.include_in_animation;
        let include_changed = ui.checkbox(&mut include, "Include in Animation").changed();
        let commit = name.lost_focus() || desc.lost_focus();
        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if commit && cancel {
            more.scene_fields_of = None;
        } else if commit || include_changed {
            let new_name = match more.scene_name.trim() {
                "" => data.scenes[i].clone(),
                n => n.to_owned(),
            };
            let description = more.scene_description.trim_end().to_owned();
            if new_name != data.scenes[i]
                || description != details.description
                || include != details.include_in_animation
            {
                self.pending.push(Event::SceneDetailsEdit {
                    index: i,
                    name: new_name,
                    description,
                    include_in_animation: include,
                });
            }
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Properties to save:").strong());
        let mut props = details.props;
        for (k, label) in SCENE_PROPS.iter().enumerate() {
            ui.checkbox(&mut props[k], *label);
        }
        if props != details.props {
            self.pending.push(Event::SceneProps { index: i, props });
        }
    }

    /// The Update Scene dialog: which properties to update, then Update or Cancel.
    fn scene_update_dialog(&mut self, ui: &mut egui::Ui, data: &TrayData) {
        let Some((i, mut props)) = self.tray_ui.more.scene_update else {
            return;
        };
        let name = data.scenes.get(i).cloned().unwrap_or_default();
        let mut close = false;
        egui::Window::new("Update Scene")
            .id(egui::Id::new("mt_ui_scene_update"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.label(format!("Properties of \"{name}\" to update:"));
                for (k, label) in SCENE_PROPS.iter().enumerate() {
                    ui.checkbox(&mut props[k], *label);
                }
                ui.horizontal(|ui| {
                    if ui.button("Update").clicked() {
                        self.pending
                            .push(Event::SceneUpdateWith { index: i, props });
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        self.tray_ui.more.scene_update = (!close && i < data.scenes.len()).then_some((i, props));
    }

    /// Styles panel: the Select tab lists the model's styles (click applies one) with
    /// Create, Update, Delete and Purge Unused; the Edit tab edits the current
    /// settings. Collections, the Mix tab and .style files are left out.
    pub(crate) fn styles_panel2(&mut self, ui: &mut egui::Ui, data: &mut TrayData) {
        let t = self.theme.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (edit, label) in [(false, "Select"), (true, "Edit")] {
                if tab_button(ui, &t, label, self.tray_ui.more.styles_edit == edit).clicked() {
                    self.tray_ui.more.styles_edit = edit;
                }
            }
        });
        if self.tray_ui.more.styles_edit {
            self.styles_edit(ui, data);
            return;
        }
        let n = data.styles.len();
        let selected = self
            .tray_ui
            .more
            .style_selected
            .or(data.active_style)
            .filter(|&i| i < n);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, &t, "Create Style", true, paint_plus).clicked() {
                self.pending.push(Event::StyleCreate);
            }
            if icon_button(
                ui,
                &t,
                "Update Style with changes",
                selected.is_some(),
                paint_refresh,
            )
            .clicked()
                && let Some(i) = selected
            {
                self.pending.push(Event::StyleUpdate(i));
            }
            if icon_button(ui, &t, "Delete Style", selected.is_some(), paint_minus).clicked()
                && let Some(i) = selected
            {
                self.pending.push(Event::StyleDelete(i));
                self.tray_ui.more.style_selected = None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = icon_button(ui, &t, "Details", true, paint_details);
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(140.0);
                    if ui
                        .add_enabled(n > 0, egui::Button::new("Purge Unused"))
                        .clicked()
                    {
                        self.pending.push(Event::StylePurge);
                        ui.close();
                    }
                });
            });
        });
        ui.label("In Model");
        if n == 0 {
            ui.label(
                RichText::new(
                    "No styles in the model. Click + to make one from the current settings.",
                )
                .color(t.text_disabled),
            );
            return;
        }
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        for (i, name) in data.styles.iter().enumerate() {
            let (row, r) = ui.allocate_exact_size(vec2(w, ROW + 4.0), Sense::click());
            let (fill, ink) = if selected == Some(i) {
                (t.selection, Color32::WHITE)
            } else if r.hovered() {
                (t.hover, t.text)
            } else {
                (t.field, t.text)
            };
            ui.painter().rect_filled(row, 0.0, fill);
            let text = if data.active_style == Some(i) {
                format!("{name}  (in use)")
            } else {
                name.clone()
            };
            ui.painter().with_clip_rect(row).text(
                pos2(row.left() + 8.0, row.center().y),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(t.sizes.font),
                ink,
            );
            if r.clicked() {
                self.tray_ui.more.style_selected = Some(i);
                data.active_style = Some(i);
                self.pending.push(Event::StyleApply(i));
            }
        }
        let area = Rect::from_x_y_ranges(
            ui.min_rect().left()..=ui.min_rect().left() + w,
            top..=ui.cursor().top(),
        );
        ui.painter()
            .rect_stroke(area, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }
}

/// The Color by Tag toggle: three colored squares, framed when on.
fn color_by_tag_button(ui: &mut egui::Ui, t: &Theme32, on: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    if on || r.hovered() {
        ui.painter().rect(
            rect,
            0.0,
            if on { t.pressed } else { t.hover },
            Stroke::new(1.0, if on { t.pressed_border } else { t.hover_border }),
            StrokeKind::Inside,
        );
    }
    let inner = rect.shrink(5.0);
    let h = inner.height() / 3.0;
    for (k, c) in [
        Color32::from_rgb(220, 80, 80),
        Color32::from_rgb(80, 170, 80),
        Color32::from_rgb(70, 110, 220),
    ]
    .into_iter()
    .enumerate()
    {
        let y = inner.top() + h * k as f32;
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), h - 1.0)),
            0.0,
            c,
        );
    }
    r.on_hover_text("Color by Tag")
}

/// Folder icon.
fn paint_folder(p: &Painter, r: Rect, ink: Color32) {
    let s = Stroke::new(1.1, ink);
    let tab = r.top() + 3.0;
    p.add(egui::Shape::closed_line(
        vec![
            pos2(r.left(), r.top()),
            pos2(r.left() + r.width() * 0.4, r.top()),
            pos2(r.left() + r.width() * 0.5, tab),
            pos2(r.right(), tab),
            pos2(r.right(), r.bottom()),
            pos2(r.left(), r.bottom()),
        ],
        s,
    ));
}

/// Add Tag Folder: a folder with a plus.
fn paint_folder_plus(p: &Painter, r: Rect, ink: Color32) {
    paint_folder(p, r.shrink2(vec2(0.0, 2.0)), ink);
    let c = r.center() + vec2(0.0, 1.0);
    let s = Stroke::new(1.4, ink);
    p.line_segment([c - vec2(3.0, 0.0), c + vec2(3.0, 0.0)], s);
    p.line_segment([c - vec2(0.0, 3.0), c + vec2(0.0, 3.0)], s);
}

/// Lock status icon.
fn paint_lock(p: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    let body = Rect::from_center_size(c + vec2(0.0, 2.0), vec2(8.0, 6.0));
    p.rect_filled(body, 1.0, ink);
    let pts: Vec<_> = (0..=10)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 10.0);
            c + vec2(0.0, -1.0) + vec2(a.cos(), a.sin()) * 3.0
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.3, ink)));
}

/// Open (being edited) status icon: an open box.
fn paint_open(p: &Painter, r: Rect, ink: Color32) {
    let r = Rect::from_center_size(r.center(), vec2(10.0, 9.0));
    let s = Stroke::new(1.2, ink);
    p.add(egui::Shape::line(
        vec![
            r.left_top(),
            r.left_bottom(),
            r.right_bottom(),
            r.right_top(),
        ],
        s,
    ));
    p.line_segment([r.left_top(), r.left_top() + vec2(-2.0, -2.0)], s);
    p.line_segment([r.right_top(), r.right_top() + vec2(2.0, -2.0)], s);
}

fn paint_up(p: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    p.add(egui::Shape::convex_polygon(
        vec![c + vec2(0.0, -4.5), c + vec2(5.0, 3.0), c + vec2(-5.0, 3.0)],
        ink,
        Stroke::NONE,
    ));
}

fn paint_down(p: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    p.add(egui::Shape::convex_polygon(
        vec![
            c + vec2(-5.0, -3.0),
            c + vec2(5.0, -3.0),
            c + vec2(0.0, 4.5),
        ],
        ink,
        Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityInfo, Frame, OutlinerNode, TagFolderRow, TagRow};
    use mt_spec::Spec;

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

    #[test]
    fn tags_folders_and_outliner_status_draw() {
        let spec = Spec::builtin().unwrap();
        let ctx = egui::Context::default();
        let mut s = Shell::new(&spec);
        s.set_tray_data(TrayData {
            tags: vec![
                TagRow {
                    name: "Untagged".into(),
                    ..TagRow::default()
                },
                TagRow {
                    name: "Walls".into(),
                    folder: Some(0),
                    dashes: 2,
                    ..TagRow::default()
                },
            ],
            tag_folders: vec![TagFolderRow {
                name: "Building".into(),
                parent: None,
                visible: false,
            }],
            outliner: vec![OutlinerNode {
                name: "House".into(),
                locked: true,
                open: true,
                instance_hidden: true,
                hidden: true,
                ..OutlinerNode::default()
            }],
            scenes: vec!["Scene 1".into(), "Scene 2".into()],
            scene_details: vec![Default::default(), Default::default()],
            active_scene: Some(1),
            styles: vec!["Plain".into()],
            ..TrayData::default()
        });
        for id in [
            "window.tags",
            "window.outliner",
            "window.scenes",
            "window.styles",
        ] {
            s.open_panel(id);
        }
        // Drawing the panels with nothing clicked sends no events.
        for _ in 0..3 {
            assert!(run(&ctx, &mut s, &spec, Vec::new()).is_empty());
        }
    }
}
