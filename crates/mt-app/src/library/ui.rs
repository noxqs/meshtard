//! The Model Library window: source, search, a grid of thumbnails, the selected
//! model's details and Download & Place (a modeless dialog, so the Place Component
//! tool works in the view while it stays open).

use eframe::egui;

use super::{Asset, Source, THUMB_PX, format_size};
use crate::App;

/// Grid cell size in points.
const CELL_W: f32 = 116.0;
const CELL_H: f32 = 132.0;
const THUMB_PT: f32 = 96.0;
/// Height kept for the details below the grid.
const DETAILS_H: f32 = 190.0;

/// What the window asks for this frame (done after drawing it).
#[derive(Default)]
struct Actions {
    search: bool,
    more: bool,
    select: Option<String>,
    place: bool,
    cancel: bool,
    add_folder: bool,
    remove_folder: Option<usize>,
    save_token: bool,
    open_url: Option<String>,
    cache_limit: bool,
    clear_cache: bool,
    clear_recent: bool,
}

impl App {
    pub(crate) fn library_command(&mut self, ctx: &egui::Context, id: &str) -> bool {
        if id != "window.3d_warehouse" {
            return false;
        }
        self.library.open(ctx);
        true
    }

    /// Polls the library's threads, renders a waiting thumbnail, places a finished
    /// download and draws the window while open.
    pub(crate) fn library_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.library.open && self.library.download.is_none() {
            return;
        }
        self.library.poll(ctx);
        self.render_library_thumb(ctx, frame);
        self.poll_library_download(frame);
        if self.library.open {
            let actions = self.library_window(ctx);
            self.library_actions(ctx, actions);
        }
    }

    fn poll_library_download(&mut self, frame: &eframe::Frame) {
        let Some(d) = &self.library.download else {
            return;
        };
        let Some(result) = d.poll() else { return };
        let asset = d.asset.clone();
        self.library.download = None;
        match result {
            Ok(model) => {
                let Some(rs) = frame.wgpu_render_state() else {
                    return;
                };
                let device = rs.device.clone();
                let name = if asset.name.is_empty() {
                    "Model".to_owned()
                } else {
                    asset.name.clone()
                };
                self.place_imported_with(model, &name, &device, |def| {
                    store_source(def, &asset);
                });
                self.library.add_recent(&asset);
                self.message = Some(format!(
                    "{}: click to place it. License: {}.",
                    asset.credit(),
                    asset.license
                ));
            }
            Err(e) => self.library.error = Some(e),
        }
    }

    /// Renders one local file's thumbnail offscreen (one per frame).
    fn render_library_thumb(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        let Some((key, model)) = self.library.to_render.pop() else {
            return;
        };
        let img = render_thumbnail(
            &mut self.library.viewport,
            &self.spec,
            &rs.device,
            &rs.queue,
            *model,
        );
        self.library.set_rendered_thumb(ctx, key, &img);
        if !self.library.to_render.is_empty() {
            ctx.request_repaint();
        }
    }

    fn library_actions(&mut self, ctx: &egui::Context, a: Actions) {
        let lib = &mut self.library;
        if let Some(id) = a.select {
            lib.select(id);
        }
        if a.search {
            lib.search();
        }
        if a.more {
            lib.more();
        }
        if a.place {
            lib.start_download(ctx);
        }
        if a.cancel
            && let Some(d) = &lib.download
        {
            d.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if a.add_folder {
            let start = lib.config.folders.last().cloned();
            if let Some(dir) = crate::filedialog::pick_folder(start.as_deref()) {
                lib.add_folder(dir);
            }
        }
        if let Some(i) = a.remove_folder {
            lib.remove_folder(i);
        }
        if a.save_token {
            lib.save_token();
        }
        if a.cache_limit {
            lib.apply_cache_limit(ctx);
        }
        if a.clear_cache {
            lib.clear_cache(ctx);
        }
        if a.clear_recent {
            lib.clear_recent();
        }
        // The thumbnail queue follows the filter typed this frame.
        lib.follow_filter();
        if let Some(url) = a.open_url
            && let Err(e) = crate::presentation::open_with_system(&url)
        {
            self.message = Some(format!("Could not open {url}: {e}"));
        }
    }

    fn library_window(&mut self, ctx: &egui::Context) -> Actions {
        let mut actions = Actions::default();
        let r = mt_ui::Dialog::new("model_library", "Model Library")
            .width(600.0)
            .modal(false)
            .show(ctx, |ui| self.library_body(ui, &mut actions));
        if r.closed {
            self.library.open = false;
        }
        actions
    }

    fn library_body(&mut self, ui: &mut egui::Ui, act: &mut Actions) {
        let lib = &mut self.library;
        ui.horizontal(|ui| {
            ui.label("Source:");
            let before = lib.source;
            egui::ComboBox::from_id_salt("mt_library_source")
                .selected_text(lib.source.label())
                .show_ui(ui, |ui| {
                    for s in lib.sources() {
                        ui.selectable_value(&mut lib.source, s, s.label());
                    }
                });
            if lib.source != before {
                act.search = true;
            }
            let edit = ui.add(
                egui::TextEdit::singleline(&mut lib.query)
                    .hint_text("Search models")
                    .desired_width(260.0),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            // Local and Poly Haven filter as you type; Sketchfab searches on Enter.
            let label = if lib.source == Source::Sketchfab {
                "Search"
            } else {
                "Refresh"
            };
            if ui.button(label).clicked() || (enter && lib.source == Source::Sketchfab) {
                act.search = true;
            }
        });
        match lib.source {
            Source::Local => {
                egui::CollapsingHeader::new("Folders")
                    .id_salt("mt_library_folders")
                    .show(ui, |ui| {
                        match crate::filedialog::folder(crate::filedialog::Folder::Components) {
                            Some(f) => {
                                ui.label(format!("{} (Preferences > Files > Components)", f.display()));
                            }
                            None => {
                                ui.weak("Set a Components folder in Preferences > Files, or add folders here.");
                            }
                        }
                        for (i, f) in lib.config.folders.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("Remove").clicked() {
                                    act.remove_folder = Some(i);
                                }
                                ui.label(f.display().to_string());
                            });
                        }
                        if ui.button("Add Folder...").clicked() {
                            act.add_folder = true;
                        }
                    });
            }
            Source::PolyHaven => {
                ui.horizontal(|ui| {
                    ui.weak("Models from");
                    if ui.link("Poly Haven").clicked() {
                        act.open_url = Some("https://polyhaven.com/models".into());
                    }
                    ui.weak("(CC0, free for any use). Not affiliated with Poly Haven.");
                });
            }
            Source::Sketchfab => {
                ui.weak(
                    "Downloadable models from Sketchfab. Each model has its own license; \
                     credit the author as it asks.",
                );
            }
            Source::Recent => {
                ui.horizontal(|ui| {
                    ui.weak("Models you downloaded and placed, newest first.");
                    if ui
                        .add_enabled(
                            !lib.config.recent.is_empty(),
                            egui::Button::new("Clear List"),
                        )
                        .clicked()
                    {
                        act.clear_recent = true;
                    }
                });
            }
        }
        egui::CollapsingHeader::new("Sketchfab account")
            .id_salt("mt_library_sketchfab")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("API token:");
                    ui.add(
                        egui::TextEdit::singleline(&mut lib.token_edit)
                            .password(true)
                            .desired_width(280.0),
                    );
                    if ui.button("Save").clicked() {
                        act.save_token = true;
                    }
                });
                ui.weak(
                    "Your own token from sketchfab.com > Settings > Password & API. It is \
                     kept in plain text in library.toml in the settings folder.",
                );
            });
        egui::CollapsingHeader::new("Download cache")
            .id_salt("mt_library_cache")
            .show(ui, |ui| {
                let ctx = ui.ctx().clone();
                lib.refresh_usage(&ctx);
                ui.horizontal(|ui| {
                    ui.label("Size limit:");
                    let r = ui.add(
                        egui::DragValue::new(&mut lib.config.cache_limit_mb)
                            .range(super::cache::MIN_LIMIT_MB..=1_000_000)
                            .speed(10.0)
                            .suffix(" MB"),
                    );
                    // Applied when a drag ends or a typed value is committed.
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        act.cache_limit = true;
                    }
                    ui.label(match lib.cache_usage() {
                        Some(b) => format!("{} used", format_size(b)),
                        None => "measuring...".to_owned(),
                    });
                    if ui
                        .add_enabled(lib.download.is_none(), egui::Button::new("Clear Cache"))
                        .clicked()
                    {
                        act.clear_cache = true;
                    }
                });
                ui.weak(
                    "Downloaded models and thumbnails; the least recently used are removed \
                     when the cache grows past the limit.",
                );
            });
        ui.separator();

        // Status line.
        let shown = lib.shown();
        ui.horizontal(|ui| {
            if lib.loading {
                ui.spinner();
                ui.label("Loading...");
            } else {
                ui.label(match shown.len() {
                    1 => "1 model".to_owned(),
                    n => format!("{n} models"),
                });
            }
            let queued = lib.queued_thumbs();
            if queued > 0 {
                ui.weak(format!("({queued} thumbnails to load)"));
            }
            if let Some(e) = &lib.error {
                ui.colored_label(egui::Color32::from_rgb(196, 43, 28), e);
            }
        });

        // Results grid.
        let width = ui.available_width().max(CELL_W);
        let cols = ((width / CELL_W).floor() as usize).max(1);
        let rows = shown.len().div_ceil(cols);
        let grid_h = (ui.ctx().content_rect().height() * 0.75 - DETAILS_H - 200.0).max(140.0);
        egui::ScrollArea::vertical()
            .id_salt("mt_library_grid")
            .max_height(grid_h)
            .min_scrolled_height(grid_h)
            .auto_shrink([false, false])
            .show_rows(ui, CELL_H, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        for &i in shown.iter().skip(row * cols).take(cols) {
                            let a = lib.assets[i].clone();
                            let tex = lib.thumbnail(&a);
                            let selected = lib.selected.as_ref() == Some(&a.id);
                            let loading = tex.is_none() && lib.thumb_pending(&a);
                            let r = cell(ui, &a, tex.as_ref(), selected, loading);
                            if r.clicked() {
                                act.select = Some(a.id.clone());
                            }
                            if r.double_clicked() {
                                act.place = true;
                            }
                        }
                    });
                }
                if lib.next_page.is_some() && ui.button("More results").clicked() {
                    act.more = true;
                }
            });
        ui.separator();

        // Details of the selected model.
        ui.allocate_ui(egui::vec2(ui.available_width(), DETAILS_H), |ui| {
            let Some(a) = lib.selected_asset().cloned() else {
                ui.weak("Select a model to see its details.");
                return;
            };
            ui.strong(&a.name);
            egui::Grid::new("mt_library_details")
                .num_columns(2)
                .show(ui, |ui| {
                    if !a.author.is_empty() {
                        ui.label("Author:");
                        ui.label(&a.author);
                        ui.end_row();
                    }
                    ui.label("License:");
                    if a.license_url.is_empty() {
                        ui.label(&a.license);
                    } else if ui.link(&a.license).clicked() {
                        act.open_url = Some(a.license_url.clone());
                    }
                    ui.end_row();
                    ui.label("Size:");
                    ui.label(match a.size {
                        Some(s) => format_size(s),
                        None if a.source == Source::PolyHaven => "(1k textures) ...".into(),
                        None => "unknown".into(),
                    });
                    ui.end_row();
                    if let Some(p) = a.polycount {
                        ui.label("Polygons:");
                        ui.label(p.to_string());
                        ui.end_row();
                    }
                    ui.label("Source:");
                    if a.page_url.is_empty() {
                        ui.label(a.source.label());
                    } else if ui.link(&a.page_url).clicked() {
                        act.open_url = Some(a.page_url.clone());
                    }
                    ui.end_row();
                });
            if !a.description.is_empty() {
                let d: String = a.description.chars().take(240).collect();
                ui.weak(d);
            }
            ui.horizontal(|ui| match &lib.download {
                Some(d) => {
                    let f = d.progress.fraction();
                    ui.add(
                        egui::ProgressBar::new(f)
                            .desired_width(240.0)
                            .text(format!("Downloading {}", d.asset.name)),
                    );
                    if ui.button("Cancel").clicked() {
                        act.cancel = true;
                    }
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
                None => {
                    let label = if a.local_path.is_some() {
                        "Place"
                    } else if lib.is_cached(&a) {
                        "Place (downloaded)"
                    } else {
                        "Download & Place"
                    };
                    if ui.button(label).clicked() {
                        act.place = true;
                    }
                }
            });
        });
    }
}

/// One grid cell: thumbnail (or a placeholder) and the name.
fn cell(
    ui: &mut egui::Ui,
    a: &Asset,
    tex: Option<&egui::TextureHandle>,
    selected: bool,
    loading: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(CELL_W - 6.0, CELL_H), egui::Sense::click());
    let p = ui.painter_at(rect);
    let visuals = ui.visuals();
    if selected {
        p.rect_filled(rect, 4.0, visuals.selection.bg_fill.gamma_multiply(0.35));
    } else if resp.hovered() {
        p.rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let img = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 6.0 + THUMB_PT / 2.0),
        egui::vec2(THUMB_PT, THUMB_PT),
    );
    match tex {
        Some(t) => {
            p.image(
                t.id(),
                img,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            p.rect_filled(img, 3.0, visuals.extreme_bg_color);
            // Loading: dots; no thumbnail: the file type (local) or "3D".
            let text = if loading {
                "...".to_owned()
            } else if a.source == Source::Local {
                a.tags.first().map_or("3D", String::as_str).to_uppercase()
            } else {
                "3D".to_owned()
            };
            p.text(
                img.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(13.0),
                visuals.weak_text_color(),
            );
        }
    }
    let name: String = if a.name.chars().count() > 18 {
        a.name.chars().take(17).collect::<String>() + "..."
    } else {
        a.name.clone()
    };
    p.text(
        egui::pos2(rect.center().x, img.bottom() + 6.0),
        egui::Align2::CENTER_TOP,
        name,
        egui::FontId::proportional(12.0),
        visuals.text_color(),
    );
    resp.on_hover_text(format!("{}\n{}", a.name, a.license))
}

/// Copies the model's source, license and credit into its definition (Entity Info >
/// Advanced Attributes: URL, Type, License; Components panel: description).
pub(crate) fn store_source(def: &mut mt_kernel::Definition, a: &Asset) {
    let attrs = &mut def.attributes;
    attrs.url = match &a.local_path {
        Some(p) => p.display().to_string(),
        None => a.page_url.clone(),
    };
    attrs.kind = format!("{} model", a.source.label());
    if a.source == Source::Local {
        // A local file's license is unknown: keep what the file itself had.
        if def.description.is_empty() {
            def.description = a.credit();
        }
        return;
    }
    attrs.license = a.license.clone();
    let mut d = a.credit();
    d += &format!(". License: {}", a.license);
    if !a.license_url.is_empty() {
        d += &format!(" ({})", a.license_url);
    }
    d.push('.');
    def.description = d;
}

/// Renders `model` from the default view direction, fitted, without axes.
fn render_thumbnail(
    viewport: &mut Option<mt_render::Viewport>,
    spec: &mt_spec::Spec,
    device: &egui_wgpu::wgpu::Device,
    queue: &egui_wgpu::wgpu::Queue,
    model: mt_kernel::Model,
) -> image::RgbaImage {
    let mut doc = crate::document::Document::untitled();
    doc.model = model;
    let scene = doc.scene();
    let mut style = mt_render::Style::from_spec(&spec.default_model);
    style.axes = None;
    let vp = viewport.get_or_insert_with(|| mt_render::Viewport::new(device, style.clone()));
    vp.set_style(device, style);
    crate::instancing::upload_faces(vp, device, &scene);
    vp.set_textures(&scene.textures);
    let (edges, instanced) = scene.flat_edges();
    vp.set_edges(device, &edges);
    vp.set_edge_instances(device, &scene.gpu_instances(instanced));
    let pts = scene
        .triangles
        .iter()
        .flatten()
        .copied()
        .chain(scene.edges.iter().flat_map(|s| {
            [
                glam::DVec3::from(glam::Vec3::from(s.a)),
                glam::DVec3::from(glam::Vec3::from(s.b)),
            ]
        }));
    let (mut min, mut max) = (glam::DVec3::splat(f64::MAX), glam::DVec3::splat(f64::MIN));
    for p in pts {
        min = min.min(p);
        max = max.max(p);
    }
    if min.x > max.x {
        (min, max) = (glam::DVec3::ZERO, glam::DVec3::ONE);
    }
    let mut camera = mt_render::Camera::from_spec(&spec.default_model.camera);
    camera.fit(min, max, 1.0);
    vp.render_image(device, queue, &camera, [THUMB_PX; 2], 1.0, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_license_and_source_on_the_definition() {
        let a = Asset {
            source: Source::PolyHaven,
            name: "Stool".into(),
            author: "A".into(),
            license: "CC0 1.0 (public domain)".into(),
            license_url: "https://creativecommons.org/publicdomain/zero/1.0/".into(),
            page_url: "https://polyhaven.com/a/stool".into(),
            ..Default::default()
        };
        let mut d = mt_kernel::Definition::default();
        store_source(&mut d, &a);
        assert_eq!(d.attributes.url, "https://polyhaven.com/a/stool");
        assert_eq!(d.attributes.license, "CC0 1.0 (public domain)");
        assert_eq!(d.attributes.kind, "Poly Haven model");
        assert!(
            d.description
                .starts_with("Stool by A, from Poly Haven. License: CC0")
        );
    }
}
