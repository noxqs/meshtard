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
//!
//! The help article (search summaries again) [doc]: "Option (Mac) or Control
//! (Windows) + Arrow keys" navigate "within a group of photos taken from nearly the
//! same location", for example Trimble Vision imagery from one station; "Shift +
//! Arrow keys" select "the next scene in the photo series". The key is the
//! [`Role::Copy`] key of `tool_mods` (Ctrl on Windows and Linux, Option on the Mac).
//! Meshtard ([`hop`]): Ctrl / Option + arrows go among the photos whose camera is
//! within [`SAME_SPOT`] of the highlighted one's distance to the model origin
//! ([`same_spot`]): Left / Right turn to the photo with the next smaller turn that
//! way (wrapping round a panorama), Up / Down to the nearest photo looking higher /
//! lower. Shift + arrows step through a series ([`series_step`]): the photo scenes
//! whose names agree once a trailing number (and file extension) is dropped, in
//! scene order, Right / Down forward and Left / Up back, wrapping; a photo in no
//! such series steps through all photo scenes. What makes a series in SketchUp, the
//! same-spot tolerance and the turn order are guesses (unverified). Other modifier
//! combinations do nothing.
//!
//! Matched photos are not saved in `.skp`, so a reopened file has no igloo: the
//! v17 writer (`mt_kernel::export`) writes no scenes at all, and the reader decodes
//! only `CViewPage` records whose capture flags are the baseline or hidden tags
//! (`mt_skp::legacy`, `PAGE_ALLOWED_FLAGS`); where a page keeps its photo and the
//! solved camera is not known (`spec/parity/LEFTOVERS.md` item 1). Frame distance,
//! pull-back, looks and the arrow rule are guesses (unverified).

use std::sync::Arc;

use eframe::egui;
use glam::{DVec2, DVec3};
use mt_kernel::photo_match::{self, PhotoCamera};
use mt_render::Camera;

use crate::App;
use crate::tool_mods::{Keys, Platform, Role};

/// Where a photo's frame is drawn: this fraction of its camera's distance to the
/// model origin, in front of the camera.
pub(crate) const FRAME_AT: f64 = 0.3;
/// The view on a photo is pulled back behind its camera by this fraction of that
/// distance ("zooming out a bit").
pub(crate) const BACK: f64 = 0.5;
/// Photos whose cameras are closer than this fraction of the highlighted camera's
/// distance to the model origin count as taken from the same spot (unverified).
pub(crate) const SAME_SPOT: f64 = 0.05;
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

/// What an arrow key does in the igloo, by the modifiers held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hop {
    /// No modifier: the nearest photo on that side of the screen ([`neighbor`]).
    Near,
    /// Ctrl / Option: among the photos from the same spot ([`same_spot`]).
    SameSpot,
    /// Shift: the next scene of the series ([`series_step`]).
    Series,
}

/// The arrow keys' meaning with `keys` held on `platform`; `None` for other
/// combinations (they do nothing).
pub(crate) fn hop(platform: Platform, keys: Keys) -> Option<Hop> {
    let held = [keys.ctrl, keys.alt, keys.cmd, keys.shift];
    match held.iter().filter(|k| **k).count() {
        0 => Some(Hop::Near),
        1 if keys.shift => Some(Hop::Series),
        1 if platform.held(keys, Role::Copy) => Some(Hop::SameSpot),
        _ => None,
    }
}

/// Heading of a view direction about the blue axis (radians, counterclockwise from
/// red seen from above) and its pitch above the horizon.
fn heading_pitch(f: DVec3) -> (f64, f64) {
    (f.y.atan2(f.x), f.z.clamp(-1.0, 1.0).asin())
}

/// The photo after `from` among those taken from the same spot, in screen
/// direction `dir` (x right, y down): Left / Right the smallest turn that way about
/// the blue axis (wrapping, so a panorama goes round), Up / Down the photo nearest in
/// direction among those looking higher / lower.
pub(crate) fn same_spot(cams: &[PhotoCamera], from: usize, dir: DVec2) -> Option<usize> {
    use std::f64::consts::TAU;
    let here = cams.get(from)?;
    let near = SAME_SPOT * reach(here);
    let (h0, p0) = heading_pitch(here.forward());
    let group = (cams.iter().enumerate())
        .filter(move |(i, c)| *i != from && c.eye().distance(here.eye()) <= near);
    const EPS: f64 = 1e-6;
    if dir.x.abs() >= dir.y.abs() {
        // Turning right is clockwise seen from above.
        let sign = if dir.x > 0.0 { -1.0 } else { 1.0 };
        // The smallest turn; among photos turned as far, the one pitched nearest.
        group
            .filter_map(|(i, c)| {
                let (h, p) = heading_pitch(c.forward());
                let turn = (sign * (h - h0)).rem_euclid(TAU);
                (turn > EPS && turn < TAU - EPS).then_some((i, turn, (p - p0).abs()))
            })
            .min_by(|a, b| {
                if (a.1 - b.1).abs() < EPS {
                    a.2.total_cmp(&b.2)
                } else {
                    a.1.total_cmp(&b.1)
                }
            })
            .map(|(i, ..)| i)
    } else {
        let up = dir.y < 0.0;
        group
            .filter(|(_, c)| {
                let p = heading_pitch(c.forward()).1;
                if up { p > p0 + EPS } else { p < p0 - EPS }
            })
            .map(|(i, c)| (i, c.forward().angle_between(here.forward())))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

/// A scene name's series: the name without a file extension, a trailing number and
/// the separators before it ("Station 2 - 07.jpg" and "Station 2 - 08.jpg" are
/// both "Station 2").
pub(crate) fn series_of(name: &str) -> &str {
    let name = match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && (1..=4).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            stem
        }
        _ => name,
    };
    name.trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches(|c: char| c.is_whitespace() || "-_.#(".contains(c))
}

/// The next (`forward`) or previous photo of `from`'s series, given the photos'
/// scene names in scene order, wrapping; a photo alone in its series steps through
/// all of them.
pub(crate) fn series_step(names: &[&str], from: usize, forward: bool) -> Option<usize> {
    let key = series_of(names.get(from)?);
    let mut group: Vec<usize> = (0..names.len())
        .filter(|i| series_of(names[*i]) == key)
        .collect();
    if group.len() < 2 {
        group = (0..names.len()).collect();
    }
    if group.len() < 2 {
        return None;
    }
    let at = group.iter().position(|i| *i == from)?;
    let n = group.len();
    Some(
        group[if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        }],
    )
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
        self.message = Some(format!(
            "Image Igloo: orbit to a photo, Enter or double-click opens its scene; \
             arrows go to the next photo ({} + arrows: same spot, Shift + arrows: \
             series); Esc leaves.",
            Platform::current().key(Role::Copy).name()
        ));
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
        let (escape, enter, arrow) = ui.input_mut(|i| {
            let mut k = |key| i.consume_key(egui::Modifiers::NONE, key);
            let (escape, enter) = (k(egui::Key::Escape), k(egui::Key::Enter));
            // The first arrow press with any modifiers; every arrow press is taken.
            let mut arrow = None;
            i.events.retain(|e| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                else {
                    return true;
                };
                let dir = match key {
                    egui::Key::ArrowLeft => DVec2::NEG_X,
                    egui::Key::ArrowRight => DVec2::X,
                    egui::Key::ArrowUp => DVec2::NEG_Y,
                    egui::Key::ArrowDown => DVec2::Y,
                    _ => return true,
                };
                arrow.get_or_insert((dir, *modifiers));
                false
            });
            (escape, enter, arrow)
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
        let next = arrow.and_then(
            |(dir, m)| match hop(Platform::current(), Keys::from_egui(m))? {
                Hop::Near => neighbor(&self.camera, &cams, igloo.hot, dir, viewport_px),
                Hop::SameSpot => same_spot(&cams, igloo.hot, dir),
                Hop::Series => {
                    let doc = self.document.as_ref()?;
                    let names: Vec<&str> = (igloo.photos.iter())
                        .map(|p| doc.model.pages[p.page].name.as_str())
                        .collect();
                    series_step(&names, igloo.hot, dir.x + dir.y > 0.0)
                }
            },
        );
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
            p.texture = mt_kernel::images::decode(&p.data).ok().map(|img| {
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

    /// A camera at `eye` looking along heading `deg` (degrees from red,
    /// counterclockwise) and pitch `pitch` (degrees up).
    fn looking(eye: DVec3, deg: f64, pitch: f64) -> PhotoCamera {
        let (h, p) = (deg.to_radians(), pitch.to_radians());
        let f = DVec3::new(h.cos() * p.cos(), h.sin() * p.cos(), p.sin());
        let right = f.cross(DVec3::Z).normalize();
        let down = f.cross(right);
        let rotation = DMat3::from_cols(right, down, f).transpose();
        PhotoCamera {
            rotation,
            translation: -(rotation * eye),
            focal: 1000.0,
            principal: DVec2::new(500.0, 375.0),
            size: DVec2::new(1000.0, 750.0),
        }
    }

    const NO: Keys = Keys {
        ctrl: false,
        alt: false,
        cmd: false,
        shift: false,
    };

    #[test]
    fn modifiers_choose_the_hop() {
        let ctrl = Keys { ctrl: true, ..NO };
        let alt = Keys { alt: true, ..NO };
        let cmd = Keys { cmd: true, ..NO };
        let shift = Keys { shift: true, ..NO };
        for p in [Platform::Pc, Platform::Mac] {
            assert_eq!(hop(p, NO), Some(Hop::Near));
            assert_eq!(hop(p, shift), Some(Hop::Series));
            assert_eq!(
                hop(
                    p,
                    Keys {
                        ctrl: true,
                        ..shift
                    }
                ),
                None
            );
            assert_eq!(hop(p, cmd), None);
        }
        // Control on Windows and Linux, Option on the Mac (help article).
        assert_eq!(hop(Platform::Pc, ctrl), Some(Hop::SameSpot));
        assert_eq!(hop(Platform::Pc, alt), None);
        assert_eq!(hop(Platform::Mac, alt), Some(Hop::SameSpot));
        assert_eq!(hop(Platform::Mac, ctrl), None);
    }

    #[test]
    fn same_spot_turns_round_a_station() {
        // A station of four photos a quarter turn apart, one looking up, and a photo
        // from elsewhere looking the same way as the first.
        let at = DVec3::new(300.0, 0.0, 60.0);
        let mut cams: Vec<PhotoCamera> = [180.0, 90.0, 0.0, 270.0]
            .map(|h| looking(at, h, 0.0))
            .to_vec();
        cams.push(looking(at + DVec3::new(1.0, 2.0, 0.0), 180.0, 30.0)); // 4: up
        cams.push(looking(DVec3::new(600.0, 0.0, 60.0), 180.0, 0.0)); // 5: far
        // Right turns clockwise seen from above: 180 -> 90 -> 0 -> 270 -> 180.
        assert_eq!(same_spot(&cams, 0, DVec2::X), Some(1));
        assert_eq!(same_spot(&cams, 1, DVec2::X), Some(2));
        assert_eq!(same_spot(&cams, 2, DVec2::X), Some(3));
        assert_eq!(same_spot(&cams, 3, DVec2::X), Some(0));
        assert_eq!(same_spot(&cams, 0, DVec2::NEG_X), Some(3));
        assert_eq!(same_spot(&cams, 2, DVec2::NEG_X), Some(1));
        // Up goes to the photo looking higher, and back down.
        assert_eq!(same_spot(&cams, 0, DVec2::NEG_Y), Some(4));
        assert_eq!(same_spot(&cams, 4, DVec2::Y), Some(0));
        assert_eq!(same_spot(&cams, 1, DVec2::Y), None);
        // The far photo has no other photo from its spot.
        for d in [DVec2::X, DVec2::NEG_X, DVec2::Y, DVec2::NEG_Y] {
            assert_eq!(same_spot(&cams, 5, d), None);
        }
        // The plain arrows' ring has no two photos from one spot.
        assert_eq!(same_spot(&ring(), 0, DVec2::X), None);
    }

    #[test]
    fn series_names() {
        assert_eq!(series_of("Station 2 - 07.jpg"), "Station 2");
        assert_eq!(series_of("IMG_0012.JPG"), "IMG");
        assert_eq!(series_of("Scene 3"), "Scene");
        assert_eq!(series_of("Porch"), "Porch");
        assert_eq!(series_of("v1.2"), "v1");
    }

    #[test]
    fn shift_arrows_step_through_the_series() {
        let names = [
            "A-1.jpg", "B 1.png", "A-2.jpg", "Lone", "A-3.jpg", "B 2.png",
        ];
        // Forward and back within "A", wrapping, skipping other scenes.
        assert_eq!(series_step(&names, 0, true), Some(2));
        assert_eq!(series_step(&names, 2, true), Some(4));
        assert_eq!(series_step(&names, 4, true), Some(0));
        assert_eq!(series_step(&names, 0, false), Some(4));
        assert_eq!(series_step(&names, 1, true), Some(5));
        assert_eq!(series_step(&names, 5, true), Some(1));
        // A photo in no series steps through all photo scenes.
        assert_eq!(series_step(&names, 3, true), Some(4));
        assert_eq!(series_step(&names, 3, false), Some(2));
        assert_eq!(series_step(&["Only"], 0, true), None);
        assert_eq!(series_step(&names, 9, true), None);
    }

    #[test]
    fn triangle_test() {
        let (a, b, c) = (DVec2::ZERO, DVec2::new(10.0, 0.0), DVec2::new(0.0, 10.0));
        assert!(in_triangle(DVec2::new(2.0, 2.0), a, b, c));
        assert!(in_triangle(DVec2::new(2.0, 2.0), a, c, b));
        assert!(!in_triangle(DVec2::new(8.0, 8.0), a, b, c));
    }
}
