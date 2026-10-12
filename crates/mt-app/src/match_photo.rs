//! Camera > Match New Photo and Edit Matched Photo (FEATURES.md 30), after SketchUp's
//! help page "Matching a Photo to a Model (or a Model to a Photo)"
//! (help.sketchup.com/en/sketchup/matching-photo-model-or-model-photo):
//!
//! - choosing the photo switches to the Match Photo view and saves it as a scene tab
//!   named after the file;
//! - the photo is drawn behind the model (Background) or in front of it, with an
//!   opacity; Model unchecked hides the model to line up the bars;
//! - two red and two green perspective bars with square grips at their ends and
//!   dashed lines to their vanishing points, the yellow horizon through both
//!   vanishing points (dragging it moves every bar), the axes origin and the blue
//!   axis, whose grip scales the model;
//! - the camera (position, direction, field of view) is solved live from them
//!   (`mt_kernel::photo_match`), with the grid of the chosen planes drawn on the photo;
//! - grid style Inside / Above / Below (lays the bars out for that kind of photo),
//!   planes, spacing, Grid on or off; Project Textures From Photo; Done, Cancel;
//! - selecting the scene shows the photo again until the camera moves (orbiting
//!   away leaves the photo, as the help says).
//!
//! The matched photo lives in the scene while the model is open (part of undo); it is
//! not saved in `.skp` files. Unverified against SketchUp: the panel's exact layout
//! and defaults (spacing, which planes), what Inside / Above / Below do beyond laying
//! out the bars, dragging the horizon, and that Cancel is offered.

use std::sync::Arc;

use eframe::egui;
use glam::{DMat2, DVec2, DVec3};
use mt_kernel::photo_match::{self, GridStyle, MatchError, MatchedPhoto, PhotoCamera};
use mt_render::Camera;

use crate::App;

/// Photo pixels to viewport pixels: a rotation and scale about the centers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    m: DMat2,
    photo_center: DVec2,
    view_center: DVec2,
}

impl Placement {
    pub(crate) fn to_view(self, p: DVec2) -> DVec2 {
        self.view_center + self.m * (p - self.photo_center)
    }

    pub(crate) fn to_photo(self, q: DVec2) -> DVec2 {
        self.photo_center + self.m.inverse() * (q - self.view_center)
    }
}

/// The app camera of a solved photo camera in a viewport of `viewport` pixels, and
/// where the photo goes. The photo is fit inside the viewport; the app's camera has
/// no roll, so the photo is turned by the photo camera's roll instead.
pub(crate) fn view_of(cam: &PhotoCamera, viewport: DVec2) -> (Camera, Placement) {
    let vp = viewport.max(DVec2::ONE);
    let s = (vp.x / cam.size.x).min(vp.y / cam.size.y);
    let fov = (2.0 * (vp.y / 2.0 / (cam.focal * s)).atan()).to_degrees();
    let eye = cam.eye();
    let camera = Camera {
        eye,
        target: eye + cam.forward() * cam.depth().abs().max(1.0),
        fov_deg: fov,
        perspective: true,
        two_point: false,
    };
    let (r0, d0) = (camera.right(), -camera.up());
    let (r, d) = (cam.right(), cam.down());
    let m = DMat2::from_cols(
        DVec2::new(r0.dot(r), d0.dot(r)),
        DVec2::new(r0.dot(d), d0.dot(d)),
    );
    (
        camera,
        Placement {
            m: m * s,
            photo_center: cam.principal,
            view_center: vp / 2.0,
        },
    )
}

/// A grip being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Handle {
    /// A bar end ([`photo_match::PhotoBars::ends`] order).
    End(usize),
    Origin,
    Scale,
    /// The horizon, with the last photo point it was dragged to.
    Horizon(DVec2),
}

/// A Match Photo session.
struct Edit {
    photo: MatchedPhoto,
    /// The scene it belongs to.
    page: usize,
    /// The scene was made by Match New Photo (Cancel removes it).
    new: bool,
    /// The view before matching started (Cancel goes back to it for a new photo).
    camera_before: Camera,
    solved: Result<PhotoCamera, MatchError>,
    /// Where the photo is drawn: kept still while a grip is dragged (the photo turns
    /// with the camera's roll on release).
    placement: Option<Placement>,
    drag: Option<Handle>,
    spacing_text: String,
}

/// A scene's photo on screen.
struct Shown {
    photo: Box<MatchedPhoto>,
    page: usize,
    camera: Camera,
    placement: Placement,
    viewport: DVec2,
}

/// Match Photo state of the app.
#[derive(Default)]
pub(crate) struct MatchPhoto {
    edit: Option<Edit>,
    shown: Option<Shown>,
    /// The photo's texture, for the image bytes it was made from.
    texture: Option<(Arc<[u8]>, egui::TextureHandle)>,
    /// Camera > Image Igloo, while shown (`image_igloo`).
    pub(crate) igloo: Option<crate::image_igloo::Igloo>,
}

/// Grip size in points.
const GRIP: f32 = 7.0;
/// Grid lines per axis.
const GRID_LINES: usize = 10;
const RED: egui::Color32 = egui::Color32::from_rgb(220, 0, 0);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0, 160, 0);
const BLUE: egui::Color32 = egui::Color32::from_rgb(0, 0, 230);
const YELLOW: egui::Color32 = egui::Color32::from_rgb(240, 200, 0);

fn decode(data: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory(data).ok().map(|i| i.to_rgba8())
}

impl Edit {
    fn new(
        photo: MatchedPhoto,
        page: usize,
        new: bool,
        camera_before: Camera,
        units: mt_infer::units::LengthUnit,
    ) -> Self {
        let solved = photo_match::solve(&photo.bars);
        Self {
            spacing_text: units.format(photo.spacing),
            photo,
            page,
            new,
            camera_before,
            solved,
            placement: None,
            drag: None,
        }
    }

    /// Solves again after the bars changed; the scale point follows the blue axis.
    fn resolve(&mut self) {
        self.solved = photo_match::solve(&self.photo.bars);
        if let Ok(cam) = &self.solved {
            self.photo.bars.snap_scale(cam);
        }
    }
}

impl App {
    /// Camera > Match New Photo and Edit Matched Photo; false for other commands.
    pub(crate) fn match_photo_command(&mut self, id: &str) -> bool {
        match id {
            "camera.match_new_photo" => {
                let filters: [(&str, &[&str]); 1] =
                    [("Images (*.png, *.jpg)", &["png", "jpg", "jpeg"])];
                if let Some(path) = crate::filedialog::open(&filters) {
                    self.match_new_photo(&path);
                }
                true
            }
            "camera.edit_matched_photo" => {
                self.image_igloo_leave();
                self.edit_matched_photo();
                true
            }
            "camera.image_igloo" => {
                self.image_igloo_command();
                true
            }
            _ => false,
        }
    }

    /// No photo is being matched.
    pub(crate) fn match_photo_editing_none(&self) -> bool {
        self.match_photo.edit.is_none()
    }

    /// Stops showing a scene's photo (the camera is about to leave its view).
    pub(crate) fn match_photo_hide_shown(&mut self) {
        self.match_photo.shown = None;
    }

    /// Whether Edit Matched Photo applies: a scene's photo is shown, or the selected
    /// scene has one.
    pub(crate) fn can_edit_matched_photo(&self) -> bool {
        self.match_photo.edit.is_none()
            && (self.match_photo.shown.is_some()
                || self.active_scene.is_some_and(|i| {
                    self.document.as_ref().is_some_and(|d| {
                        d.model
                            .pages
                            .get(i)
                            .is_some_and(|p| p.matched_photo.is_some())
                    })
                }))
    }

    /// Starts matching the photo at `path`: a new scene named after the file holds it.
    pub(crate) fn match_new_photo(&mut self, path: &std::path::Path) {
        let data: Arc<[u8]> = match std::fs::read(path) {
            Ok(d) => d.into(),
            Err(e) => return self.file_error("open", path, e),
        };
        let size = match image::load_from_memory(&data) {
            Ok(i) => DVec2::new(f64::from(i.width()), f64::from(i.height())),
            Err(e) => return self.file_error("open", path, e),
        };
        let name = path
            .file_name()
            .map_or_else(|| "Photo".into(), |n| n.to_string_lossy().into_owned());
        let mut photo = MatchedPhoto::new(name.clone(), data, size, GridStyle::Above);
        if let Some(bars) = test_bars(size) {
            photo.bars = bars;
        }
        let camera = self.camera.clone();
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        let units = doc.units;
        let stored = photo.clone();
        doc.edit_model(|m, _| {
            let mut page = crate::tray_more::new_page(m, name, &camera);
            page.matched_photo = Some(Box::new(stored));
            m.pages.push(page);
            true
        });
        let page = doc.model.pages.len() - 1;
        self.active_scene = Some(page);
        self.transition = None;
        self.match_photo.shown = None;
        self.match_photo.edit = Some(Edit::new(photo, page, true, camera, units));
        self.message = None;
    }

    /// Edit Matched Photo: lines up the shown (or selected) scene's photo again.
    pub(crate) fn edit_matched_photo(&mut self) {
        let page = match &self.match_photo.shown {
            Some(s) => Some(s.page),
            None => self.active_scene,
        };
        let Some(doc) = &self.document else { return };
        let Some((i, photo)) = page.and_then(|i| {
            let p = doc.model.pages.get(i)?.matched_photo.as_deref()?;
            Some((i, p.clone()))
        }) else {
            return;
        };
        self.transition = None;
        self.match_photo.shown = None;
        self.match_photo.edit = Some(Edit::new(photo, i, false, self.camera.clone(), doc.units));
    }

    /// Shows scene `page`'s photo (selecting the scene): the camera jumps to the
    /// matched view.
    pub(crate) fn show_matched_photo(&mut self, page: usize) {
        let Some(photo) = self
            .document
            .as_ref()
            .and_then(|d| d.model.pages.get(page)?.matched_photo.clone())
        else {
            return;
        };
        let Ok(cam) = photo_match::solve(&photo.bars) else {
            return;
        };
        let (camera, placement) = view_of(&cam, self.viewport_px);
        self.transition = None;
        self.camera = camera.clone();
        self.match_photo.shown = Some(Shown {
            photo,
            page,
            camera,
            placement,
            viewport: self.viewport_px,
        });
    }

    /// Whether Match Photo takes the viewport's mouse this frame (while editing):
    /// drags the grips, keeps the camera on the solved view.
    pub(crate) fn match_photo_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
    ) -> bool {
        self.match_photo_check_shown(viewport_px);
        let ppp = f64::from(ui.ctx().pixels_per_point());
        let Some(edit) = self.match_photo.edit.as_mut() else {
            return false;
        };
        let to_px = |p: egui::Pos2| {
            DVec2::new(f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y)) * ppp
        };
        let placement = match (edit.placement, &edit.solved) {
            (Some(p), _) if edit.drag.is_some() => p,
            (_, Ok(cam)) => view_of(cam, viewport_px).1,
            (Some(p), Err(_)) => p,
            (None, Err(_)) => {
                // Unsolved from the start: the photo fit without turning.
                let size = edit.photo.bars.size;
                let s = (viewport_px.x / size.x).min(viewport_px.y / size.y);
                Placement {
                    m: DMat2::from_diagonal(DVec2::splat(s)),
                    photo_center: size / 2.0,
                    view_center: viewport_px / 2.0,
                }
            }
        };
        edit.placement = Some(placement);
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(at) = ui.input(|i| i.pointer.press_origin())
        {
            edit.drag = grip_at(edit, &placement, to_px(at), f64::from(GRIP) * ppp);
        }
        if let (Some(handle), Some(pos)) = (edit.drag, response.interact_pointer_pos())
            && response.dragged_by(egui::PointerButton::Primary)
        {
            let p = placement.to_photo(to_px(pos));
            drag_grip(edit, handle, p);
        }
        if response.drag_stopped() {
            edit.drag = None;
        }
        if let Ok(cam) = &edit.solved {
            self.camera = view_of(cam, viewport_px).0;
        }
        self.transition = None;
        true
    }

    /// Drops the shown photo when the camera left its view or the scene changed, and
    /// follows viewport size changes.
    fn match_photo_check_shown(&mut self, viewport_px: DVec2) {
        let Some(shown) = self.match_photo.shown.as_mut() else {
            return;
        };
        let same = self.document.as_ref().is_some_and(|d| {
            d.model
                .pages
                .get(shown.page)
                .and_then(|p| p.matched_photo.as_deref())
                .is_some_and(|p| Arc::ptr_eq(&p.data, &shown.photo.data))
        });
        if !same || self.camera != shown.camera {
            self.match_photo.shown = None;
            return;
        }
        if shown.viewport != viewport_px
            && let Ok(cam) = photo_match::solve(&shown.photo.bars)
        {
            let (camera, placement) = view_of(&cam, viewport_px);
            self.camera = camera.clone();
            shown.camera = camera;
            shown.placement = placement;
            shown.viewport = viewport_px;
        }
    }

    /// The photo being shown or edited, and where it goes.
    fn match_photo_current(&self) -> Option<(&MatchedPhoto, Placement)> {
        if let Some(e) = &self.match_photo.edit {
            return Some((&e.photo, e.placement?));
        }
        let s = self.match_photo.shown.as_ref()?;
        (self.camera == s.camera).then_some((&*s.photo, s.placement))
    }

    /// The viewport leaves its background out: the photo is drawn behind the model.
    pub(crate) fn match_photo_behind(&self) -> bool {
        self.match_photo_current()
            .is_some_and(|(p, _)| p.background && p.show_model)
    }

    /// Paints the photo behind the model; returns whether the model is drawn.
    pub(crate) fn match_photo_under(&mut self, ui: &egui::Ui, rect: egui::Rect) -> bool {
        let Some((photo, placement)) = self.match_photo_current() else {
            return true;
        };
        let (show_model, behind) = (photo.show_model, photo.background);
        if behind || !show_model {
            let (photo, placement) = (photo.clone(), placement);
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
            self.paint_photo(ui.ctx(), &painter, rect, &photo, &placement);
        }
        show_model
    }

    /// Paints the photo in front of the model (Background unchecked) and, while
    /// editing, the grid, bars, horizon, axes and grips.
    pub(crate) fn match_photo_over(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let Some((photo, placement)) = self.match_photo_current() else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        if !photo.background && photo.show_model {
            let photo = photo.clone();
            self.paint_photo(ui.ctx(), &painter, rect, &photo, &placement);
        }
        let Some(edit) = &self.match_photo.edit else {
            return;
        };
        let ppp = ui.ctx().pixels_per_point();
        let hover = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| rect.contains(*p))
            .map(|p| {
                DVec2::new(f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y))
                    * f64::from(ppp)
            });
        paint_edit(&painter, rect, ppp, edit, &placement, hover);
    }

    fn paint_photo(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        rect: egui::Rect,
        photo: &MatchedPhoto,
        placement: &Placement,
    ) {
        let fresh =
            !matches!(&self.match_photo.texture, Some((d, _)) if Arc::ptr_eq(d, &photo.data));
        if fresh {
            self.match_photo.texture = decode(&photo.data).map(|img| {
                // Large photos are scaled down for the texture (it covers the screen).
                let img = if img.width().max(img.height()) > 4096 {
                    image::imageops::thumbnail(
                        &img,
                        4096 * img.width() / img.width().max(img.height()),
                        4096 * img.height() / img.width().max(img.height()),
                    )
                } else {
                    img
                };
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                (
                    photo.data.clone(),
                    ctx.load_texture("matched photo", color, egui::TextureOptions::LINEAR),
                )
            });
        }
        let Some((_, tex)) = &self.match_photo.texture else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        let size = photo.bars.size;
        let corners = [
            DVec2::ZERO,
            DVec2::new(size.x, 0.0),
            size,
            DVec2::new(0.0, size.y),
        ];
        let uvs = [
            egui::pos2(0.0, 0.0),
            egui::pos2(1.0, 0.0),
            egui::pos2(1.0, 1.0),
            egui::pos2(0.0, 1.0),
        ];
        let tint = egui::Color32::WHITE.gamma_multiply(photo.opacity.clamp(0.0, 1.0));
        let mut mesh = egui::Mesh::with_texture(tex.id());
        for (c, uv) in corners.iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: to_screen(rect, ppp, placement.to_view(*c)),
                uv,
                color: tint,
            });
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        painter.add(egui::Shape::mesh(mesh));
    }

    /// The Match Photo panel while editing.
    pub(crate) fn match_photo_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut edit) = self.match_photo.edit.take() else {
            return;
        };
        let units = self
            .document
            .as_ref()
            .map_or(mt_infer::units::LengthUnit::Inch, |d| d.units);
        let id = egui::Id::new(("mt_dialog", "match_photo")).with("pos");
        if ctx.data(|d| d.get_temp::<egui::Pos2>(id)).is_none() {
            let screen = ctx.content_rect();
            ctx.data_mut(|d| {
                d.insert_temp(id, egui::pos2(screen.max.x - 300.0, screen.min.y + 120.0))
            });
        }
        let mut action = None;
        let r = mt_ui::Dialog::new("match_photo", "Match Photo")
            .modal(false)
            .width(250.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                let p = &mut edit.photo;
                ui.label(&p.filename);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.show_model, "Model");
                    ui.checkbox(&mut p.background, "Background");
                });
                ui.horizontal(|ui| {
                    ui.label("Opacity");
                    ui.add(egui::Slider::new(&mut p.opacity, 0.0..=1.0).show_value(false));
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.show_grid, "Grid");
                });
                ui.horizontal(|ui| {
                    ui.label("Style");
                    for s in GridStyle::ALL {
                        if ui.selectable_label(p.style == s, s.label()).clicked() && p.style != s {
                            p.style = s;
                            p.bars = photo_match::default_bars(p.bars.size, s);
                            action = Some("resolve");
                        }
                    }
                });
                ui.label("Planes");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.planes[0], "Red/Green");
                    ui.checkbox(&mut p.planes[1], "Red/Blue");
                    ui.checkbox(&mut p.planes[2], "Green/Blue");
                });
                ui.horizontal(|ui| {
                    ui.label("Spacing");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut edit.spacing_text).desired_width(90.0),
                    );
                    if r.lost_focus()
                        && let Some(v) = mt_infer::units::parse_length(&edit.spacing_text, units)
                        && v > 0.0
                    {
                        edit.photo.spacing = v;
                        edit.spacing_text = units.format(v);
                    }
                });
                ui.add_space(4.0);
                match &edit.solved {
                    Ok(cam) => {
                        ui.weak(format!("Field of view {:.1}°", cam.fov_y_deg()));
                    }
                    Err(e) => {
                        ui.colored_label(egui::Color32::from_rgb(180, 0, 0), e.to_string());
                    }
                }
                ui.add_space(4.0);
                if ui
                    .add_enabled(
                        edit.solved.is_ok(),
                        egui::Button::new("Project Textures From Photo"),
                    )
                    .clicked()
                {
                    action = Some("project");
                }
                ui.add_space(6.0);
                match mt_ui::dialog::buttons(ui, &["Done", "Cancel"], Some(0)) {
                    Some(0) => action = Some("done"),
                    Some(_) => action = Some("cancel"),
                    None => {}
                }
            });
        if r.closed {
            action = Some("cancel");
        }
        match action {
            Some("resolve") => {
                edit.resolve();
                self.match_photo.edit = Some(edit);
            }
            Some("project") => {
                if let Ok(cam) = edit.solved {
                    self.project_photo_textures(frame, &edit.photo, &cam);
                }
                self.match_photo.edit = Some(edit);
            }
            Some("done") => self.match_photo_done(edit),
            Some("cancel") => self.match_photo_cancel(edit),
            _ => self.match_photo.edit = Some(edit),
        }
    }

    /// Done: the scene keeps the photo and the matched camera.
    fn match_photo_done(&mut self, edit: Edit) {
        let cam = match edit.solved {
            Ok(c) => c,
            Err(e) => {
                self.message = Some(e.to_string());
                self.match_photo.edit = Some(edit);
                return;
            }
        };
        let (camera, _) = view_of(&cam, self.viewport_px);
        log::info!(
            "matched photo: eye {:.3} forward {:.5} focal {:.2} px (vertical field of view {:.3} deg)",
            cam.eye(),
            cam.forward(),
            cam.focal,
            cam.fov_y_deg()
        );
        self.camera = camera.clone();
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        let page = edit.page;
        let photo = edit.photo;
        doc.edit_model(|m, _| {
            let Some(p) = m.pages.get_mut(page) else {
                return false;
            };
            p.eye = camera.eye;
            p.target = camera.target;
            p.fov_deg = camera.fov_deg;
            p.perspective = true;
            p.matched_photo = Some(Box::new(photo));
            true
        });
        self.active_scene = Some(page);
        self.show_matched_photo(page);
    }

    /// Cancel: the photo goes back to how it was (a new one is removed with its scene).
    fn match_photo_cancel(&mut self, edit: Edit) {
        if edit.new {
            if let Some(doc) = self.document.as_mut() {
                let page = edit.page;
                let data = edit.photo.data.clone();
                doc.edit_model(|m, _| {
                    let ours = m
                        .pages
                        .get(page)
                        .and_then(|p| p.matched_photo.as_deref())
                        .is_some_and(|p| Arc::ptr_eq(&p.data, &data));
                    if ours {
                        m.pages.remove(page);
                    }
                    ours
                });
                self.active_scene = None;
            }
            self.camera = edit.camera_before;
        } else {
            // The scene still holds the photo as it was.
            self.show_matched_photo(edit.page);
        }
    }

    /// Project Textures From Photo: the selected faces of the open group or component
    /// (all its faces when none is selected) get a new material of the photo, mapped
    /// as the camera saw it, on the side facing the camera. SketchUp asks whether to
    /// replace existing materials; here they are replaced.
    fn project_photo_textures(
        &mut self,
        frame: &eframe::Frame,
        photo: &MatchedPhoto,
        cam: &PhotoCamera,
    ) {
        let selected: Vec<_> = self
            .selection
            .items()
            .iter()
            .filter_map(|o| match o {
                crate::document::Owner::Face(f) => Some(*f),
                _ => None,
            })
            .collect();
        let xf = self.ctx_xf;
        let color = decode(&photo.data).map_or([128; 3], |i| {
            crate::image_entity::average_color(&image::DynamicImage::ImageRgba8(i).to_rgb8())
        });
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        let eye = xf.inverse().transform_point3(cam.eye());
        let mut count = 0;
        doc.edit_model(|m, ents| {
            let faces: Vec<_> = if selected.is_empty() {
                ents.faces.keys().collect()
            } else {
                selected.clone()
            };
            let name = {
                let base = photo
                    .filename
                    .rsplit_once('.')
                    .map_or(photo.filename.as_str(), |(s, _)| s);
                let mut name = base.to_owned();
                let mut k = 1;
                while m.materials.iter().any(|x| x.name == name) {
                    k += 1;
                    name = format!("{base}{k}");
                }
                name
            };
            m.materials.push(mt_kernel::Material {
                name,
                color,
                opacity: 1.0,
                texture: Some(mt_kernel::Texture {
                    filename: photo.filename.clone(),
                    data: photo.data.clone(),
                    width: 1.0,
                    height: 1.0,
                    colorize: false,
                }),
            });
            let mat = m.materials.len() - 1;
            for f in faces {
                let Some(face) = ents.faces.get(f) else {
                    continue;
                };
                let Some(&on) = ents.loop_points(&face.outer).first() else {
                    continue;
                };
                let normal = face.normal;
                let Some(uv) = cam.face_uv(xf, normal, on) else {
                    continue;
                };
                let back = normal.dot(eye - on) < 0.0;
                let Some(face) = ents.faces.get_mut(f) else {
                    continue;
                };
                let old = face.attrs.uv.as_deref().cloned().unwrap_or_default();
                if back {
                    face.attrs.back_material = Some(mat);
                    face.attrs.uv = mt_kernel::FaceUv::boxed(old.front, Some(uv));
                } else {
                    face.attrs.material = Some(mat);
                    face.attrs.uv = mt_kernel::FaceUv::boxed(Some(uv), old.back);
                }
                count += 1;
            }
            if count == 0 {
                m.materials.pop();
            }
            count > 0
        });
        self.message = Some(format!("Projected the photo onto {count} faces."));
        if count > 0
            && let Some(rs) = frame.wgpu_render_state()
        {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
    }
}

/// GUI-check hook: when `MESHTARD_TEST_PHOTO_BARS` names a file of 21 numbers (red
/// bars 1 and 2, green bars 1 and 2 as x y pairs of their ends, the origin, the scale
/// point and its height in inches), Match New Photo starts with those bars instead of
/// the defaults (dragging eight grips exactly is impractical from a test script).
fn test_bars(size: DVec2) -> Option<photo_match::PhotoBars> {
    let path = std::env::var_os("MESHTARD_TEST_PHOTO_BARS")?;
    let text = std::fs::read_to_string(path).ok()?;
    let v: Vec<f64> = text
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if v.len() < 21 {
        return None;
    }
    let p = |i: usize| DVec2::new(v[i], v[i + 1]);
    Some(photo_match::PhotoBars {
        size,
        red: [[p(0), p(2)], [p(4), p(6)]],
        green: [[p(8), p(10)], [p(12), p(14)]],
        origin: p(16),
        scale: p(18),
        scale_height: v[20],
        inside: false,
    })
}

/// Logical screen point of a viewport pixel.
fn to_screen(rect: egui::Rect, ppp: f32, q: DVec2) -> egui::Pos2 {
    rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp
}

/// The grip under viewport pixel `at` (bar ends first, then the origin, the scale
/// grip and the horizon).
fn grip_at(edit: &Edit, placement: &Placement, at: DVec2, radius: f64) -> Option<Handle> {
    let b = &edit.photo.bars;
    let near = |p: DVec2| placement.to_view(p).distance(at) <= radius;
    if let Some(i) = b.ends().iter().position(|p| near(*p)) {
        return Some(Handle::End(i));
    }
    if near(b.origin) {
        return Some(Handle::Origin);
    }
    if near(b.scale) {
        return Some(Handle::Scale);
    }
    let cam = edit.solved.as_ref().ok()?;
    let (p, dir) = cam.horizon()?;
    let (a, c) = (placement.to_view(p), placement.to_view(p + dir));
    let d = (c - a).normalize();
    ((at - a).perp_dot(d).abs() <= radius).then(|| Handle::Horizon(placement.to_photo(at)))
}

/// Drags `handle` to photo point `p`.
fn drag_grip(edit: &mut Edit, handle: Handle, p: DVec2) {
    let bars = &mut edit.photo.bars;
    match handle {
        Handle::End(i) => {
            *bars.end_mut(i) = p;
            edit.resolve();
        }
        Handle::Origin => {
            match &edit.solved {
                Ok(cam) => bars.move_origin(p, cam),
                Err(_) => bars.origin = p,
            }
            edit.resolve();
        }
        Handle::Scale => {
            // Along the blue axis only; a drag that has no solution is ignored.
            let Ok(cam) = &edit.solved else { return };
            let o = bars.origin;
            let Some(up) = cam.project(DVec3::Z * bars.scale_height) else {
                return;
            };
            let dir = (up - o).normalize_or_zero();
            let mut moved = *bars;
            moved.scale = o + dir * (p - o).dot(dir).max(1.0);
            if let Ok(c) = photo_match::solve(&moved) {
                *bars = moved;
                edit.solved = Ok(c);
            }
        }
        Handle::Horizon(last) => {
            let Ok(cam) = &edit.solved else { return };
            let Some((_, dir)) = cam.horizon() else {
                return;
            };
            let n = dir.perp();
            bars.shift_bars(n * (p - last).dot(n));
            edit.drag = Some(Handle::Horizon(p));
            edit.resolve();
        }
    }
}

/// Paints the Match Photo grips, bars, horizon, axes and grid.
fn paint_edit(
    painter: &egui::Painter,
    rect: egui::Rect,
    ppp: f32,
    edit: &Edit,
    placement: &Placement,
    hover: Option<DVec2>,
) {
    let scr = |p: DVec2| to_screen(rect, ppp, placement.to_view(p));
    let b = &edit.photo.bars;
    let line = |a: DVec2, c: DVec2, w: f32, color: egui::Color32| {
        painter.line_segment([scr(a), scr(c)], egui::Stroke::new(w, color));
    };
    if let Ok(cam) = &edit.solved {
        let p = &edit.photo;
        if p.show_grid {
            let axes = [DVec3::X, DVec3::Y, DVec3::Z];
            let colors = [RED, GREEN, BLUE];
            let s = p.spacing.max(1e-3);
            let extent = s * GRID_LINES as f64;
            for (k, (i, j)) in [(0, 1), (0, 2), (1, 2)].into_iter().enumerate() {
                if !p.planes[k] {
                    continue;
                }
                for n in 0..=GRID_LINES {
                    let t = s * n as f64;
                    for (a, c) in [(i, j), (j, i)] {
                        let from = axes[c] * t;
                        let to = from + axes[a] * extent;
                        if let Some((u, v)) = cam.project_segment(from, to) {
                            line(u, v, 1.0, colors[a].gamma_multiply(0.6));
                        }
                    }
                }
            }
        }
        // The axes from the origin (far along each, towards its vanishing point).
        let far = cam.depth().abs().max(1.0) * 1e3;
        for (axis, color) in [(DVec3::X, RED), (DVec3::Y, GREEN), (DVec3::Z, BLUE)] {
            if let Some((u, v)) = cam.project_segment(DVec3::ZERO, axis * far) {
                line(u, v, 1.5, color);
            }
        }
        if let Some((h, dir)) = cam.horizon() {
            let span = (b.size.x + b.size.y) * 50.0;
            line(h - dir * span, h + dir * span, 2.0, YELLOW);
        }
        // Dashed lines from the bars to their vanishing points.
        for (bars, axis, color) in [(&b.red, DVec3::X, RED), (&b.green, DVec3::Y, GREEN)] {
            let Some(vp) = cam.vanishing_point(axis) else {
                continue;
            };
            for [a, c] in bars {
                let near = if a.distance(vp) < c.distance(vp) {
                    *a
                } else {
                    *c
                };
                painter.add(egui::Shape::dashed_line(
                    &[scr(near), scr(vp)],
                    egui::Stroke::new(1.0, color),
                    6.0,
                    4.0,
                ));
            }
        }
    }
    for (bars, color) in [(&b.red, RED), (&b.green, GREEN)] {
        for [a, c] in bars {
            line(*a, *c, 2.5, color);
        }
    }
    let hot =
        |p: DVec2| hover.is_some_and(|h| placement.to_view(p).distance(h) <= f64::from(GRIP * ppp));
    let grip = |p: DVec2, color: egui::Color32| {
        let size = if hot(p) { GRIP * 1.4 } else { GRIP };
        let r = egui::Rect::from_center_size(scr(p), egui::vec2(size, size));
        painter.rect_filled(r, 0.0, color);
        painter.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
            egui::StrokeKind::Middle,
        );
    };
    for (i, p) in b.ends().into_iter().enumerate() {
        grip(p, if i < 4 { RED } else { GREEN });
    }
    grip(b.scale, BLUE);
    let o = scr(b.origin);
    painter.circle(
        o,
        GRIP * 0.7,
        egui::Color32::YELLOW,
        egui::Stroke::new(1.5, egui::Color32::BLACK),
    );
    if let Err(e) = &edit.solved {
        let pos = rect.left_top() + egui::vec2(12.0, 12.0);
        let galley = painter.layout_no_wrap(
            e.to_string(),
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(pos, galley.size()).expand(4.0),
            3.0,
            egui::Color32::from_rgb(170, 0, 0),
        );
        painter.galley(pos, galley, egui::Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_camera_and_placed_photo_agree() {
        let size = DVec2::new(1600.0, 1200.0);
        for (roll, viewport) in [
            (0.0, DVec2::new(1200.0, 800.0)),
            (7.0, DVec2::new(1000.0, 1000.0)),
            (-11.0, DVec2::new(640.0, 900.0)),
        ] {
            let cam = PhotoCamera::look_at(
                DVec3::new(-600.0, -450.0, 240.0),
                DVec3::new(120.0, 80.0, 60.0),
                roll,
                1500.0,
                size,
            );
            let (camera, placement) = view_of(&cam, viewport);
            for p in [
                DVec3::ZERO,
                DVec3::new(300.0, 0.0, 0.0),
                DVec3::new(0.0, 250.0, 100.0),
                DVec3::new(150.0, 150.0, 300.0),
            ] {
                let on_screen = camera.project(p, viewport).unwrap();
                let via_photo = placement.to_view(cam.project(p).unwrap());
                assert!(
                    on_screen.distance(via_photo) < 1e-6,
                    "{on_screen} vs {via_photo}"
                );
                let back = placement.to_photo(via_photo);
                assert!(back.distance(cam.project(p).unwrap()) < 1e-6);
            }
        }
    }

    #[test]
    fn solved_defaults_round_trip_through_the_app_camera() {
        let size = DVec2::new(1200.0, 900.0);
        let bars = photo_match::default_bars(size, GridStyle::Above);
        let cam = photo_match::solve(&bars).unwrap();
        let (camera, placement) = view_of(&cam, DVec2::new(1000.0, 700.0));
        let o = camera
            .project(DVec3::ZERO, DVec2::new(1000.0, 700.0))
            .unwrap();
        assert!(placement.to_view(bars.origin).distance(o) < 1e-6);
    }
}
