//! Component Snaps (FEATURES.md 59; help page "Snaps", RN 2024 and 2025): the snap
//! disks of groups and components ([`mt_kernel::snaps`]), the Make/Edit Snaps tool,
//! and what Move and Place Component see of them.
//!
//! - Make/Edit Snaps (right click inside an open group or component, or on one: it
//!   opens first): Modify mode: a click places a snap, a second click gives the
//!   direction its front faces (toward the cursor; arrow keys lock it to an axis); a
//!   click on a snap's center picks it up and the next click puts it down; a click on
//!   its rotation grip (the square on its rim) then a click turns its Alignment
//!   Direction. Ctrl switches to Erase mode (a click on a center erases the snap) and
//!   back [doc]. Alt over a snap reverses it [doc].
//! - Disks are drawn (white front, blue back [doc], the center dot, a line to the rim
//!   along the Alignment Direction) for the open context's own snaps, and by Move and
//!   Place Component for the component under the cursor, the one moved and the target.
//! - Move: a component picked up by one of its snaps joins another component's snap
//!   near the cursor, or attaches to the face under it; Alt finds the nearest snap
//!   wherever the cursor is (the help's "tractor beam") [doc].
//! - Select > Connected by Snaps [RN 2025].
//!
//! Sizes (disk radius 12 px, 10 px pick distance, 14 px to join), colors other than
//! white and blue, the grip's look and the clicks of the tool are from memory or
//! guesses (unverified). Model-only: snaps are not saved in .skp files.

use eframe::egui;
use glam::{DMat4, DVec2, DVec3};
use mt_infer::Inference;
use mt_infer::units::LengthUnit;
use mt_kernel::Entities;
use mt_kernel::snaps::{Definitions, Snap};

use crate::App;
use crate::document::Owner;
use crate::modify::{Ctx, ModifyTool};
use crate::nav::Tool;

/// Disk radius on screen.
pub const DISK_PX: f64 = 12.0;
/// A snap center within this many pixels of the cursor is under it.
pub const PICK_PX: f64 = 10.0;
/// A target snap within this many pixels of the cursor is joined.
pub const JOIN_PX: f64 = 14.0;

/// A snap of an instance of the open context, in context coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextSnap {
    pub instance: usize,
    /// The snap in the definition's coordinates.
    pub local: Snap,
    pub snap: Snap,
}

/// The snaps of the open context's instances.
pub fn context_snaps(defs: &Definitions, ents: &Entities) -> Vec<ContextSnap> {
    let mut out = Vec::new();
    for (i, inst) in ents.instances.iter().enumerate() {
        if inst.hidden {
            continue;
        }
        let Some(d) = defs.get(inst.definition) else {
            continue;
        };
        for s in &d.entities.snaps {
            out.push(ContextSnap {
                instance: i,
                local: *s,
                snap: s.transformed(inst.transform),
            });
        }
    }
    out
}

/// Select > Connected by Snaps: the selected groups and components and every one
/// joined to them through snaps, transitively.
pub fn connected(defs: &Definitions, ents: &Entities, items: &[Owner]) -> Vec<Owner> {
    let start: Vec<usize> = items
        .iter()
        .filter_map(|o| match *o {
            Owner::Instance(i) => Some(i),
            _ => None,
        })
        .collect();
    mt_kernel::snaps::connected_by_snaps(defs, ents, &start)
        .into_iter()
        .map(Owner::Instance)
        .collect()
}

/// What a modify tool sees of snaps each frame (context coordinates).
pub struct SnapCtx<'a> {
    pub defs: &'a Definitions,
    /// The open context.
    pub ents: &'a Entities,
    pub snaps: Vec<ContextSnap>,
    pub project: &'a dyn Fn(DVec3) -> Option<DVec2>,
    pub cursor: Option<DVec2>,
    /// The top-level entity under the cursor.
    pub picked: Option<Owner>,
    pub alt: bool,
    /// The disk radius at a point, in context units.
    pub radius: &'a dyn Fn(DVec3) -> f64,
}

impl SnapCtx<'_> {
    /// Pixels from the cursor to `p`.
    pub fn px(&self, p: DVec3) -> Option<f64> {
        Some((self.project)(p)?.distance(self.cursor?))
    }

    /// The snap nearest the cursor within `max` pixels, among those `keep` accepts.
    pub fn under_cursor(
        &self,
        max: f64,
        keep: impl Fn(&ContextSnap) -> bool,
    ) -> Option<ContextSnap> {
        self.snaps
            .iter()
            .filter(|s| keep(s))
            .filter_map(|s| Some((self.px(s.snap.position)?, *s)))
            .filter(|(d, _)| *d <= max)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, s)| s)
    }

    /// The snap nearest `p` (context space) among those `keep` accepts.
    pub fn nearest(&self, p: DVec3, keep: impl Fn(&ContextSnap) -> bool) -> Option<ContextSnap> {
        self.snaps
            .iter()
            .filter(|s| keep(s))
            .min_by(|a, b| {
                a.snap
                    .position
                    .distance(p)
                    .total_cmp(&b.snap.position.distance(p))
            })
            .copied()
    }

    /// The disks of instance `i` (the component under the cursor).
    pub fn disks_of(&self, i: usize) -> Vec<Disk> {
        self.snaps
            .iter()
            .filter(|s| s.instance == i)
            .map(|s| Disk::plain(s.snap))
            .collect()
    }
}

/// A disk to draw, context coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Disk {
    pub snap: Snap,
    /// Under the cursor, picked up or joined.
    pub hot: bool,
    /// Draw the rotation grip.
    pub grip: bool,
}

impl Disk {
    pub fn plain(snap: Snap) -> Self {
        Self {
            snap,
            hot: false,
            grip: false,
        }
    }

    pub fn hot(snap: Snap) -> Self {
        Self {
            hot: true,
            ..Self::plain(snap)
        }
    }
}

/// The world-space point of a snap's rotation grip.
fn grip_point(s: &Snap, radius: f64) -> DVec3 {
    s.position + s.align * radius
}

/// A change the tool makes outside a click (Alt reversing a snap).
pub type Edit = Box<dyn FnOnce(&mut Entities) -> bool>;

// ------------------------------------------------------------------ Snap placing

/// Where a moved or placed component goes when snapping: the join target and the new
/// instance transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Joined {
    pub xf: DMat4,
    pub target: Option<Snap>,
}

/// The instance (transform `inst_xf`) held by its snap `mine`, with the cursor at
/// `cur`: joined to a snap of another instance near the cursor (Alt: the nearest
/// anywhere), else attached to the face under the cursor (unit `face` normal turned
/// toward the viewer), else nothing. `skip` is the moved instance.
pub fn join(
    s: &SnapCtx,
    inst_xf: DMat4,
    mine: &Snap,
    cur: DVec3,
    face: Option<DVec3>,
    skip: Option<usize>,
) -> Option<Joined> {
    let other = |c: &ContextSnap| Some(c.instance) != skip;
    let target = if s.alt {
        s.nearest(cur, other)
    } else {
        s.under_cursor(JOIN_PX, other)
    };
    if let Some(t) = target {
        return Some(Joined {
            xf: mt_kernel::snaps::snap_to_snap(inst_xf, mine, &t.snap),
            target: Some(t.snap),
        });
    }
    let on_self = skip.is_some()
        && s.picked
            .is_some_and(|o| Some(o) == skip.map(Owner::Instance));
    let n = face.filter(|_| !on_self)?;
    Some(Joined {
        xf: mt_kernel::snaps::snap_to_face(inst_xf, mine, cur, n),
        target: None,
    })
}

/// The face normal under the cursor turned toward the viewer (`ray`: the view ray's
/// direction), `None` off faces.
pub fn facing_normal(cur: &Inference, ray: Option<DVec3>) -> Option<DVec3> {
    if cur.kind != mt_infer::Kind::OnFace {
        return None;
    }
    let n = cur.normal?.normalize_or_zero();
    if n == DVec3::ZERO {
        return None;
    }
    Some(match ray {
        Some(d) if d.dot(n) > 0.0 => -n,
        _ => n,
    })
}

// ------------------------------------------------------------------ Make/Edit Snaps

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Center,
    Grip,
}

/// The Make/Edit Snaps tool; it works on the open context's snaps.
#[derive(Debug, Default)]
pub struct EditSnaps {
    erase: bool,
    /// A new snap's center and the face normal there, waiting for its direction.
    placing: Option<(DVec3, Option<DVec3>)>,
    /// Arrow key: the new snap faces along this axis.
    lock: Option<DVec3>,
    grabbed: Option<(usize, Part)>,
    hot: Option<(usize, Part)>,
    own: Vec<Snap>,
    cur: Option<DVec3>,
    radius: f64,
    alt_was: bool,
    reverse: Option<usize>,
}

impl EditSnaps {
    fn direction(&self, p: DVec3, cur: DVec3, face: Option<DVec3>) -> DVec3 {
        if let Some(l) = self.lock {
            return l;
        }
        let d = cur - p;
        if d.length() > mt_kernel::TOL {
            d.normalize()
        } else {
            face.unwrap_or(DVec3::Z)
        }
    }
}

impl ModifyTool for EditSnaps {
    fn status(&self) -> String {
        if self.erase {
            return "Erase Snaps: click a snap's center to erase it. Ctrl = Modify.".into();
        }
        match (self.placing, self.grabbed) {
            (Some(_), _) => "Click to set the snap's direction. Arrow keys lock it to an axis.",
            (_, Some((_, Part::Center))) => "Click to place the snap.",
            (_, Some((_, Part::Grip))) => "Click to set the snap's Alignment Direction.",
            _ => {
                "Click to place a snap; click a snap's center to move it or its grip to turn it. Ctrl = Erase, Alt = reverse."
            }
        }
        .into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        self.placing.map(|p| p.0).or_else(|| {
            self.grabbed
                .and_then(|(i, _)| self.own.get(i).map(|s| s.position))
        })
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        let snaps = &mut ctx.ents.snaps;
        if self.erase {
            return match self.hot {
                Some((i, Part::Center)) if i < snaps.len() => {
                    snaps.remove(i);
                    self.hot = None;
                    true
                }
                _ => false,
            };
        }
        if let Some((p, face)) = self.placing.take() {
            let n = self.direction(p, inf.point, face);
            snaps.push(Snap::new(p, n));
            self.lock = None;
            return true;
        }
        if let Some((i, part)) = self.grabbed.take() {
            let Some(s) = snaps.get_mut(i) else {
                return false;
            };
            return match part {
                Part::Center => {
                    s.position = inf.point;
                    true
                }
                Part::Grip => s.set_align(inf.point - s.position),
            };
        }
        if self.hot.is_some() {
            self.grabbed = self.hot;
            return false;
        }
        self.placing = Some((inf.point, inf.normal));
        false
    }

    fn typed(
        &mut self,
        _ctx: &mut Ctx,
        _cur: Option<&Inference>,
        _text: &str,
        _units: LengthUnit,
    ) -> bool {
        false
    }

    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        match self.anchor() {
            Some(a) if self.placing.is_some() || self.grabbed.is_some() => vec![vec![a, cur.point]],
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.placing = None;
        self.grabbed = None;
        self.lock = None;
    }

    fn toggle_ctrl(&mut self) {
        self.erase = !self.erase;
        self.cancel();
    }

    fn arrow(&mut self, axis: mt_infer::Axis) -> bool {
        if self.placing.is_none() {
            return false;
        }
        let d = match axis {
            mt_infer::Axis::Red => DVec3::X,
            mt_infer::Axis::Green => DVec3::Y,
            mt_infer::Axis::Blue => DVec3::Z,
        };
        self.lock = if self.lock == Some(d) { None } else { Some(d) };
        true
    }

    fn input(&mut self, i: &crate::modify::Input) {
        self.cur = i.cur.map(|c| c.point);
    }

    fn snaps(&mut self, s: &SnapCtx) {
        self.own = s.ents.snaps.clone();
        self.radius = self.own.first().map_or(1.0, |x| (s.radius)(x.position));
        self.hot = None;
        if self.placing.is_none() && self.grabbed.is_none() {
            let mut best: Option<(f64, (usize, Part))> = None;
            for (i, x) in self.own.iter().enumerate() {
                let mut parts = vec![(x.position, Part::Center)];
                if !self.erase {
                    parts.push((grip_point(x, (s.radius)(x.position)), Part::Grip));
                }
                for (p, part) in parts {
                    if let Some(d) = s.px(p).filter(|d| *d <= PICK_PX)
                        && best.is_none_or(|b| d < b.0)
                    {
                        best = Some((d, (i, part)));
                    }
                }
            }
            self.hot = best.map(|b| b.1);
        }
        if s.alt && !self.alt_was && self.placing.is_none() && self.grabbed.is_none() {
            self.reverse = self.hot.map(|h| h.0);
        }
        self.alt_was = s.alt;
    }

    fn snap_disks(&self) -> Vec<Disk> {
        let mut out: Vec<Disk> = self
            .own
            .iter()
            .enumerate()
            .map(|(i, x)| Disk {
                snap: *x,
                hot: self.hot.is_some_and(|h| h.0 == i) || self.grabbed.is_some_and(|g| g.0 == i),
                grip: !self.erase,
            })
            .collect();
        if let (Some((p, face)), Some(c)) = (self.placing, self.cur) {
            out.push(Disk::hot(Snap::new(p, self.direction(p, c, face))));
        }
        if let (Some((i, part)), Some(c)) = (self.grabbed, self.cur)
            && let Some(x) = self.own.get(i)
        {
            let mut moved = *x;
            match part {
                Part::Center => moved.position = c,
                Part::Grip => {
                    moved.set_align(c - x.position);
                }
            }
            out.push(Disk {
                snap: moved,
                hot: true,
                grip: true,
            });
        }
        out
    }

    fn take_edit(&mut self) -> Option<Edit> {
        let i = self.reverse.take()?;
        Some(Box::new(move |ents: &mut Entities| {
            match ents.snaps.get_mut(i) {
                Some(s) => {
                    *s = s.reversed();
                    true
                }
                None => false,
            }
        }))
    }
}

// ------------------------------------------------------------------ App

impl App {
    /// Make/Edit Snaps from a menu: a selected group or component opens first.
    /// Returns false (and says why) when there is nothing to put snaps in.
    pub(crate) fn snaps_command(&mut self, device: &egui_wgpu::wgpu::Device) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        if let [Owner::Instance(i)] = self.selection.items()[..]
            && doc.open_instance(i)
        {
            self.selection.clear();
            self.rebuild(device);
            return true;
        }
        if self.document.as_ref().is_some_and(|d| d.context.is_empty()) {
            self.message =
                Some("Snaps are made inside a group or component: open one first.".into());
            return false;
        }
        true
    }

    /// Draws the open context's snaps and the disks of the running tool.
    pub(crate) fn snaps_paint(&self, ui: &egui::Ui, rect: egui::Rect, viewport_px: DVec2) {
        let Some(doc) = &self.document else {
            return;
        };
        let mut disks: Vec<Disk> = Vec::new();
        let editing = self.tool == Tool::Modify("tool.snaps");
        if !doc.context.is_empty() && !editing {
            disks.extend(doc.active().snaps.iter().map(|s| Disk::plain(*s)));
        }
        if let (Some(m), Tool::Modify(_)) = (&self.modify, self.tool) {
            disks.extend(m.snap_disks());
        }
        if disks.is_empty() {
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let (camera, xf) = (&self.camera, self.ctx_xf);
        let to_screen = |p: DVec3| {
            camera
                .project(p, viewport_px)
                .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
        };
        let painter = ui.painter().with_clip_rect(rect);
        for d in disks {
            let s = d.snap.transformed(xf);
            let r = disk_radius(camera, viewport_px, s.position);
            paint_disk(&painter, &to_screen, &s, r, camera.eye, d);
        }
    }
}

/// Each frame of a modify tool (after its hover): what it sees of snaps, and a change
/// it asks for outside a click, made as one undo step. Returns whether the model
/// changed (the caller rebuilds the scene).
pub(crate) struct Frame<'a> {
    pub camera: &'a mt_render::Camera,
    pub ctx_xf: DMat4,
    pub scene: &'a crate::document::Scene,
    pub bvh: &'a dyn mt_kernel::bvh::RayIndex,
    pub cursor: Option<DVec2>,
    pub viewport_px: DVec2,
    pub alt: bool,
}

pub(crate) fn frame(
    tool: &mut dyn ModifyTool,
    doc: Option<&mut crate::document::Document>,
    f: Frame,
) -> bool {
    let Some(doc) = doc else {
        return false;
    };
    let (camera, xf, viewport_px) = (f.camera, f.ctx_xf, f.viewport_px);
    let picked = f
        .cursor
        .and_then(|c| crate::selection::pick(f.scene, f.bvh, camera, c, viewport_px));
    let ents = doc.active();
    let ctx = SnapCtx {
        defs: &doc.model.definitions,
        ents,
        snaps: context_snaps(&doc.model.definitions, ents),
        project: &|p| camera.project(xf.transform_point3(p), viewport_px),
        cursor: f.cursor,
        picked,
        alt: f.alt,
        radius: &|p| disk_radius(camera, viewport_px, xf.transform_point3(p)),
    };
    tool.snaps(&ctx);
    match tool.take_edit() {
        Some(edit) => doc.edit(edit),
        None => false,
    }
}

/// The world radius of a disk drawn [`DISK_PX`] wide at world point `p`.
fn disk_radius(camera: &mt_render::Camera, viewport_px: DVec2, p: DVec3) -> f64 {
    let at_target = camera.units_per_px(viewport_px) * DISK_PX;
    let depth = (p - camera.eye).dot(camera.forward());
    let d = camera.distance();
    if camera.perspective && d > 1e-9 && depth > 0.0 {
        at_target * depth / d
    } else {
        at_target
    }
}

fn paint_disk(
    painter: &egui::Painter,
    to_screen: &dyn Fn(DVec3) -> Option<egui::Pos2>,
    s: &Snap,
    r: f64,
    eye: DVec3,
    d: Disk,
) {
    let a = s.align;
    let b = s.normal.cross(a);
    let ring: Option<Vec<egui::Pos2>> = (0..24)
        .map(|k| {
            let t = std::f64::consts::TAU * f64::from(k) / 24.0;
            to_screen(s.position + (a * t.cos() + b * t.sin()) * r)
        })
        .collect();
    let (Some(ring), Some(c)) = (ring, to_screen(s.position)) else {
        return;
    };
    // White front, blue back [doc]; front means facing the eye.
    let front = (eye - s.position).dot(s.normal) >= 0.0;
    let fill = if front {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 210)
    } else {
        egui::Color32::from_rgba_unmultiplied(70, 120, 230, 210)
    };
    let outline = if d.hot {
        egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 140, 0))
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_gray(60))
    };
    painter.add(egui::Shape::convex_polygon(ring, fill, outline));
    painter.circle_filled(c, 2.5, egui::Color32::from_gray(30));
    if let Some(tip) = to_screen(s.position + a * r) {
        painter.line_segment(
            [c, tip],
            egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
        );
        if d.grip {
            painter.rect(
                egui::Rect::from_center_size(tip, egui::vec2(6.0, 6.0)),
                0.0,
                egui::Color32::from_rgb(40, 180, 40),
                egui::Stroke::new(1.0, egui::Color32::BLACK),
                egui::StrokeKind::Middle,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inf(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: mt_infer::Kind::Free,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
            container: None,
        }
    }

    /// A pixel projection: x, y as pixels (top view, one unit per pixel).
    fn top(p: DVec3) -> Option<DVec2> {
        Some(DVec2::new(p.x, p.y))
    }

    fn frame(tool: &mut dyn ModifyTool, ents: &Entities, cursor: DVec3, alt: bool) {
        let defs = Definitions::with_key();
        let ctx = SnapCtx {
            defs: &defs,
            ents,
            snaps: Vec::new(),
            project: &top,
            cursor: Some(DVec2::new(cursor.x, cursor.y)),
            picked: None,
            alt,
            radius: &|_| 5.0,
        };
        tool.snaps(&ctx);
    }

    #[test]
    fn make_move_turn_reverse_and_erase_snaps() {
        let mut ents = Entities::new();
        let mut t = EditSnaps::default();
        let click = |t: &mut EditSnaps, ents: &mut Entities, p: DVec3| {
            frame(t, ents, p, false);
            let mut ctx = Ctx {
                ents,
                picked: None,
                selection: &[],
                ctrl: false,
                eye: DVec3::new(0.0, 0.0, 100.0),
            };
            t.click(&mut ctx, &inf(p))
        };
        // Place, then direction along +X.
        assert!(!click(&mut t, &mut ents, DVec3::ZERO));
        assert!(click(&mut t, &mut ents, DVec3::new(30.0, 0.0, 0.0)));
        assert_eq!(ents.snaps.len(), 1);
        assert!(ents.snaps[0].normal.distance(DVec3::X) < 1e-12);
        // Arrow lock: a second snap facing blue wherever the cursor is.
        assert!(!click(&mut t, &mut ents, DVec3::new(50.0, 50.0, 0.0)));
        assert!(t.arrow(mt_infer::Axis::Blue));
        assert!(click(&mut t, &mut ents, DVec3::new(80.0, 50.0, 0.0)));
        assert_eq!(ents.snaps[1].normal, DVec3::Z);
        // Move the first by its center.
        assert!(!click(&mut t, &mut ents, DVec3::new(1.0, 1.0, 0.0)));
        assert!(click(&mut t, &mut ents, DVec3::new(0.0, 20.0, 0.0)));
        assert_eq!(ents.snaps[0].position, DVec3::new(0.0, 20.0, 0.0));
        // Turn it by its grip (radius 5 along its alignment, blue: projects to the
        // center on the top view, so use the second snap whose alignment is red).
        let g = grip_point(&ents.snaps[1], 5.0);
        assert!(!click(&mut t, &mut ents, g));
        assert!(t.grabbed.is_some_and(|x| x.1 == Part::Grip));
        let to = ents.snaps[1].position + DVec3::Y * 9.0;
        assert!(click(&mut t, &mut ents, to));
        assert!(ents.snaps[1].align.distance(DVec3::Y) < 1e-12);
        // Alt over a center reverses the snap.
        frame(&mut t, &ents, DVec3::new(50.0, 50.0, 0.0), true);
        let edit = t.take_edit().unwrap();
        assert!(edit(&mut ents));
        assert_eq!(ents.snaps[1].normal, -DVec3::Z);
        // Erase mode.
        t.toggle_ctrl();
        assert!(click(&mut t, &mut ents, DVec3::new(0.0, 20.0, 0.0)));
        assert_eq!(ents.snaps.len(), 1);
    }

    #[test]
    fn move_picks_a_component_by_its_snap_and_joins_another() {
        use crate::modify::{Input, Kind, Transform};
        let mut defs = Definitions::with_key();
        let mut body = Entities::new();
        // A post with snaps at both ends.
        body.snaps = vec![
            Snap::new(DVec3::new(0.0, 0.0, 10.0), DVec3::Z),
            Snap::new(DVec3::ZERO, -DVec3::Z),
        ];
        let d = defs.insert(mt_kernel::Definition {
            entities: body,
            ..Default::default()
        });
        let mut ents = Entities::new();
        for x in [0.0, 50.0] {
            ents.instances.push(mt_kernel::Instance {
                definition: d,
                transform: DMat4::from_translation(DVec3::new(x, 0.0, 0.0)),
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
        }
        // Side view: x and z are pixels.
        let side = |p: DVec3| Some(DVec2::new(p.x, -p.z));
        let mut mv = Transform::new(Kind::Move);
        let step = |mv: &mut Transform, ents: &mut Entities, at: DVec3, click: bool| {
            let cur = inf(at);
            mv.input(&Input {
                held: crate::cursor::Mods {
                    ctrl: false,
                    shift: false,
                    alt: false,
                },
                press: false,
                ray: None,
                cur: Some(&cur),
                material: None,
            });
            let snaps = context_snaps(&defs, ents);
            mv.snaps(&SnapCtx {
                defs: &defs,
                ents,
                snaps,
                project: &side,
                cursor: side(at),
                picked: None,
                alt: false,
                radius: &|_| 2.0,
            });
            click && {
                let mut ctx = Ctx {
                    ents,
                    picked: None,
                    selection: &[],
                    ctrl: false,
                    eye: DVec3::new(0.0, -100.0, 0.0),
                };
                mv.click(&mut ctx, &cur)
            }
        };
        // Pick the second post by its bottom snap (cursor a few pixels off).
        step(&mut mv, &mut ents, DVec3::new(52.0, 0.0, 1.0), false);
        assert!(!step(&mut mv, &mut ents, DVec3::new(52.0, 0.0, 1.0), true));
        assert!(!mv.snap_disks().is_empty());
        // Near the first post's top snap: it stacks on it.
        step(&mut mv, &mut ents, DVec3::new(3.0, 0.0, 12.0), false);
        assert!(step(&mut mv, &mut ents, DVec3::new(3.0, 0.0, 12.0), true));
        let t = ents.instances[1].transform;
        assert!(
            t.transform_point3(DVec3::ZERO)
                .distance(DVec3::new(0.0, 0.0, 10.0))
                < 1e-9
        );
        assert!(t.transform_vector3(DVec3::Z).distance(DVec3::Z) < 1e-9);
    }

    #[test]
    fn join_prefers_a_near_snap_then_a_face() {
        let target = Snap::new(DVec3::new(100.0, 0.0, 0.0), -DVec3::X);
        let ents = Entities::new();
        let defs = Definitions::with_key();
        let mk = |alt: bool, cursor: DVec2| SnapCtx {
            defs: &defs,
            ents: &ents,
            snaps: vec![ContextSnap {
                instance: 1,
                local: target,
                snap: target,
            }],
            project: &top,
            cursor: Some(cursor),
            picked: None,
            alt,
            radius: &|_| 5.0,
        };
        let mine = Snap::new(DVec3::ZERO, DVec3::X);
        // Near the target: joined front to front.
        let s = mk(false, DVec2::new(95.0, 3.0));
        let j = join(
            &s,
            DMat4::IDENTITY,
            &mine,
            DVec3::new(95.0, 3.0, 0.0),
            None,
            Some(0),
        )
        .unwrap();
        assert_eq!(j.target, Some(target));
        let m = mine.transformed(j.xf);
        assert!(m.position.distance(target.position) < 1e-9);
        assert!(m.normal.distance(DVec3::X) < 1e-9);
        // Far, no face: nothing; with Alt: the nearest.
        let s = mk(false, DVec2::new(0.0, 0.0));
        assert!(join(&s, DMat4::IDENTITY, &mine, DVec3::ZERO, None, Some(0)).is_none());
        let s = mk(true, DVec2::new(0.0, 0.0));
        assert!(join(&s, DMat4::IDENTITY, &mine, DVec3::ZERO, None, Some(0)).is_some());
        // Over a face: attached to it.
        let s = mk(false, DVec2::new(0.0, 0.0));
        let j = join(
            &s,
            DMat4::IDENTITY,
            &mine,
            DVec3::new(0.0, 0.0, 5.0),
            Some(DVec3::Z),
            Some(0),
        )
        .unwrap();
        assert!(mine.transformed(j.xf).normal.distance(-DVec3::Z) < 1e-9);
        assert_eq!(j.target, None);
    }
}
