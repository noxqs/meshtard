//! Camera > Image Igloo (I, FEATURES.md 73): the matched photos of the model's scenes
//! shown together, each where its camera took it, so you orbit among them and open
//! one's scene.
//!
//! After SketchUp's help article "Image Igloo View: Navigating between scenes" (read
//! through search summaries; the page is not on help.sketchup.com's English sitemap
//! any more) [doc]: Camera > Image Igloo needs two or more scenes with a matched photo
//! (otherwise it is grayed out); you orbit to highlight a photo, Enter or a double
//! click opens its scene; arrow keys go to photos with a similar focal point. The
//! community describes it as an "igloo" of the photos over the model (SketchUcation)
//! and SketchUp 2026 as "zooming out a bit" from the photo (forum t/342096).
//!
//! Meshtard: every photo is drawn as a frame in its camera's view, at
//! [`FRAME_AT`] of the way from the camera to the model origin; the view starts
//! behind the photo closest to the current view direction and pulled back by
//! [`BACK`]; a left drag orbits (the middle button and wheel as always); the
//! highlighted photo is the one nearest the view's center; arrows go to the nearest
//! photo in that screen direction; Enter or a double click on a frame opens the
//! scene (`scene_activate`, which shows the photo); Esc or the command again leaves.
//! Not done: Ctrl / Option + arrows (photos from nearly the same spot) and Shift +
//! arrows (the next scene in a series). Frame distance, pull-back, looks and the
//! arrow rule are guesses (unverified).

use std::sync::Arc;

use eframe::egui;
use glam::{DVec2, DVec3};
use mt_kernel::photo_match::{self, PhotoCamera};
use mt_render::Camera;

use crate::App;

/// Where a photo's frame is drawn: this fraction of its camera's distance to the
/// model origin, in front of the camera.
pub(crate) const FRAME_AT: f64 = 0.3;
/// The view on a photo is pulled back behind its camera by this fraction of that
/// distance ("zooming out a bit").
pub(crate) const BACK: f64 = 0.5;
/// Longest side of a photo's texture, in pixels.
const THUMB: u32 = 512;
/// Frames are drawn as this many cells per side (affine texture mapping per cell).
const CELLS: usize = 6;
const HOT: egui::Color32 = egui::Color32::from_rgb(240, 200, 0);

/// One scene's photo in the igloo.
pub(crate) struct Photo {
    pub page: usize,
    pub cam: PhotoCamera,
    data: Arc<[u8]>,
    texture: Option<egui::TextureHandle>,
    decoded: bool,
}

/// The Image Igloo while it is shown.
pub(crate) struct Igloo {
    pub photos: Vec<Photo>,
    /// The highlighted photo.
    pub hot: usize,
}

/// Distance from a photo's camera to the model origin (at least one inch).
fn reach(cam: &PhotoCamera) -> f64 {
    cam.depth().abs().max(1.0)
}

/// A photo's frame corners in world space (top left, top right, bottom right,
/// bottom left of the image).
pub(crate) fn frame_corners(cam: &PhotoCamera) -> [DVec3; 4] {
    let d = reach(cam) * FRAME_AT;
    let at = |p: DVec2| {
        let q = (p - cam.principal) * (d / cam.focal);
        cam.eye() + cam.forward() * d + cam.right() * q.x + cam.down() * q.y
    };
    let s = cam.size;
    [
        at(DVec2::ZERO),
        at(DVec2::new(s.x, 0.0)),
        at(s),
        at(DVec2::new(0.0, s.y)),
    ]
}

/// The igloo view on a photo: its camera's view, pulled back by [`BACK`].
pub(crate) fn view_on(cam: &PhotoCamera, viewport: DVec2) -> Camera {
    let mut c = crate::match_photo::view_of(cam, viewport).0;
    let back = cam.forward() * reach(cam) * BACK;
    c.eye -= back;
    c.target = cam.eye() + cam.forward() * reach(cam);
    c
}

/// The photo whose frame center is nearest the view's center, among those in front
/// of the camera and facing the same way.
pub(crate) fn nearest_center(
    camera: &Camera,
    cams: &[PhotoCamera],
    viewport: DVec2,
) -> Option<usize> {
    let mid = viewport / 2.0;
    (cams.iter().enumerate())
        .filter(|(_, c)| c.forward().dot(camera.forward()) > 0.0)
        .filter_map(|(i, c)| {
            let k = frame_corners(c);
            let center = (k[0] + k[2]) / 2.0;
            Some((i, camera.project(center, viewport)?.distance(mid)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The photo after `from` in screen direction `dir` (x right, y down): the nearest
/// one whose frame center lies within 60 degrees of that direction.
pub(crate) fn neighbor(
    camera: &Camera,
    cams: &[PhotoCamera],
    from: usize,
    dir: DVec2,
    viewport: DVec2,
) -> Option<usize> {
    let center = |c: &PhotoCamera| {
        let k = frame_corners(c);
        camera.project((k[0] + k[2]) / 2.0, viewport)
    };
    let start = center(cams.get(from)?)?;
    (cams.iter().enumerate())
        .filter(|(i, _)| *i != from)
        .filter_map(|(i, c)| {
            let v = center(c)? - start;
            let len = v.length();
            (len > 1e-9 && v.dot(dir) / len > 0.5).then_some((i, len))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The scenes with a matched photo whose camera solves, in scene order.
fn scene_photos(app: &App) -> Vec<Photo> {
    let Some(doc) = &app.document else {
        return Vec::new();
    };
    (doc.model.pages.iter().enumerate())
        .filter_map(|(page, p)| {
            let photo = p.matched_photo.as_deref()?;
            let cam = photo_match::solve(&photo.bars).ok()?;
            Some(Photo {
                page,
                cam,
                data: photo.data.clone(),
                texture: None,
                decoded: false,
            })
        })
        .collect()
}

impl App {
    /// Whether Camera > Image Igloo applies: two or more scenes with a matched photo.
    pub(crate) fn can_image_igloo(&self) -> bool {
        self.match_photo_editing_none()
            && self.document.as_ref().is_some_and(|d| {
                d.model
                    .pages
                    .iter()
                    .filter(|p| p.matched_photo.is_some())
                    .take(2)
                    .count()
                    == 2
            })
    }

    /// Camera > Image Igloo: shows the igloo, or leaves it when shown.
    pub(crate) fn image_igloo_command(&mut self) {
        if self.match_photo.igloo.take().is_some() {
            self.message = None;
            return;
        }
        let photos = scene_photos(self);
        if photos.len() < 2 {
            self.message =
                Some("Image Igloo needs two or more scenes with a matched photo.".into());
            return;
        }
        let cams: Vec<PhotoCamera> = photos.iter().map(|p| p.cam).collect();
        // Behind the photo closest to the current view direction.
        let fwd = self.camera.forward();
        let hot = (cams.iter().enumerate())
            .max_by(|a, b| a.1.forward().dot(fwd).total_cmp(&b.1.forward().dot(fwd)))
            .map_or(0, |(i, _)| i);
        self.fly_to(view_on(&cams[hot], self.viewport_px));
        self.match_photo.igloo = Some(Igloo { photos, hot });
        self.message = Some(
            "Image Igloo: orbit to a photo, Enter or double-click opens its scene; \
             arrows go to the next photo; Esc leaves."
                .into(),
        );
    }

    fn fly_to(&mut self, to: Camera) {
        self.match_photo_hide_shown();
        self.transition = Some(crate::tray::Transition::with_secs(
            self.camera.clone(),
            to,
            self.animation.transition_secs(),
        ));
    }

    /// Leaves the igloo (keeps the view).
    pub(crate) fn image_igloo_leave(&mut self) {
        if self.match_photo.igloo.take().is_some() {
            self.message = None;
        }
    }

    /// The igloo's mouse and keys; true while it takes the viewport's left button.
    pub(crate) fn image_igloo_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
    ) -> bool {
        let editing = !self.match_photo_editing_none();
        let Some(igloo) = self.match_photo.igloo.as_mut() else {
            return false;
        };
        // The scenes changed under it (deleted, undone): leave.
        let still = self.document.as_ref().is_some_and(|d| {
            igloo.photos.iter().all(|p| {
                d.model
                    .pages
                    .get(p.page)
                    .and_then(|x| x.matched_photo.as_deref())
                    .is_some_and(|m| Arc::ptr_eq(&m.data, &p.data))
            })
        });
        if !still || editing {
            self.image_igloo_leave();
            return false;
        }
        let cams: Vec<PhotoCamera> = igloo.photos.iter().map(|p| p.cam).collect();
        let ppp = f64::from(ui.ctx().pixels_per_point());
        let to_px = |p: egui::Pos2| {
            DVec2::new(f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y)) * ppp
        };
        let (escape, enter, arrows) = ui.input_mut(|i| {
            let mut k = |key| i.consume_key(egui::Modifiers::NONE, key);
            (
                k(egui::Key::Escape),
                k(egui::Key::Enter),
                [
                    (k(egui::Key::ArrowLeft), DVec2::NEG_X),
                    (k(egui::Key::ArrowRight), DVec2::X),
                    (k(egui::Key::ArrowUp), DVec2::NEG_Y),
                    (k(egui::Key::ArrowDown), DVec2::Y),
                ],
            )
        });
        if escape {
            self.image_igloo_leave();
            return true;
        }
        // A left drag orbits; the highlight follows the view's center.
        if response.dragged_by(egui::PointerButton::Primary) {
            let d = response.drag_delta();
            self.transition = None;
            self.camera
                .orbit(DVec2::new(f64::from(d.x), f64::from(d.y)) * ppp);
            if let Some(h) = nearest_center(&self.camera, &cams, viewport_px) {
                igloo.hot = h;
            }
        }
        let hit = response.interact_pointer_pos().and_then(|p| {
            let p = to_px(p);
            let vp = self.camera.view_proj(viewport_px.x / viewport_px.y);
            (0..cams.len()).rev().find(|i| {
                let k = frame_corners(&cams[*i]).map(|c| Camera::project_with(&vp, c, viewport_px));
                let [Some(a), Some(b), Some(c), Some(d)] = k else {
                    return false;
                };
                in_triangle(p, a, b, c) || in_triangle(p, a, c, d)
            })
        });
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(h) = hit
        {
            igloo.hot = h;
        }
        let open = if response.double_clicked_by(egui::PointerButton::Primary) {
            hit
        } else if enter {
            Some(igloo.hot)
        } else {
            None
        };
        if let Some(i) = open {
            let page = igloo.photos[i].page;
            self.image_igloo_leave();
            self.scene_activate(page);
            return true;
        }
        let next = (arrows.iter().find(|(pressed, _)| *pressed))
            .and_then(|(_, dir)| neighbor(&self.camera, &cams, igloo.hot, *dir, viewport_px));
        if let Some(n) = next {
            igloo.hot = n;
            self.fly_to(view_on(&cams[n], viewport_px));
        }
        true
    }

    /// Draws the photo frames over the model, the highlighted one outlined.
    pub(crate) fn image_igloo_paint(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let viewport = self.viewport_px;
        let Some(igloo) = self.match_photo.igloo.as_mut() else {
            return;
        };
        let ctx = ui.ctx();
        let ppp = ctx.pixels_per_point();
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let vp = self.camera.view_proj(viewport.x / viewport.y);
        let screen = |p: DVec2| rect.min + egui::vec2(p.x as f32, p.y as f32) / ppp;
        // Far photos first, so nearer frames cover them.
        let eye = self.camera.eye;
        let mut order: Vec<usize> = (0..igloo.photos.len()).collect();
        let dist = |p: &Photo| {
            let k = frame_corners(&p.cam);
            ((k[0] + k[2]) / 2.0).distance(eye)
        };
        order.sort_by(|a, b| dist(&igloo.photos[*b]).total_cmp(&dist(&igloo.photos[*a])));
        // One texture decoded per frame keeps the frame time down.
        if let Some(p) = igloo.photos.iter_mut().find(|p| !p.decoded) {
            p.decoded = true;
            p.texture = image::load_from_memory(&p.data).ok().map(|img| {
                let img = img.thumbnail(THUMB, THUMB).to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                ctx.load_texture("igloo photo", color, egui::TextureOptions::LINEAR)
            });
            ctx.request_repaint();
        }
        for i in order {
            let p = &igloo.photos[i];
            let hot = i == igloo.hot;
            let [a, b, c, d] = frame_corners(&p.cam);
            let at = |u: f64, v: f64| {
                let top = a.lerp(b, u);
                let bottom = d.lerp(c, u);
                Camera::project_with(&vp, top.lerp(bottom, v), viewport)
            };
            let tint = egui::Color32::WHITE.gamma_multiply(if hot { 0.95 } else { 0.6 });
            if let Some(tex) = &p.texture {
                let mut mesh = egui::Mesh::with_texture(tex.id());
                let n = CELLS;
                let mut ok = true;
                for y in 0..=n {
                    for x in 0..=n {
                        let (u, v) = (x as f64 / n as f64, y as f64 / n as f64);
                        let Some(q) = at(u, v) else {
                            ok = false;
                            break;
                        };
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: screen(q),
                            uv: egui::pos2(u as f32, v as f32),
                            color: tint,
                        });
                    }
                }
                if ok {
                    let w = (n + 1) as u32;
                    for y in 0..n as u32 {
                        for x in 0..n as u32 {
                            let k = y * w + x;
                            mesh.indices.extend_from_slice(&[
                                k,
                                k + 1,
                                k + w + 1,
                                k,
                                k + w + 1,
                                k + w,
                            ]);
                        }
                    }
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
            let outline: Option<Vec<egui::Pos2>> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
                .iter()
                .map(|(u, v)| at(*u, *v).map(screen))
                .collect();
            if let Some(pts) = outline {
                let stroke = if hot {
                    egui::Stroke::new(3.0, HOT)
                } else {
                    egui::Stroke::new(1.0, egui::Color32::from_gray(90))
                };
                painter.add(egui::Shape::closed_line(pts, stroke));
            }
        }
    }
}

/// Whether `p` lies in triangle `a`, `b`, `c` (either winding).
fn in_triangle(p: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    let s = |u: DVec2, v: DVec2, w: DVec2| (v - u).perp_dot(w - u);
    let (d1, d2, d3) = (s(a, b, p), s(b, c, p), s(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DMat3;

    /// A level camera at `eye` looking at the origin, 1000x750 pixels.
    fn cam(eye: DVec3) -> PhotoCamera {
        let fwd = (-eye).normalize();
        let right = fwd.cross(DVec3::Z).normalize();
        let down = fwd.cross(right);
        let rotation = DMat3::from_cols(right, down, fwd).transpose();
        PhotoCamera {
            rotation,
            translation: -(rotation * eye),
            focal: 1000.0,
            principal: DVec2::new(500.0, 375.0),
            size: DVec2::new(1000.0, 750.0),
        }
    }

    const VP: DVec2 = DVec2::new(1200.0, 800.0);

    fn ring() -> Vec<PhotoCamera> {
        // Four photos around the model, a quarter turn apart.
        [(400.0, 0.0), (0.0, 400.0), (-400.0, 0.0), (0.0, -400.0)]
            .map(|(x, y)| cam(DVec3::new(x, y, 60.0)))
            .to_vec()
    }

    #[test]
    fn frames_sit_in_front_of_their_cameras() {
        let c = cam(DVec3::new(400.0, 0.0, 60.0));
        assert!((c.eye() - DVec3::new(400.0, 0.0, 60.0)).length() < 1e-6);
        let k = frame_corners(&c);
        let center = (k[0] + k[2]) / 2.0;
        // On the axis, FRAME_AT of the way to the origin plane.
        let along = (center - c.eye()).dot(c.forward());
        assert!((along - c.depth() * FRAME_AT).abs() < 1e-6);
        // Top edge above the bottom one, left of the right one as seen.
        assert!(k[0].z > k[3].z);
        assert!((k[1] - k[0]).dot(c.right()) > 0.0);
        // The photo's own view shows the frame filling the photo's area.
        let (view, place) = crate::match_photo::view_of(&c, VP);
        let p = view.project(k[0], VP).unwrap();
        assert!(p.distance(place.to_view(DVec2::ZERO)) < 1e-3, "{p}");
    }

    #[test]
    fn the_view_on_a_photo_highlights_it() {
        let cams = ring();
        for (i, c) in cams.iter().enumerate() {
            let v = view_on(c, VP);
            // Pulled back behind the photo's camera, looking the same way.
            assert!((v.eye - c.eye()).dot(c.forward()) < 0.0);
            assert!(v.forward().dot(c.forward()) > 0.99);
            assert_eq!(nearest_center(&v, &cams, VP), Some(i));
        }
    }

    #[test]
    fn arrows_go_to_the_photo_on_that_side() {
        let cams = ring();
        let v = view_on(&cams[0], VP);
        // Looking from +X towards the origin, +Y is to the right.
        assert_eq!(neighbor(&v, &cams, 0, DVec2::X, VP), Some(1));
        assert_eq!(neighbor(&v, &cams, 0, DVec2::NEG_X, VP), Some(3));
        // The far photo is seen above the view's center, nothing below.
        assert_eq!(neighbor(&v, &cams, 0, DVec2::NEG_Y, VP), Some(2));
        assert_eq!(neighbor(&v, &cams, 0, DVec2::Y, VP), None);
    }

    #[test]
    fn triangle_test() {
        let (a, b, c) = (DVec2::ZERO, DVec2::new(10.0, 0.0), DVec2::new(0.0, 10.0));
        assert!(in_triangle(DVec2::new(2.0, 2.0), a, b, c));
        assert!(in_triangle(DVec2::new(2.0, 2.0), a, c, b));
        assert!(!in_triangle(DVec2::new(8.0, 8.0), a, b, c));
    }
}
