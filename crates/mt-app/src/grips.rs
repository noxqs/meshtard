//! Move tool grips on group and component bounding boxes (FEATURES.md 58; SketchUp
//! help "Moving entities around", release notes 2024 / 2025).
//!
//! With the Move tool idle and the cursor near a selected group or component, its
//! bounding box shows grips: the corners first; Alt cycles to the edge midpoints, the
//! side centers and the center. Clicking (or pressing) a grip takes it as Move's base
//! point, so the instance moves by that point. A grip on a corner hidden behind the
//! box draws the instance transparent while hovered, as the help describes.
//!
//! Each face of the box facing the camera also shows four rotation handles. Hovering
//! one shows a protractor in that face's plane; dragging it (or clicking it, moving
//! and clicking again) rotates the instance about the box center, around the axis
//! normal to that face. Angles snap to Model Info's angle snapping. One undo step;
//! Escape cancels.
//!
//! Visuals are unverified: grip squares are the Scale tool's, the handles are small
//! red crosses at half way from the face center to its sides, the protractor has
//! 15 degree ticks. Typed angles while rotating by a handle are not supported.

use eframe::egui;
use egui_wgpu::wgpu;
use glam::{DMat4, DQuat, DVec2, DVec3};

use crate::App;
use crate::document::Owner;
use crate::nav::Tool;
use crate::selection::OrientedBox;

/// Grip and handle pick radius, pixels.
const PICK_PX: f64 = 9.0;
/// Grips show when the cursor is this close to the box's screen bounds, pixels.
const NEAR_PX: f64 = 40.0;
/// Opacity of an instance shown transparent (a hidden grip is hovered; unverified).
pub(crate) const TRANSPARENT_ALPHA: f32 = 0.3;

/// Which box points the grips sit on (Alt cycles them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum GripSet {
    #[default]
    Corners,
    EdgeMidpoints,
    SideCenters,
    Center,
}

impl GripSet {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Corners => Self::EdgeMidpoints,
            Self::EdgeMidpoints => Self::SideCenters,
            Self::SideCenters => Self::Center,
            Self::Center => Self::Corners,
        }
    }

    /// Box coordinates (-1, 0, 1 per axis) of the grips.
    pub(crate) fn coords(self) -> Vec<DVec3> {
        let all = (0..27).map(|k| {
            DVec3::new(
                f64::from(k % 3) - 1.0,
                f64::from(k / 3 % 3) - 1.0,
                f64::from(k / 9) - 1.0,
            )
        });
        let ones = |g: &DVec3| g.to_array().iter().filter(|x| x.abs() > 0.5).count();
        let want = match self {
            Self::Corners => 3,
            Self::EdgeMidpoints => 2,
            Self::SideCenters => 1,
            Self::Center => 0,
        };
        all.filter(|g| ones(g) == want).collect()
    }
}

/// A grip point of a box: box coordinates and position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Grip {
    pub coords: DVec3,
    pub point: DVec3,
}

/// The grips of `b` in `set`; points of a flat box that coincide are listed once.
pub(crate) fn grips(b: &OrientedBox, set: GripSet) -> Vec<Grip> {
    let mut out: Vec<Grip> = Vec::new();
    for g in set.coords() {
        let p = b.at(g);
        if !out.iter().any(|q| q.point.distance(p) < 1e-9) {
            out.push(Grip {
                coords: g,
                point: p,
            });
        }
    }
    out
}

/// Whether the grip at box coordinates `g` is hidden behind the box for a camera at
/// `eye` (perspective) or looking along `view_dir` (parallel): every face of the box
/// it lies on faces away. The center is inside the box: hidden. Points of a flat box
/// are never hidden.
pub(crate) fn grip_hidden(b: &OrientedBox, g: DVec3, eye: Option<DVec3>, view_dir: DVec3) -> bool {
    if b.half.min_element() < 1e-9 {
        return false;
    }
    let p = b.at(g);
    let to_point = |p: DVec3| eye.map_or(view_dir, |e| p - e);
    let mut faces = 0;
    for i in 0..3 {
        if g[i].abs() < 0.5 {
            continue;
        }
        faces += 1;
        let outward = b.dirs[i] * g[i].signum();
        if outward.dot(to_point(p)) <= 0.0 {
            return false;
        }
    }
    faces > 0 || g == DVec3::ZERO
}

/// A rotation handle on a box face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Handle {
    /// Where it is drawn and picked.
    pub point: DVec3,
    /// Center of its face (the protractor's center).
    pub face_center: DVec3,
    /// Unit rotation axis: the face's outward normal.
    pub axis: DVec3,
}

/// The rotation handles on the faces of `b` that face the camera: four per face, half
/// way from the face center towards its sides. Flat faces (no area) get none.
pub(crate) fn handles(b: &OrientedBox, eye: Option<DVec3>, view_dir: DVec3) -> Vec<Handle> {
    let mut out = Vec::new();
    for i in 0..3 {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        if b.half[j] < 1e-9 || b.half[k] < 1e-9 {
            continue;
        }
        for s in [-1.0, 1.0] {
            let axis = b.dirs[i] * s;
            let face_center = b.center + axis * b.half[i];
            let to_face = eye.map_or(view_dir, |e| face_center - e);
            // A flat box shows its one face from both sides.
            if b.half[i] > 1e-9 && axis.dot(to_face) >= 0.0 {
                continue;
            }
            if b.half[i] <= 1e-9 && s > 0.0 {
                continue;
            }
            for (d, h) in [(b.dirs[j], b.half[j]), (b.dirs[k], b.half[k])] {
                for t in [-0.5, 0.5] {
                    out.push(Handle {
                        point: face_center + d * h * t,
                        face_center,
                        axis,
                    });
                }
            }
        }
    }
    out
}

/// Signed angle (radians, right-handed about `axis`) from `from` to `to`, both
/// measured from `center` in the plane normal to `axis`; `None` when either lies on
/// the axis.
pub(crate) fn angle_about(center: DVec3, axis: DVec3, from: DVec3, to: DVec3) -> Option<f64> {
    let flat = |v: DVec3| {
        let f = v - axis * v.dot(axis);
        (f.length() > 1e-9).then(|| f.normalize())
    };
    let (u, w) = (flat(from - center)?, flat(to - center)?);
    Some(u.cross(w).dot(axis).atan2(u.dot(w)))
}

/// Rotation by `angle` about the line through `center` along unit `axis`.
pub(crate) fn rotation_about(center: DVec3, axis: DVec3, angle: f64) -> DMat4 {
    DMat4::from_translation(center)
        * DMat4::from_quat(DQuat::from_axis_angle(axis, angle))
        * DMat4::from_translation(-center)
}

/// Where the ray meets the plane through `p` with normal `n`.
fn ray_plane(origin: DVec3, dir: DVec3, p: DVec3, n: DVec3) -> Option<DVec3> {
    let d = dir.dot(n);
    if d.abs() < 1e-12 {
        return None;
    }
    let t = (p - origin).dot(n) / d;
    (t > 0.0).then(|| origin + dir * t)
}

/// What is under the cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hot {
    Grip {
        instance: usize,
        grip: Grip,
        hidden: bool,
    },
    Handle {
        instance: usize,
        handle: Handle,
        center: DVec3,
    },
}

/// A rotation by a handle in progress (context space).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rotation {
    instance: usize,
    /// The box center and the unit rotation axis.
    center: DVec3,
    axis: DVec3,
    /// The protractor's center (the face center) and the grabbed point.
    face_center: DVec3,
    start: DVec3,
    angle: f64,
    /// Started by a click (not a drag): the next click finishes.
    by_click: bool,
}

/// State of the Move tool's grips.
#[derive(Debug, Default)]
pub(crate) struct Grips {
    set: GripSet,
    alt_was: bool,
    hot: Option<Hot>,
    /// The instance whose box shows grips.
    shown: Option<(usize, OrientedBox)>,
    rotation: Option<Rotation>,
    /// The grip grabbed this frame: Move's base point (context space).
    base: Option<DVec3>,
    /// The instance drawn transparent (a hidden grip is hovered).
    transparent: Option<usize>,
    transparent_sent: Option<usize>,
}

impl Grips {
    /// The grabbed grip, once: Move's first click takes it as its base point.
    pub(crate) fn take_base(&mut self) -> Option<DVec3> {
        self.base.take()
    }

    /// The instance rotated by a handle: drawn live like Move's targets.
    pub(crate) fn live(&self) -> Option<(Vec<Owner>, DMat4, bool)> {
        let r = self.rotation?;
        Some((
            vec![Owner::Instance(r.instance)],
            rotation_about(r.center, r.axis, r.angle),
            false,
        ))
    }

    pub(crate) fn is_rotating(&self) -> bool {
        self.rotation.is_some()
    }

    fn reset(&mut self) {
        let sent = self.transparent_sent;
        *self = Self {
            set: self.set,
            transparent_sent: sent,
            ..Self::default()
        };
    }
}

/// Snaps `angle` (radians) to Model Info's angle snapping when it is close to a
/// multiple of the snap angle.
fn snapped(angle: f64) -> f64 {
    mt_infer::units::snap_angle_display(angle)
}

impl App {
    /// The grips' part of the Move tool's frame. Returns true when the grips used the
    /// mouse this frame (a rotation by a handle), so the Move tool must not.
    pub(crate) fn grips_frame(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) -> bool {
        let moving = self.modify.as_ref().is_some_and(|m| m.anchor().is_some());
        if self.tool != Tool::Modify("tool.move") || (moving && !self.grips.is_rotating()) {
            self.grips.reset();
            self.send_transparent(device);
            return false;
        }
        let ppp = ui.ctx().pixels_per_point();
        let cursor = response
            .hover_pos()
            .or(response.interact_pointer_pos())
            .map(|p| {
                DVec2::new(
                    f64::from((p.x - rect.min.x) * ppp),
                    f64::from((p.y - rect.min.y) * ppp),
                )
            });
        let xf = self.ctx_xf;
        let inv = xf.inverse();
        let local_ray = cursor.map(|c| {
            let r = self.camera.ray(c, viewport_px);
            (
                inv.transform_point3(r.origin),
                inv.transform_vector3(r.dir).normalize_or_zero(),
            )
        });
        let primary = egui::PointerButton::Primary;
        let (escape, alt) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.modifiers.alt));
        let clicked = response.clicked_by(primary);
        let pressed = response.drag_started_by(primary);
        // A drag starts where the button went down: pick what was under the press.
        let pick_cursor = if pressed {
            ui.input(|i| i.pointer.press_origin())
                .map(|p| {
                    DVec2::new(
                        f64::from((p.x - rect.min.x) * ppp),
                        f64::from((p.y - rect.min.y) * ppp),
                    )
                })
                .or(cursor)
        } else {
            cursor
        };
        let released = response.drag_stopped_by(primary);

        if let Some(mut r) = self.grips.rotation {
            // No inference marker while rotating by a handle.
            self.inference = None;
            if escape {
                self.grips.rotation = None;
                return true;
            }
            if let Some((o, d)) = local_ray
                && let Some(p) = ray_plane(o, d, r.face_center, r.axis)
                && let Some(a) = angle_about(r.face_center, r.axis, r.start, p)
            {
                r.angle = snapped(a);
            }
            self.grips.rotation = Some(r);
            let finish = (released && !r.by_click) || (clicked && r.by_click);
            if released && r.angle.abs() < 1e-9 && !r.by_click {
                // Pressed and released in place: rotate until the next click.
                self.grips.rotation = Some(Rotation {
                    by_click: true,
                    ..r
                });
                return true;
            }
            if finish {
                self.grips.rotation = None;
                let rot = rotation_about(r.center, r.axis, r.angle);
                let changed = r.angle.abs() > 1e-12
                    && self.document.as_mut().is_some_and(|doc| {
                        doc.edit(|ents| match ents.instances.get_mut(r.instance) {
                            Some(inst) if !inst.locked => {
                                inst.transform = rot * inst.transform;
                                true
                            }
                            _ => false,
                        })
                    });
                if changed {
                    self.rebuild(device);
                }
            }
            return true;
        }

        // Idle Move tool: grips of the selected instance near the cursor.
        let Some(doc) = &self.document else {
            return false;
        };
        let ents = doc.active();
        let camera = &self.camera;
        let project = |p: DVec3| camera.project(xf.transform_point3(p), viewport_px);
        let eye = camera.perspective.then(|| inv.transform_point3(camera.eye));
        let view_dir = inv
            .transform_vector3(camera.target - camera.eye)
            .normalize_or_zero();
        let mut shown = None;
        let mut best: Option<(f64, Hot)> = None;
        for o in self.selection.items() {
            let Owner::Instance(i) = *o else { continue };
            let (Some(inst), Some((min, max)), Some(c)) =
                (ents.instances.get(i), self.inst_bounds.get(&i), pick_cursor)
            else {
                continue;
            };
            if inst.locked {
                continue;
            }
            let b = OrientedBox::from_aabb(*min, *max).transformed(inst.transform);
            let corners: Vec<DVec2> = b.corners().iter().filter_map(|p| project(*p)).collect();
            if corners.len() < 8 {
                continue;
            }
            let lo = corners.iter().fold(DVec2::INFINITY, |a, p| a.min(*p));
            let hi = corners.iter().fold(DVec2::NEG_INFINITY, |a, p| a.max(*p));
            if c.x < lo.x - NEAR_PX
                || c.y < lo.y - NEAR_PX
                || c.x > hi.x + NEAR_PX
                || c.y > hi.y + NEAR_PX
            {
                continue;
            }
            if shown.is_none() {
                shown = Some((i, b));
            }
            let mut consider = |p: DVec3, hot: Hot| {
                if let Some(q) = project(p) {
                    let d = q.distance(c);
                    if d <= PICK_PX && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, hot));
                        shown = Some((i, b));
                    }
                }
            };
            for grip in grips(&b, self.grips.set) {
                let hidden = grip_hidden(&b, grip.coords, eye, view_dir);
                consider(
                    grip.point,
                    Hot::Grip {
                        instance: i,
                        grip,
                        hidden,
                    },
                );
            }
            for handle in handles(&b, eye, view_dir) {
                consider(
                    handle.point,
                    Hot::Handle {
                        instance: i,
                        handle,
                        center: b.center,
                    },
                );
            }
        }
        let hot = best.map(|(_, h)| h);
        // Alt (pressed, not held) cycles the grip points while grips are shown.
        if alt && !self.grips.alt_was && shown.is_some() {
            self.grips.set = self.grips.set.next();
        }
        self.grips.alt_was = alt;
        self.grips.shown = shown;
        self.grips.hot = hot;
        self.grips.transparent = match hot {
            Some(Hot::Grip {
                instance,
                hidden: true,
                ..
            }) => Some(instance),
            _ => None,
        };
        self.send_transparent(device);
        if !(clicked || pressed) {
            return false;
        }
        match hot {
            Some(Hot::Grip { grip, .. }) => {
                self.grips.base = Some(grip.point);
                false
            }
            Some(Hot::Handle {
                instance,
                handle,
                center,
            }) => {
                self.grips.rotation = Some(Rotation {
                    instance,
                    center,
                    axis: handle.axis,
                    face_center: handle.face_center,
                    start: handle.point,
                    angle: 0.0,
                    by_click: clicked,
                });
                self.grips.transparent = None;
                self.send_transparent(device);
                true
            }
            None => false,
        }
    }

    /// Draws the instance with a hovered hidden grip transparent: its faces leave the
    /// model's buffer and come back as translucent preview faces.
    fn send_transparent(&mut self, device: &wgpu::Device) {
        let want = self.grips.transparent;
        if want == self.grips.transparent_sent || self.live.is_some() {
            return;
        }
        self.grips.transparent_sent = want;
        match want {
            Some(i) => {
                let me = Some(Owner::Instance(i));
                let mut kept = Vec::new();
                let mut vertices = Vec::new();
                for (t, owner) in self.scene.tri_owner.iter().enumerate() {
                    let tri = &self.scene.indices[3 * t..3 * t + 3];
                    if *owner == me {
                        for &k in tri {
                            let mut v = self.scene.vertices[k as usize];
                            v.front[3] *= TRANSPARENT_ALPHA;
                            v.back[3] *= TRANSPARENT_ALPHA;
                            vertices.push(v);
                        }
                    } else {
                        kept.extend_from_slice(tri);
                    }
                }
                self.viewport.set_faces(device, &self.scene.vertices, &kept);
                let indices: Vec<u32> = (0..vertices.len() as u32).collect();
                self.viewport.set_preview_faces(device, &vertices, &indices);
            }
            None => {
                self.viewport
                    .set_faces(device, &self.scene.vertices, &self.scene.indices);
                self.viewport.set_preview_faces(device, &[], &[]);
            }
        }
    }

    /// Paints the grips, rotation handles and the protractor of a hovered or dragged
    /// handle.
    pub(crate) fn grips_paint(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        if self.tool != Tool::Modify("tool.move") {
            // Another tool: no grips, and the transparent instance is drawn again.
            self.grips.reset();
            self.send_transparent(device);
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let xf = self.ctx_xf;
        let camera = &self.camera;
        let to_screen = |p: DVec3| {
            camera
                .project(xf.transform_point3(p), viewport_px)
                .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
        };
        let painter = ui.painter().with_clip_rect(rect);
        if let Some(r) = self.grips.rotation {
            let end = rotation_about(r.face_center, r.axis, r.angle).transform_point3(r.start);
            protractor(&painter, &to_screen, r.face_center, r.axis, r.start);
            let line = |a: DVec3, b: DVec3, c: egui::Color32| {
                if let (Some(a), Some(b)) = (to_screen(a), to_screen(b)) {
                    painter.line_segment([a, b], egui::Stroke::new(1.5, c));
                }
            };
            line(r.face_center, r.start, egui::Color32::from_gray(90));
            line(r.face_center, end, egui::Color32::from_rgb(200, 30, 30));
            return;
        }
        let Some((_, b)) = self.grips.shown else {
            return;
        };
        let inv = xf.inverse();
        let eye = camera.perspective.then(|| inv.transform_point3(camera.eye));
        let view_dir = inv
            .transform_vector3(camera.target - camera.eye)
            .normalize_or_zero();
        let hot_point = match self.grips.hot {
            Some(Hot::Grip { grip, .. }) => Some(grip.point),
            _ => None,
        };
        let markers: Vec<(DVec3, bool)> = grips(&b, self.grips.set)
            .into_iter()
            .map(|g| (xf.transform_point3(g.point), Some(g.point) == hot_point))
            .collect();
        let to_screen_world = |p: DVec3| {
            camera
                .project(p, viewport_px)
                .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
        };
        crate::overlay::markers(&painter, to_screen_world, &markers);
        let hot_handle = match self.grips.hot {
            Some(Hot::Handle { handle, .. }) => Some(handle),
            _ => None,
        };
        for h in handles(&b, eye, view_dir) {
            let Some(q) = to_screen(h.point) else {
                continue;
            };
            let hot = hot_handle == Some(h);
            let (size, color) = if hot {
                (6.0, egui::Color32::from_rgb(230, 30, 30))
            } else {
                (4.0, egui::Color32::from_rgb(200, 60, 60))
            };
            let stroke = egui::Stroke::new(if hot { 2.0 } else { 1.5 }, color);
            painter.line_segment(
                [q - egui::vec2(size, 0.0), q + egui::vec2(size, 0.0)],
                stroke,
            );
            painter.line_segment(
                [q - egui::vec2(0.0, size), q + egui::vec2(0.0, size)],
                stroke,
            );
        }
        if let Some(h) = hot_handle {
            protractor(&painter, &to_screen, h.face_center, h.axis, h.point);
        }
    }
}

/// A protractor in the plane through `center` normal to `axis`, its radius reaching
/// `rim`, with ticks every 15 degrees.
fn protractor(
    painter: &egui::Painter,
    to_screen: &impl Fn(DVec3) -> Option<egui::Pos2>,
    center: DVec3,
    axis: DVec3,
    rim: DVec3,
) {
    let r = rim - center;
    let r = r - axis * r.dot(axis);
    let radius = r.length();
    if radius < 1e-9 {
        return;
    }
    let u = r / radius;
    let v = axis.cross(u);
    let at = |a: f64, k: f64| center + (u * a.cos() + v * a.sin()) * radius * k;
    let color = egui::Color32::from_rgb(40, 90, 220);
    let pts: Vec<egui::Pos2> = (0..=72)
        .filter_map(|i| to_screen(at(f64::from(i) * std::f64::consts::TAU / 72.0, 1.0)))
        .collect();
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.0, color)));
    for i in 0..24 {
        let a = f64::from(i) * std::f64::consts::PI / 12.0;
        let inner = if i % 6 == 0 { 0.85 } else { 0.92 };
        if let (Some(p), Some(q)) = (to_screen(at(a, inner)), to_screen(at(a, 1.0))) {
            painter.line_segment([p, q], egui::Stroke::new(1.0, color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> OrientedBox {
        OrientedBox::from_aabb(DVec3::ZERO, DVec3::new(2.0, 4.0, 6.0))
    }

    #[test]
    fn grip_sets_cycle_and_count() {
        let b = unit_box();
        assert_eq!(grips(&b, GripSet::Corners).len(), 8);
        assert_eq!(grips(&b, GripSet::EdgeMidpoints).len(), 12);
        assert_eq!(grips(&b, GripSet::SideCenters).len(), 6);
        assert_eq!(
            grips(&b, GripSet::Center),
            [Grip {
                coords: DVec3::ZERO,
                point: DVec3::new(1.0, 2.0, 3.0)
            }]
        );
        let mut s = GripSet::Corners;
        for _ in 0..4 {
            s = s.next();
        }
        assert_eq!(s, GripSet::Corners, "Alt cycles back to the corners");
        // Edge midpoints are the middles of the box's edges.
        let mids = grips(&b, GripSet::EdgeMidpoints);
        assert!(mids.iter().any(|g| g.point == DVec3::new(1.0, 0.0, 0.0)));
        assert!(mids.iter().any(|g| g.point == DVec3::new(2.0, 4.0, 3.0)));
        // A flat box (a face in a component) lists its four corners once.
        let flat = OrientedBox::from_aabb(DVec3::ZERO, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(grips(&flat, GripSet::Corners).len(), 4);
    }

    #[test]
    fn hidden_corner_is_the_one_behind_the_box() {
        let b = unit_box();
        // Camera far out at +x +y +z: the corner at the origin is behind the box.
        let eye = Some(DVec3::new(50.0, 60.0, 70.0));
        let dir = DVec3::ZERO;
        assert!(grip_hidden(&b, DVec3::new(-1.0, -1.0, -1.0), eye, dir));
        assert!(!grip_hidden(&b, DVec3::new(1.0, 1.0, 1.0), eye, dir));
        assert!(!grip_hidden(&b, DVec3::new(-1.0, 1.0, 1.0), eye, dir));
        // Parallel projection looking down: the bottom face's points are hidden.
        let down = DVec3::new(0.1, 0.2, -1.0).normalize();
        assert!(grip_hidden(&b, DVec3::new(1.0, 1.0, -1.0), None, down));
        assert!(!grip_hidden(&b, DVec3::new(1.0, 1.0, 1.0), None, down));
        // Grabbing the center shows the box transparent; flat boxes hide nothing.
        assert!(grip_hidden(&b, DVec3::ZERO, eye, dir));
        let flat = OrientedBox::from_aabb(DVec3::ZERO, DVec3::new(2.0, 2.0, 0.0));
        assert!(!grip_hidden(&flat, DVec3::new(-1.0, -1.0, 0.0), eye, dir));
    }

    #[test]
    fn handles_on_front_faces_rotate_about_their_normals() {
        let b = unit_box();
        let eye = DVec3::new(50.0, 60.0, 70.0);
        let hs = handles(&b, Some(eye), DVec3::ZERO);
        // Three faces face the camera, four handles each.
        assert_eq!(hs.len(), 12);
        for h in &hs {
            assert!(h.axis.dot(h.face_center - eye) < 0.0);
            assert!(
                (h.point - h.face_center).dot(h.axis).abs() < 1e-9,
                "in the face"
            );
        }
        let top: Vec<_> = hs.iter().filter(|h| h.axis == DVec3::Z).collect();
        assert_eq!(top.len(), 4);
        assert_eq!(top[0].face_center, DVec3::new(1.0, 2.0, 6.0));
    }

    #[test]
    fn rotating_by_a_handle_turns_about_the_box_center() {
        let b = unit_box();
        let axis = DVec3::Z;
        let face_center = b.center + axis * b.half.z;
        // From the handle towards +x to a point towards +y: a quarter turn.
        let a = angle_about(
            face_center,
            axis,
            face_center + DVec3::X,
            face_center + DVec3::Y * 3.0,
        )
        .unwrap();
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        let r = rotation_about(b.center, axis, a);
        // The center stays, a corner goes round it.
        assert!(r.transform_point3(b.center).distance(b.center) < 1e-12);
        let p = r.transform_point3(DVec3::new(2.0, 2.0, 0.0));
        assert!(p.distance(DVec3::new(1.0, 3.0, 0.0)) < 1e-12);
        assert!(angle_about(face_center, axis, face_center, face_center + DVec3::X).is_none());
    }
}
