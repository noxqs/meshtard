//! Texture Position (command `tool.texture_position`: Tools menu, the Materials panel's
//! Position Texture button, and the face context menu's Texture > Position): the
//! texture of the selected face is drawn over it, translucent and extended to the
//! tiles around, with SketchUp's four pins. Dragging the red pin (or the texture
//! itself) moves it, green scales and rotates it about red, blue scales and shears it,
//! yellow distorts it (perspective). Free Pins moves each pin alone. Reset, Flip
//! Left/Right and Up/Down, and Rotate 90 / 180 / 270 act on the pins' tile. Enter, a
//! click outside the face or Done keeps the result as one undo step; Escape or Cancel
//! puts the texture back.
//!
//! Unverified against SketchUp 2026: the pins' look (plain colored disks here; SketchUp
//! draws pin icons with small action glyphs), where they first appear (here on the
//! tile corner nearest below-left of the face's center), the translucent texture's
//! extent (three by three tiles here), and the popup (a small toolbar at the top of
//! the viewport plus the same items on right-click). Projected textures are not
//! supported (`uv_projected` is not read), and a distorted texture is drawn with
//! per-vertex texture coordinates, so it is only exact at the face's vertices.

use eframe::egui;
use egui_wgpu::wgpu;
use glam::{DVec2, DVec3};
use mt_kernel::texture::{
    DEFAULT_UV, Pin, PinAction, drag_pin, pin_action, pins_from_uv, plane_point, uv_from_pins,
};
use mt_kernel::{Entities, FaceId, Material};

use crate::App;
use crate::document::Owner;
use crate::nav::Tool;

/// Pixels around a pin that grab it.
const PIN_GRAB_PX: f32 = 10.0;
/// Radius of a drawn pin, points.
const PIN_RADIUS: f32 = 7.0;
/// Alpha of the translucent texture drawn over the face (0..255).
const OVERLAY_ALPHA: u8 = 150;

/// A running Texture Position.
pub(crate) struct TexturePosition {
    pub face: FaceId,
    /// The back side's texture is positioned (the front has no texture).
    pub back: bool,
    material: usize,
    tile: [f64; 2],
    /// Texture point (repeats) of the red pin.
    corner: DVec2,
    /// Pins' positions in the face's texture plane coordinates.
    pub pins: [DVec2; 4],
    /// The side's mapping before (restored by Cancel).
    original: Option<[f64; 9]>,
    /// Texture plane basis of the face and a point on it (context coordinates).
    xr: DVec3,
    yr: DVec3,
    on_face: DVec3,
    normal: DVec3,
    /// Free Pins: every pin moves alone.
    pub free: bool,
    drag: Option<Drag>,
    /// The document state when the mode started: another edit or an undo ends it.
    state: u64,
    tool: Tool,
    image: Option<egui::TextureHandle>,
}

struct Drag {
    /// `None`: the texture itself is dragged (moves like the red pin).
    pin: Option<Pin>,
    /// Plane point grabbed and the pins at that moment.
    grab: DVec2,
    start: [DVec2; 4],
}

/// The textured material of one side of a face, front first.
fn textured_side(ents: &Entities, materials: &[Material], face: FaceId) -> Option<(bool, usize)> {
    let f = ents.faces.get(face)?;
    [(false, f.attrs.material), (true, f.attrs.back_material)]
        .into_iter()
        .find_map(|(back, m)| {
            let m = m?;
            materials.get(m)?.texture.as_ref()?;
            Some((back, m))
        })
}

/// The face Texture Position works on: the only selected entity, a face painted with
/// a texture on either side.
pub(crate) fn target(ents: &Entities, materials: &[Material], items: &[Owner]) -> Option<FaceId> {
    match items {
        [Owner::Face(f)] => textured_side(ents, materials, *f).map(|_| *f),
        _ => None,
    }
}

impl TexturePosition {
    /// Starts on `face` (see [`target`]).
    pub(crate) fn new(
        ents: &Entities,
        materials: &[Material],
        face: FaceId,
        state: u64,
        tool: Tool,
    ) -> Option<Self> {
        let (back, material) = textured_side(ents, materials, face)?;
        let t = materials[material].texture.as_ref()?;
        let f = &ents.faces[face];
        let (xr, yr) = mt_skp::uv::face_uv_basis(f.normal.to_array());
        let (xr, yr) = (DVec3::from_array(xr), DVec3::from_array(yr));
        let original = f
            .attrs
            .uv
            .as_deref()
            .and_then(|u| if back { u.back } else { u.front });
        let m = original.unwrap_or(DEFAULT_UV);
        let tile = [t.width, t.height];
        // The red pin on the tile corner below-left of the face's center.
        let points = ents.loop_points(&f.outer);
        let center = points.iter().copied().sum::<DVec3>() / points.len().max(1) as f64;
        let mapping = mt_skp::uv::UvMapping::new(f.normal.to_array(), original.as_ref(), tile);
        let uv = mt_kernel::texture::uv_at(&mapping, center);
        let corner = uv.floor();
        Some(Self {
            face,
            back,
            material,
            tile,
            corner,
            pins: pins_from_uv(&m, tile, corner),
            original,
            xr,
            yr,
            on_face: center,
            normal: f.normal,
            free: false,
            drag: None,
            state,
            tool,
            image: None,
        })
    }

    /// The side's mapping the pins give (`None`: the default projection).
    pub(crate) fn uv(&self) -> Option<[f64; 9]> {
        let m = uv_from_pins(&self.pins, self.tile, self.corner)?;
        let default = m.iter().zip(DEFAULT_UV).all(|(a, b)| (a - b).abs() < 1e-9);
        (!default).then_some(m)
    }

    /// Plane coordinates of a point of the face's plane (context coordinates).
    pub(crate) fn to_plane(&self, p: DVec3) -> DVec2 {
        DVec2::new(p.dot(self.xr), p.dot(self.yr))
    }

    /// The point of the face's plane at plane coordinates `q`.
    pub(crate) fn lift(&self, q: DVec2) -> DVec3 {
        self.on_face
            + self.xr * (q.x - self.on_face.dot(self.xr))
            + self.yr * (q.y - self.on_face.dot(self.yr))
    }

    /// Writes `uv` into the face's side.
    fn set_side(&self, ents: &mut Entities, uv: Option<[f64; 9]>) -> bool {
        let Some(f) = ents.faces.get_mut(self.face) else {
            return false;
        };
        let old = f.attrs.uv.as_deref().cloned().unwrap_or_default();
        let (front, back) = if self.back {
            (old.front, uv)
        } else {
            (uv, old.back)
        };
        let changed = (front, back) != (old.front, old.back);
        f.attrs.uv = mt_kernel::FaceUv::boxed(front, back);
        changed
    }

    /// Starts a drag at plane point `at`; `pin` when a pin was grabbed.
    pub(crate) fn press(&mut self, pin: Option<Pin>, at: DVec2) {
        self.drag = Some(Drag {
            pin,
            grab: at,
            start: self.pins,
        });
    }

    /// Moves the drag to plane point `at`. A move that would fold the pins over is
    /// refused (the pins stay where they last were valid).
    pub(crate) fn drag_to(&mut self, at: DVec2) {
        let Some(d) = &self.drag else {
            return;
        };
        let pin = d.pin.unwrap_or(Pin::Red);
        let i = Pin::ALL.iter().position(|p| *p == pin).unwrap_or(0);
        let to = d.start[i] + (at - d.grab);
        let free = self.free && d.pin.is_some();
        let pins = drag_pin(&d.start, pin, to, free);
        if uv_from_pins(&pins, self.tile, self.corner).is_some() {
            self.pins = pins;
        }
    }

    /// Ends a drag; whether one was running.
    pub(crate) fn release(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// Reset: the default projection.
    pub(crate) fn reset(&mut self) {
        self.pins = pins_from_uv(&DEFAULT_UV, self.tile, self.corner);
    }

    pub(crate) fn act(&mut self, action: PinAction) {
        self.pins = pin_action(&self.pins, action);
    }
}

/// What the Texture Position toolbar or menu asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Done,
    Cancel,
    Reset,
    Pins(PinAction),
    ToggleFree,
}

/// The items of the toolbar and the right-click menu.
fn action_buttons(ui: &mut egui::Ui, free: bool) -> Option<Action> {
    let mut out = None;
    let mut b = |ui: &mut egui::Ui, label: &str, a: Action| {
        if ui.button(label).clicked() {
            out = Some(a);
        }
    };
    b(ui, "Done", Action::Done);
    b(ui, "Cancel", Action::Cancel);
    b(ui, "Reset", Action::Reset);
    b(
        ui,
        "Flip Left/Right",
        Action::Pins(PinAction::FlipLeftRight),
    );
    b(ui, "Flip Up/Down", Action::Pins(PinAction::FlipUpDown));
    b(ui, "Rotate 90", Action::Pins(PinAction::Rotate(1)));
    b(ui, "Rotate 180", Action::Pins(PinAction::Rotate(2)));
    b(ui, "Rotate 270", Action::Pins(PinAction::Rotate(3)));
    let mut on = free;
    if ui.checkbox(&mut on, "Free Pins").changed() {
        out = Some(Action::ToggleFree);
    }
    out
}

impl App {
    /// Runs `tool.texture_position`; false for other commands.
    pub(crate) fn texture_command(&mut self, frame: &eframe::Frame, id: &str) -> bool {
        if id != "tool.texture_position" {
            return false;
        }
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.texture_position_end(false, &device);
        }
        let Some(doc) = &self.document else {
            return true;
        };
        let ents = doc.active();
        let started = target(ents, &doc.model.materials, self.selection.items()).and_then(|f| {
            TexturePosition::new(ents, &doc.model.materials, f, doc.state_id(), Tool::Select)
        });
        match started {
            Some(tp) => {
                // Texture Position works like a mode of the Select tool.
                self.tool = Tool::Select;
                self.draw = None;
                self.modify = None;
                self.textures.position = Some(tp);
                self.message = Some(
                    "Drag the pins or the texture. Enter or click outside the face: done; Escape: cancel."
                        .into(),
                );
            }
            None => {
                self.message =
                    Some("Select one face painted with a texture to position it.".into());
            }
        }
        true
    }

    /// Whether Texture Position can start (one textured face selected).
    pub(crate) fn can_position_texture(&self) -> bool {
        self.document.as_ref().is_some_and(|doc| {
            target(doc.active(), &doc.model.materials, self.selection.items()).is_some()
        })
    }

    /// Ends Texture Position: `keep` records the result as one undo step, otherwise
    /// the texture goes back to where it was.
    pub(crate) fn texture_position_end(&mut self, keep: bool, device: &wgpu::Device) {
        let Some(tp) = self.textures.position.take() else {
            return;
        };
        self.message = None;
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        if doc.state_id() != tp.state {
            // The model changed under the mode (undo, another edit): leave it as is.
            self.rebuild(device);
            return;
        }
        tp.set_side(doc.active_mut_untracked(), tp.original);
        if keep {
            let uv = tp.uv();
            doc.edit(|ents| tp.set_side(ents, uv));
        }
        self.rebuild(device);
    }

    /// Mouse and keys while Texture Position runs. Returns false when it is not
    /// running (the active tool gets the input then).
    pub(crate) fn texture_position_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) -> bool {
        let Some(tp) = &self.textures.position else {
            return false;
        };
        // Another tool, an undo or a lost face ends the mode.
        let gone = self
            .document
            .as_ref()
            .is_none_or(|d| d.state_id() != tp.state || !d.active().faces.contains_key(tp.face));
        let other_tool = self.tool != tp.tool;
        if other_tool || gone {
            self.texture_position_end(true, device);
            return false;
        }
        let ppp = ui.ctx().pixels_per_point();
        let ctx_xf = self.ctx_xf;
        let inv = ctx_xf.inverse();
        let camera = self.camera.clone();
        let to_screen = |p: DVec3| {
            camera
                .project(ctx_xf.transform_point3(p), viewport_px)
                .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
        };
        // Plane coordinates under a screen position.
        let plane_at = |tp: &TexturePosition, pos: egui::Pos2| {
            let px = DVec2::new(
                f64::from((pos.x - rect.min.x) * ppp),
                f64::from((pos.y - rect.min.y) * ppp),
            );
            let ray = camera.ray(px, viewport_px);
            let n = ctx_xf.transform_vector3(tp.normal).normalize_or_zero();
            // None when the face is seen edge-on.
            let hit = ray.hit_plane(ctx_xf.transform_point3(tp.on_face), n)?;
            Some((
                tp.to_plane(inv.transform_point3(hit)),
                inv.transform_point3(hit),
            ))
        };

        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let mut action = None;
        if enter {
            action = Some(Action::Done);
        }
        if escape {
            action = Some(Action::Cancel);
        }

        // Toolbar at the top of the viewport.
        let free = tp.free;
        egui::Area::new(egui::Id::new("mt_texture_position_bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.left_top() + egui::vec2(8.0, 8.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Texture Position").strong());
                        if let Some(a) = action_buttons(ui, free) {
                            action = Some(a);
                        }
                    });
                });
            });
        // The same items on right-click.
        response.context_menu(|ui| {
            ui.set_min_width(150.0);
            ui.vertical(|ui| {
                if let Some(a) = action_buttons(ui, free) {
                    action = Some(a);
                    ui.close();
                }
            });
        });

        let tp = self.textures.position.as_mut().expect("running");
        let mut changed = false;
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((at, _)) = plane_at(tp, pos)
        {
            let pin = Pin::ALL.iter().zip(tp.pins).find_map(|(pin, q)| {
                let s = to_screen(tp.lift(q))?;
                (s.distance(pos) <= PIN_GRAB_PX).then_some(*pin)
            });
            tp.press(pin, at);
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((at, _)) = plane_at(tp, pos)
        {
            tp.drag_to(at);
        }
        if response.drag_stopped() && tp.release() {
            changed = true;
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            // A click outside the face finishes.
            let inside = plane_at(tp, pos).is_some_and(|(_, local)| {
                self.document
                    .as_ref()
                    .is_some_and(|d| d.active().face_contains(tp.face, local))
            });
            if !inside {
                action = Some(Action::Done);
            }
        }
        match action {
            Some(Action::Done) => {
                self.texture_position_end(true, device);
                return true;
            }
            Some(Action::Cancel) => {
                self.texture_position_end(false, device);
                return true;
            }
            Some(Action::Reset) => {
                tp.reset();
                changed = true;
            }
            Some(Action::Pins(a)) => {
                tp.act(a);
                changed = true;
            }
            Some(Action::ToggleFree) => tp.free = !tp.free,
            None => {}
        }
        if changed {
            // Show the result on the face itself (not an undo step until Done).
            let uv = tp.uv();
            if let Some(doc) = self.document.as_mut() {
                let tp = self.textures.position.as_ref().expect("running");
                tp.set_side(doc.active_mut_untracked(), uv);
                self.rebuild(device);
            }
        }
        true
    }

    /// Draws the translucent texture and the pins over the viewport image.
    pub(crate) fn texture_position_paint(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        viewport_px: DVec2,
    ) {
        let Some(tp) = self.textures.position.as_mut() else {
            return;
        };
        let Some(doc) = &self.document else {
            return;
        };
        if tp.image.is_none() {
            let data = doc
                .model
                .materials
                .get(tp.material)
                .and_then(|m| m.texture.as_ref())
                .map(|t| t.data.clone());
            if let Some(img) = data.and_then(|d| image::load_from_memory(&d).ok()) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                tp.image = Some(ui.ctx().load_texture(
                    "mt_texture_position",
                    color,
                    egui::TextureOptions::LINEAR_REPEAT,
                ));
            }
        }
        let ppp = ui.ctx().pixels_per_point();
        let ctx_xf = self.ctx_xf;
        let camera = &self.camera;
        let to_screen = |p: DVec3| {
            camera
                .project(ctx_xf.transform_point3(p), viewport_px)
                .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
        };
        let painter = ui.painter().with_clip_rect(rect);
        let m = uv_from_pins(&tp.pins, tp.tile, tp.corner).unwrap_or(DEFAULT_UV);
        // Three by three tiles around the pins' tile, subdivided so a distorted
        // texture looks right.
        // The projective denominator at texture point `t` (1 for an affine mapping).
        let q_at = |t: DVec2| {
            let size = |s: f64| if s > 1e-9 { s } else { 1.0 };
            t.x * size(tp.tile[0]) * m[2] + t.y * size(tp.tile[1]) * m[5] + m[8]
        };
        let q0 = q_at(tp.corner + DVec2::splat(0.5));
        if let Some(image) = &tp.image {
            const N: usize = 18;
            let mut mesh = egui::Mesh::with_texture(image.id());
            let tint = egui::Color32::from_white_alpha(OVERLAY_ALPHA);
            let mut index = vec![None; (N + 1) * (N + 1)];
            for j in 0..=N {
                for i in 0..=N {
                    let t =
                        tp.corner - DVec2::ONE + DVec2::new(i as f64, j as f64) * 3.0 / N as f64;
                    // A distorted texture runs off to its horizon: only the part well
                    // on the pins' side of it is drawn.
                    let q = q_at(t);
                    if q * q0 <= 0.0 || q.abs() < 0.3 * q0.abs() {
                        continue;
                    }
                    let p = tp.lift(plane_point(&m, tp.tile, t));
                    if let Some(s) = to_screen(p) {
                        index[j * (N + 1) + i] = Some(mesh.vertices.len() as u32);
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: s,
                            // Image row 0 is at the top of a tile (v = 1).
                            uv: egui::pos2(t.x as f32, (1.0 - t.y) as f32),
                            color: tint,
                        });
                    }
                }
            }
            for j in 0..N {
                for i in 0..N {
                    let k = |di: usize, dj: usize| index[(j + dj) * (N + 1) + i + di];
                    if let (Some(a), Some(b), Some(c), Some(d)) =
                        (k(0, 0), k(1, 0), k(1, 1), k(0, 1))
                    {
                        mesh.add_triangle(a, b, c);
                        mesh.add_triangle(a, c, d);
                    }
                }
            }
            painter.add(egui::Shape::mesh(mesh));
        }
        // The pins' tile outline and the pins.
        let screen: Vec<Option<egui::Pos2>> =
            tp.pins.iter().map(|q| to_screen(tp.lift(*q))).collect();
        let outline = [0, 1, 3, 2, 0];
        for w in outline.windows(2) {
            if let (Some(a), Some(b)) = (screen[w[0]], screen[w[1]]) {
                painter.line_segment([a, b], egui::Stroke::new(1.0, egui::Color32::WHITE));
                painter.line_segment(
                    [a, b],
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
                );
            }
        }
        let colors = [
            egui::Color32::from_rgb(220, 30, 30),
            egui::Color32::from_rgb(30, 170, 40),
            egui::Color32::from_rgb(40, 70, 230),
            egui::Color32::from_rgb(235, 200, 20),
        ];
        let hover = ui.input(|i| i.pointer.hover_pos());
        for (s, color) in screen.iter().zip(colors) {
            if let Some(s) = s {
                let hot = hover.is_some_and(|h| h.distance(*s) <= PIN_GRAB_PX);
                let r = if hot { PIN_RADIUS + 2.0 } else { PIN_RADIUS };
                painter.circle(*s, r, color, egui::Stroke::new(1.5, egui::Color32::WHITE));
                painter.circle_filled(*s, 2.0, egui::Color32::WHITE);
            }
        }
        if tp.drag.is_some() {
            ui.ctx().request_repaint();
        }
    }
}

/// State of the texture features that lives across frames.
#[derive(Default)]
pub(crate) struct Textures {
    pub position: Option<TexturePosition>,
    /// A texture being edited in an external editor.
    pub external: Option<crate::material_edit::ExternalEdit>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A 10 x 10 square on the ground painted with a 4 x 2 tiled texture.
    fn textured_square() -> (Entities, Vec<Material>, FaceId) {
        let mut e = Entities::new();
        let p = [
            DVec3::ZERO,
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::new(0.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            mt_kernel::add_line(&mut e, p[i], p[(i + 1) % 4]);
        }
        let f = e.faces.keys().next().unwrap();
        e.faces[f].attrs.material = Some(0);
        let materials = vec![Material {
            name: "Bricks".into(),
            color: [150, 80, 60],
            opacity: 1.0,
            texture: Some(mt_kernel::Texture {
                filename: "bricks.png".into(),
                data: Arc::from(Vec::new()),
                width: 4.0,
                height: 2.0,
                colorize: false,
            }),
        }];
        (e, materials, f)
    }

    fn uv_of(e: &Entities, materials: &[Material], f: FaceId, p: DVec3) -> DVec2 {
        let map = mt_kernel::texture::face_mapping(&e.faces[f], false, &materials[0]).unwrap();
        mt_kernel::texture::uv_at(&map, p)
    }

    #[test]
    fn needs_one_textured_face() {
        let (mut e, mut materials, f) = textured_square();
        assert_eq!(target(&e, &materials, &[Owner::Face(f)]), Some(f));
        assert_eq!(target(&e, &materials, &[]), None);
        // The back side counts too.
        e.faces[f].attrs.back_material = Some(0);
        e.faces[f].attrs.material = None;
        let tp = TexturePosition::new(&e, &materials, f, 0, Tool::Select).unwrap();
        assert!(tp.back);
        materials[0].texture = None;
        assert_eq!(target(&e, &materials, &[Owner::Face(f)]), None);
    }

    #[test]
    fn dragging_the_red_pin_moves_the_texture() {
        let (mut e, materials, f) = textured_square();
        let mut tp = TexturePosition::new(&e, &materials, f, 0, Tool::Select).unwrap();
        // The red pin starts on a tile corner, at its texture point.
        let red = tp.lift(tp.pins[0]);
        let uv = uv_of(&e, &materials, f, red);
        assert!(uv.abs_diff_eq(uv.round(), 1e-9), "{uv}");
        let at = tp.to_plane(red);
        tp.press(Some(Pin::Red), at);
        tp.drag_to(at + DVec2::new(1.0, 0.5));
        assert!(tp.release());
        let uv_m = tp.uv();
        assert!(tp.set_side(&mut e, uv_m));
        // What was under the old red pin's spot now shows a quarter tile to the left.
        let after = uv_of(&e, &materials, f, red);
        assert!(
            after.abs_diff_eq(uv - DVec2::new(0.25, 0.25), 1e-9),
            "{after}"
        );
        // Setting the default mapping back stores none.
        tp.reset();
        assert_eq!(tp.uv(), None);
        tp.set_side(&mut e, tp.uv());
        assert!(e.faces[f].attrs.uv.is_none());
    }

    #[test]
    fn green_pin_scales_and_dragging_the_texture_moves_it() {
        let (e, materials, f) = textured_square();
        let mut tp = TexturePosition::new(&e, &materials, f, 0, Tool::Select).unwrap();
        let [red, green, ..] = tp.pins;
        tp.press(Some(Pin::Green), green);
        tp.drag_to(red + (green - red) * 2.0);
        tp.release();
        let m = tp.uv().unwrap();
        // Twice as big: one tile now spans 8 units.
        let a = plane_point(&m, tp.tile, tp.corner);
        let b = plane_point(&m, tp.tile, tp.corner + DVec2::X);
        assert!((a.distance(b) - 8.0).abs() < 1e-9);
        // Dragging off the pins moves everything.
        let before = tp.pins;
        tp.press(None, DVec2::new(5.0, 5.0));
        tp.drag_to(DVec2::new(6.0, 5.0));
        tp.release();
        for (a, b) in tp.pins.iter().zip(before) {
            assert!(a.abs_diff_eq(b + DVec2::X, 1e-12));
        }
        // A distortion that folds the pins over is refused.
        let before = tp.pins;
        tp.press(Some(Pin::Yellow), tp.pins[3]);
        tp.drag_to(tp.pins[0] - DVec2::new(50.0, 50.0));
        assert_eq!(tp.pins, before);
    }
}
