//! Modify tools: Push/Pull, Move, Rotate, Scale, Offset, Follow Me. They act on the
//! entity under the cursor or the selection, and take typed values like SketchUp
//! (distance, angle, factor, `5x` / `/5` arrays after a copy).
//!
//! Behaviors are drafts from memory of SketchUp (`spec/parity/STATUS.md`). Scale uses
//! SketchUp's grip box (corner, edge and face grips).
//!
//! While Move, Rotate or Scale runs, the app draws the targets live at the transformed
//! position ([`ModifyTool::live`]) and hides them at the original one (SketchUp moves
//! the real geometry; here connected geometry does not stretch until the click).

use glam::{DMat3, DMat4, DQuat, DVec2, DVec3};
use mt_infer::Inference;
use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::{EdgeId, Entities, FaceId};

use crate::document::Owner;
use crate::selection::OrientedBox;

/// What a modify tool can see and change.
pub struct Ctx<'a> {
    pub ents: &'a mut Entities,
    /// Top-level entity under the cursor.
    pub picked: Option<Owner>,
    pub selection: &'a [Owner],
    /// Ctrl held: copy (Move, Rotate), new faces (Push/Pull).
    pub ctrl: bool,
    /// Camera position in the context's space (which side of a face is clicked).
    pub eye: DVec3,
}

/// What a modify tool sees of the cursor before any click (Scale's grip hover).
pub struct Hover<'a> {
    pub ents: &'a Entities,
    pub selection: &'a [Owner],
    /// Definition-space bounds of a top-level instance of the context.
    pub instance_bounds: &'a dyn Fn(usize) -> Option<(DVec3, DVec3)>,
    /// Context space to viewport pixels.
    pub project: &'a dyn Fn(DVec3) -> Option<DVec2>,
    /// Viewport pixels.
    pub cursor: Option<DVec2>,
}

/// A point drawn over the view (Scale's grips); `hot` under the cursor or grabbed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub point: DVec3,
    pub hot: bool,
}

pub trait ModifyTool {
    fn status(&self) -> String;
    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String);
    /// Point that directions are inferred from.
    fn anchor(&self) -> Option<DVec3>;
    /// A direction the cursor is held to (Push/Pull follows the face normal).
    fn constraint(&self) -> Option<DVec3> {
        None
    }
    /// Returns whether the model changed.
    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool;
    fn typed(
        &mut self,
        ctx: &mut Ctx,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool;
    /// Polylines to preview.
    fn preview(&self, ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>>;
    fn cancel(&mut self);
    /// The material picked in the Materials panel (Paint Bucket).
    fn set_material(&mut self, _material: Option<usize>) {}
    /// Set Component Axes: the dialog and the picked axes once complete.
    fn take_axes(&mut self) -> Option<(crate::component_dialog::MakeComponent, DMat4)> {
        None
    }
    /// Ctrl pressed and released: toggles copy (Move, Rotate) or new faces
    /// (Push/Pull) for the rest of the tool's use (SketchUp keeps the toggle while the
    /// tool is active; unverified).
    fn toggle_ctrl(&mut self) {}
    /// A double click (after the click it also is). Returns whether the model changed.
    fn double_click(&mut self, _ctx: &mut Ctx, _inf: &Inference) -> bool {
        false
    }
    /// Called every frame before a click with what is under the cursor.
    fn hover(&mut self, _h: &Hover) {}
    /// Points to draw over the view (context space) for the cursor at `cur`.
    fn markers(&self, _cur: &Inference) -> Vec<Marker> {
        Vec::new()
    }
    /// The targets being transformed and the transform at the cursor (context space),
    /// to draw them live; `copy`: the originals stay where they are.
    fn live(&self, _cur: &Inference) -> Option<(&[Owner], DMat4, bool)> {
        None
    }
    /// After a change, the selection still names the changed entities (SketchUp keeps
    /// the selection after Move, Rotate and Scale).
    fn keeps_selection(&self) -> bool {
        false
    }
    /// After a change, the entities to select instead (Paste selects what it placed).
    fn take_selection(&mut self) -> Option<Vec<Owner>> {
        None
    }
    /// The tool is done: the app goes back to the Select tool (Paste after its click
    /// or Escape).
    fn finished(&self) -> bool {
        false
    }
    /// A ghost of entities that are not in the model yet, drawn following the cursor:
    /// the transform from the ghost's coordinates to the context's (Paste).
    fn ghost(&self, _cur: &Inference) -> Option<DMat4> {
        None
    }
}

pub fn for_command(id: &str) -> Option<Box<dyn ModifyTool>> {
    Some(match id {
        "tool.push_pull" => Box::new(PushPull::default()),
        "tool.move" => Box::new(Transform::new(Kind::Move)),
        "tool.rotate" => Box::new(Transform::new(Kind::Rotate)),
        "tool.scale" => Box::new(Scale::default()),
        "tool.offset" => Box::new(Offset::default()),
        "tool.follow_me" => Box::new(FollowMe),
        "tool.paint_bucket" => Box::new(PaintBucket::default()),
        "tool.section_plane" => Box::new(SectionPlaneTool),
        _ => return None,
    })
}

/// Edges and faces of the targets, plus instance indices.
fn split(targets: &[Owner]) -> (Vec<EdgeId>, Vec<FaceId>, Vec<usize>) {
    let (mut e, mut f, mut i) = (Vec::new(), Vec::new(), Vec::new());
    for t in targets {
        match *t {
            Owner::Edge(x) => e.push(x),
            Owner::Face(x) => f.push(x),
            Owner::Instance(x) => i.push(x),
            Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => {}
        }
    }
    (e, f, i)
}

/// The selection, or the entity under the cursor when nothing is selected (SketchUp's
/// hover-select for Move, Rotate, Scale).
fn targets(ctx: &Ctx) -> Vec<Owner> {
    if ctx.selection.is_empty() {
        ctx.picked.into_iter().collect()
    } else {
        ctx.selection.to_vec()
    }
}

/// Applies `xf` to the targets, or a transformed copy when `copy`.
fn apply(ents: &mut Entities, targets: &[Owner], xf: DMat4, copy: bool) {
    // Section planes move with the selection (or copy).
    for t in targets {
        if let Owner::Section(i) = *t
            && let Some(sp) = ents.section_planes.get(i).cloned()
        {
            let moved = mt_kernel::SectionPlane {
                point: xf.transform_point3(sp.point),
                normal: (glam::DMat3::from_mat4(xf).inverse().transpose() * sp.normal)
                    .normalize_or_zero(),
                ..sp
            };
            if copy {
                ents.section_planes.push(moved);
            } else {
                ents.section_planes[i] = moved;
            }
        }
    }
    crate::annot::transform(ents, targets, xf, copy);
    let (edges, faces, instances) = split(targets);
    if copy {
        mt_kernel::transform::copy_geometry(ents, &edges, &faces, xf);
        for i in instances {
            if let Some(inst) = ents.instances.get(i).cloned() {
                let mut c = inst;
                c.transform = xf * c.transform;
                ents.instances.push(c);
            }
        }
    } else {
        let verts = mt_kernel::transform::vertices_of(ents, &edges, &faces);
        mt_kernel::transform::transform_vertices(ents, &verts, xf);
        for i in instances {
            if let Some(inst) = ents.instances.get_mut(i) {
                inst.transform = xf * inst.transform;
            }
        }
    }
}

/// `5x` (more copies at the same spacing) or `/5` (divide the copy distance).
fn parse_array(text: &str) -> Option<(usize, bool)> {
    let t = text.trim();
    if let Some(n) = t
        .strip_suffix(['x', 'X', '*'])
        .or_else(|| t.strip_prefix(['x', 'X', '*']))
    {
        return n
            .trim()
            .parse()
            .ok()
            .filter(|n| *n >= 2)
            .map(|n| (n, false));
    }
    t.strip_prefix('/')
        .and_then(|n| n.trim().parse().ok())
        .filter(|n| *n >= 2)
        .map(|n| (n, true))
}

// ------------------------------------------------------------------ Push/Pull

#[derive(Debug, Default)]
pub struct PushPull {
    /// Face, the clicked point on it, its normal.
    start: Option<(FaceId, DVec3, DVec3)>,
    /// Ctrl toggled: keep the start face (new faces).
    new_faces: bool,
    /// Last distance, repeated on another face by a double click.
    last: Option<f64>,
}

impl PushPull {
    fn distance(&self, p: DVec3) -> Option<f64> {
        let (_, s, n) = self.start?;
        Some((p - s).dot(n))
    }

    fn finish(&mut self, ctx: &mut Ctx, distance: f64) -> bool {
        let Some((f, _, _)) = self.start.take() else {
            return false;
        };
        if distance.abs() <= mt_kernel::TOL || !ctx.ents.faces.contains_key(f) {
            return false;
        }
        self.last = Some(distance);
        let new_faces = self.new_faces || ctx.ctrl;
        mt_kernel::push_pull::push_pull(ctx.ents, f, distance, new_faces).is_some()
    }
}

impl ModifyTool for PushPull {
    fn status(&self) -> String {
        if self.start.is_some() {
            "Select distance or enter value."
        } else {
            "Select a face to push or pull."
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let v = cur
            .and_then(|c| self.distance(c.point))
            .map(|d| units.format(d));
        ("Distance", v.unwrap_or_default())
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(|s| s.1)
    }

    fn constraint(&self) -> Option<DVec3> {
        self.start.map(|s| s.2)
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        match self.start {
            None => {
                if let Some(Owner::Face(f)) = ctx.picked {
                    let n = ctx.ents.faces[f].normal;
                    self.start = Some((f, inf.point, n));
                }
                false
            }
            Some(_) => {
                let d = self.distance(inf.point).unwrap_or(0.0);
                self.finish(ctx, d)
            }
        }
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let Some(v) = parse_length(text, units) else {
            return false;
        };
        // The sign follows the side the cursor is on.
        let side = cur
            .and_then(|c| self.distance(c.point))
            .map_or(1.0, |d| if d < 0.0 { -1.0 } else { 1.0 });
        self.finish(ctx, v.abs() * side)
    }

    fn preview(&self, ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let (Some((f, _, n)), Some(d)) = (self.start, self.distance(cur.point)) else {
            return Vec::new();
        };
        let Some(face) = ents.faces.get(f) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for l in face.loops() {
            let base = ents.loop_points(l);
            let top: Vec<DVec3> = base.iter().map(|p| *p + n * d).collect();
            for (b, t) in base.iter().zip(&top) {
                out.push(vec![*b, *t]);
            }
            let mut ring = top.clone();
            ring.push(top[0]);
            out.push(ring);
        }
        out
    }

    fn cancel(&mut self) {
        self.start = None;
    }

    fn toggle_ctrl(&mut self) {
        self.new_faces = !self.new_faces;
    }

    /// SketchUp: double-clicking a face pushes or pulls it by the last distance.
    fn double_click(&mut self, ctx: &mut Ctx, _inf: &Inference) -> bool {
        let (Some(d), Some(Owner::Face(f))) = (self.last, ctx.picked) else {
            return false;
        };
        self.start = None;
        ctx.ents.faces.contains_key(f)
            && mt_kernel::push_pull::push_pull(ctx.ents, f, d, self.new_faces || ctx.ctrl).is_some()
    }
}

// ------------------------------------------------------------------ Move, Rotate

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
}

/// Move: base point, destination. Rotate: center, start direction, end direction.
#[derive(Debug)]
pub struct Transform {
    kind: Kind,
    points: Vec<DVec3>,
    targets: Vec<Owner>,
    /// Ctrl at the first click: copy instead of move.
    copy: bool,
    /// The last copy, for `5x` / `/5` arrays: transform per step and copy count.
    last_copy: Option<(Vec<Owner>, DMat4)>,
}

impl Transform {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            points: Vec::new(),
            targets: Vec::new(),
            copy: false,
            last_copy: None,
        }
    }

    fn needed(&self) -> usize {
        match self.kind {
            Kind::Move => 2,
            Kind::Rotate => 3,
        }
    }

    /// The transform for the points so far plus `p`.
    fn xf(&self, p: DVec3) -> Option<DMat4> {
        match (self.kind, &self.points[..]) {
            (Kind::Move, [a]) => Some(DMat4::from_translation(p - *a)),
            (Kind::Rotate, [c, s]) => {
                let (u, w) = ((*s - *c).normalize_or_zero(), (p - *c).normalize_or_zero());
                if u == DVec3::ZERO || w == DVec3::ZERO {
                    return None;
                }
                let axis = u.cross(w);
                let axis = if axis.length() < 1e-9 {
                    DVec3::Z
                } else {
                    axis.normalize()
                };
                let angle = u.dot(w).clamp(-1.0, 1.0).acos();
                Some(rotation_about(*c, axis, angle))
            }
            _ => None,
        }
    }

    fn finish(&mut self, ctx: &mut Ctx, xf: DMat4) -> bool {
        let targets = std::mem::take(&mut self.targets);
        self.points.clear();
        if targets.is_empty() {
            return false;
        }
        let copy = self.copy || ctx.ctrl;
        apply(ctx.ents, &targets, xf, copy);
        self.last_copy = copy.then_some((targets, xf));
        true
    }
}

fn rotation_about(c: DVec3, axis: DVec3, angle: f64) -> DMat4 {
    DMat4::from_translation(c)
        * DMat4::from_quat(DQuat::from_axis_angle(axis, angle))
        * DMat4::from_translation(-c)
}

/// Scales by `s` along the unit directions `dirs` (columns of a basis) about `c`.
fn scale_in_axes(c: DVec3, dirs: [DVec3; 3], s: DVec3) -> DMat4 {
    let b = DMat3::from_cols(dirs[0], dirs[1], dirs[2]);
    let m = b * DMat3::from_diagonal(s) * b.inverse();
    DMat4::from_translation(c) * DMat4::from_mat3(m) * DMat4::from_translation(-c)
}

impl ModifyTool for Transform {
    fn status(&self) -> String {
        let text: String = match (self.kind, self.points.len()) {
            (Kind::Move, 0) => "Select object to move. Ctrl = toggle copy.",
            (Kind::Move, _) => {
                "Select destination or enter distance. After a copy: 5x or /5 for arrays."
            }
            (Kind::Rotate, 0) => "Select center of rotation. Ctrl = toggle copy.",
            (Kind::Rotate, 1) => "Select start point of the angle.",
            (Kind::Rotate, _) => "Select end point or enter angle.",
        }
        .into();
        if self.copy {
            format!("{text} (Copy)")
        } else {
            text
        }
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let p = cur.map(|c| c.point);
        match (self.kind, &self.points[..], p) {
            (Kind::Move, [a], Some(p)) => ("Distance", units.format(a.distance(p))),
            (Kind::Rotate, [c, s], Some(p)) => {
                let (u, w) = ((*s - *c).normalize_or_zero(), (p - *c).normalize_or_zero());
                (
                    "Angle",
                    format!("{:.1}", u.dot(w).clamp(-1.0, 1.0).acos().to_degrees()),
                )
            }
            (Kind::Move, ..) => ("Distance", String::new()),
            (Kind::Rotate, ..) => ("Angle", String::new()),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        self.points.first().copied()
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        if self.points.is_empty() {
            self.targets = targets(ctx);
            if self.targets.is_empty() {
                return false;
            }
            self.last_copy = None;
        }
        if self.points.len() + 1 < self.needed() {
            if self
                .points
                .last()
                .is_none_or(|q| q.distance(inf.point) > mt_kernel::TOL)
            {
                self.points.push(inf.point);
            }
            return false;
        }
        match self.xf(inf.point) {
            Some(xf) => self.finish(ctx, xf),
            None => false,
        }
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        // Arrays after a copy.
        if self.points.is_empty()
            && let (Some((n, divide)), Some((targets, xf))) =
                (parse_array(text), self.last_copy.clone())
        {
            let step = if divide {
                fractional(xf, 1.0 / n as f64, self.kind)
            } else {
                xf
            };
            if divide {
                // Replace the single copy at the full step by n copies at fractions:
                // the existing copy stays at the end, add the n - 1 in between.
                for k in 1..n {
                    apply(
                        ctx.ents,
                        &targets,
                        fractional(xf, k as f64 / n as f64, self.kind),
                        true,
                    );
                }
            } else {
                let mut acc = step;
                for _ in 1..n {
                    acc = step * acc;
                    apply(ctx.ents, &targets, acc, true);
                }
            }
            self.last_copy = None;
            return true;
        }
        let Some(p) = cur.map(|c| c.point) else {
            return false;
        };
        let xf = match (self.kind, &self.points[..]) {
            (Kind::Move, [a]) => {
                let Some(d) = parse_length(text, units) else {
                    return false;
                };
                let dir = (p - *a).normalize_or_zero();
                (dir != DVec3::ZERO).then(|| DMat4::from_translation(dir * d))
            }
            (Kind::Rotate, [c, s]) => {
                let Ok(deg) = text.trim().trim_end_matches('°').parse::<f64>() else {
                    return false;
                };
                let (u, w) = ((*s - *c).normalize_or_zero(), (p - *c).normalize_or_zero());
                let axis = u.cross(w);
                let axis = if axis.length() < 1e-9 {
                    DVec3::Z
                } else {
                    axis.normalize()
                };
                Some(rotation_about(*c, axis, deg.to_radians()))
            }
            _ => None,
        };
        match xf {
            Some(xf) => self.finish(ctx, xf),
            None => false,
        }
    }

    /// The rubber band; the targets themselves are drawn live (see [`Self::live`]).
    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let mut guide = self.points.clone();
        guide.push(cur.point);
        if guide.len() > 1 {
            vec![guide]
        } else {
            Vec::new()
        }
    }

    fn toggle_ctrl(&mut self) {
        self.copy = !self.copy;
    }

    fn cancel(&mut self) {
        self.points.clear();
        self.targets.clear();
    }

    fn live(&self, cur: &Inference) -> Option<(&[Owner], DMat4, bool)> {
        if self.targets.is_empty() || self.points.len() + 1 != self.needed() {
            return None;
        }
        Some((&self.targets, self.xf(cur.point)?, self.copy))
    }

    /// Moved entities stay selected; copies are new entities, so the selection goes.
    fn keeps_selection(&self) -> bool {
        self.last_copy.is_none()
    }
}

/// A fraction of a move or rotation (for `/n` arrays).
fn fractional(xf: DMat4, t: f64, kind: Kind) -> DMat4 {
    let (s, r, tr) = xf.to_scale_rotation_translation();
    match kind {
        Kind::Move => DMat4::from_translation(tr * t),
        _ => {
            let (axis, angle) = r.to_axis_angle();
            // Rotation about the same center: the center is the fixed point of xf.
            let _ = s;
            let center = fixed_point(xf);
            rotation_about(center, axis, angle * t)
        }
    }
}

/// A point the rigid transform leaves in place (least squares on the rotation plane).
fn fixed_point(xf: DMat4) -> DVec3 {
    let (_, r, t) = xf.to_scale_rotation_translation();
    let (axis, _) = r.to_axis_angle();
    // Solve (I - R) c = t in the plane perpendicular to the axis.
    let m = glam::DMat3::IDENTITY - glam::DMat3::from_quat(r);
    let m = m + glam::DMat3::from_cols(axis * axis.x, axis * axis.y, axis * axis.z);
    m.inverse() * t
}

/// `1.5` (uniform) or `2,1,1` (per axis).
fn parse_factors(text: &str) -> Option<DVec3> {
    let parts: Vec<f64> = text
        .split([',', ';'])
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let v = match parts[..] {
        [s] => DVec3::splat(s),
        [x, y, z] => DVec3::new(x, y, z),
        _ => return None,
    };
    (v.abs().min_element() > 1e-9).then_some(v)
}

// ------------------------------------------------------------------ Scale

/// Grip pick radius in viewport pixels (SketchUp's is unmeasured; draft).
pub const GRIP_RADIUS_PX: f64 = 8.0;

/// Box coordinates of a grip: -1, 0 or 1 per box axis. Corners have three non-zero
/// coordinates, edge midpoints two and face centers one.
pub type Grip = [i8; 3];

fn grip_vec(g: Grip) -> DVec3 {
    DVec3::new(g[0].into(), g[1].into(), g[2].into())
}

/// The grips of `b`: corners, edge midpoints and face centers. Grips that would scale a
/// flat dimension are left out (a face shows eight grips in its plane, as in SketchUp).
pub fn grips(b: &OrientedBox) -> Vec<Grip> {
    let flat = |i: usize| b.half[i] <= mt_kernel::TOL;
    let mut out = Vec::new();
    for x in -1i8..=1 {
        for y in -1i8..=1 {
            for z in -1i8..=1 {
                let g = [x, y, z];
                if g == [0, 0, 0] || (0..3).any(|i| g[i] != 0 && flat(i)) {
                    continue;
                }
                out.push(g);
            }
        }
    }
    out
}

/// The grip of `b` nearest to `cursor` on screen within `radius` pixels.
pub fn pick_grip(
    b: &OrientedBox,
    project: &dyn Fn(DVec3) -> Option<DVec2>,
    cursor: DVec2,
    radius: f64,
) -> Option<Grip> {
    grips(b)
        .into_iter()
        .filter_map(|g| Some((g, project(b.at(grip_vec(g)))?.distance(cursor))))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(g, _)| g)
}

/// The box the Scale tool shows around `targets`: a single component or group's box in
/// its own axes, the box along the context's axes otherwise.
pub fn grip_box(
    ents: &Entities,
    targets: &[Owner],
    instance_bounds: &dyn Fn(usize) -> Option<(DVec3, DVec3)>,
) -> Option<OrientedBox> {
    let instance_box = |i: usize| {
        let inst = ents.instances.get(i)?;
        let (min, max) = instance_bounds(i)?;
        Some(OrientedBox::from_aabb(min, max).transformed(inst.transform))
    };
    if let [Owner::Instance(i)] = targets {
        return instance_box(*i);
    }
    let mut pts = Vec::new();
    for t in targets {
        match *t {
            Owner::Edge(e) => {
                if let Some(e) = ents.edges.get(e) {
                    pts.extend([ents.pos(e.start), ents.pos(e.end)]);
                }
            }
            Owner::Face(f) => {
                if let Some(f) = ents.faces.get(f) {
                    pts.extend(f.loops().flat_map(|l| ents.loop_points(l)));
                }
            }
            Owner::Instance(i) => pts.extend(instance_box(i).iter().flat_map(|b| b.corners())),
            Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => {}
        }
    }
    OrientedBox::around(pts)
}

/// A grabbed grip of a box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grab {
    pub grip: Grip,
    pub bbox: OrientedBox,
}

/// A typed scale value: a factor, or a length (the size wanted).
#[derive(Debug, Clone, Copy, PartialEq)]
enum ScaleValue {
    Factor(f64),
    Length(f64),
}

/// `2`, `2,1,1`, `200mm` or `200mm,1m,1` (plain numbers are factors).
fn parse_scale_values(text: &str, units: LengthUnit) -> Option<Vec<ScaleValue>> {
    if let Some(v) = parse_factors(text) {
        let mut out = vec![ScaleValue::Factor(v.x)];
        if text.contains([',', ';']) {
            out = v.to_array().map(ScaleValue::Factor).to_vec();
        }
        return Some(out);
    }
    text.split([',', ';'])
        .map(|p| match p.trim().parse::<f64>() {
            Ok(f) => Some(ScaleValue::Factor(f)),
            Err(_) => parse_length(p.trim(), units).map(ScaleValue::Length),
        })
        .collect()
}

impl Grab {
    /// Box axes the grip scales.
    fn axes(&self) -> Vec<usize> {
        (0..3).filter(|&i| self.grip[i] != 0).collect()
    }

    /// Corner grips (all of the box's non-flat axes) scale uniformly; edge and face
    /// grips scale their axes independently.
    pub fn uniform(&self) -> bool {
        let solid = (0..3)
            .filter(|&i| self.bbox.half[i] > mt_kernel::TOL)
            .count();
        let n = self.axes().len();
        n >= 2 && n == solid
    }

    /// The grip's position.
    pub fn handle(&self) -> DVec3 {
        self.bbox.at(grip_vec(self.grip))
    }

    /// The point that stays: the opposite grip, or the center.
    pub fn fixed(&self, about_center: bool) -> DVec3 {
        if about_center {
            self.bbox.center
        } else {
            self.bbox.at(-grip_vec(self.grip))
        }
    }

    pub fn xf(&self, s: DVec3, about_center: bool) -> DMat4 {
        scale_in_axes(self.fixed(about_center), self.bbox.dirs, s)
    }

    /// Factors (per box axis) that take the grip to `p`.
    pub fn factors_at(&self, p: DVec3, about_center: bool) -> Option<DVec3> {
        let f = self.fixed(about_center);
        let (r, d) = (self.handle() - f, p - f);
        if self.uniform() {
            let rr = r.length_squared();
            return (rr > mt_kernel::TOL * mt_kernel::TOL).then(|| DVec3::splat(d.dot(r) / rr));
        }
        let mut s = DVec3::ONE;
        for i in self.axes() {
            let a = self.bbox.dirs[i];
            let ra = r.dot(a);
            if ra.abs() <= mt_kernel::TOL {
                return None;
            }
            s[i] = d.dot(a) / ra;
        }
        Some(s)
    }

    /// Factors for typed values: one value for the grip's axes (uniform for a corner),
    /// one per scaled axis, or three (red, green, blue box axes). A length is the size
    /// wanted along the axis (along the box diagonal for a uniform value; unverified).
    fn factors_typed(&self, vals: &[ScaleValue]) -> Option<DVec3> {
        let size = |i: usize| 2.0 * self.bbox.half[i];
        let factor = |v: ScaleValue, size: f64| match v {
            ScaleValue::Factor(f) => Some(f),
            ScaleValue::Length(l) => (size > mt_kernel::TOL).then(|| l / size),
        };
        let axes = self.axes();
        let mut s = DVec3::ONE;
        match vals {
            [v] if self.uniform() => {
                let diag = axes.iter().map(|&i| size(i).powi(2)).sum::<f64>().sqrt();
                s = DVec3::splat(factor(*v, diag)?);
            }
            [v] => {
                for &i in &axes {
                    s[i] = factor(*v, size(i))?;
                }
            }
            _ if vals.len() == axes.len() && !self.uniform() => {
                for (&i, v) in axes.iter().zip(vals) {
                    s[i] = factor(*v, size(i))?;
                }
            }
            [x, y, z] => {
                for (i, v) in [x, y, z].into_iter().enumerate() {
                    s[i] = factor(*v, size(i))?;
                }
            }
            _ => return None,
        }
        (s.abs().min_element() > 1e-9).then_some(s)
    }
}

/// SketchUp's Scale tool: the selection (or the clicked entity) gets a box with grips;
/// dragging a grip (or click, move, click) scales about the opposite grip, or about the
/// center after Ctrl. Corner grips scale uniformly, edge grips two axes, face grips one.
/// Typed: `2`, `2,1,1` or sizes like `200mm` (also right after a scale, to change it).
/// Differences from SketchUp: Ctrl toggles instead of being held, Shift (uniform /
/// non-uniform swap) is not implemented, and the box is not drawn with SketchUp's yellow.
#[derive(Debug, Default)]
pub struct Scale {
    /// Targets clicked while nothing is selected.
    own: Vec<Owner>,
    /// The targets the box is around.
    targets: Vec<Owner>,
    bbox: Option<OrientedBox>,
    /// Grip under the cursor.
    hot: Option<Grip>,
    grab: Option<Grab>,
    about_center: bool,
    /// The last scale (targets, grip, transform): a typed value replaces it.
    last: Option<(Vec<Owner>, Grab, DMat4)>,
}

impl Scale {
    fn finish(&mut self, ctx: &mut Ctx, grab: Grab, s: DVec3) -> bool {
        self.grab = None;
        if self.targets.is_empty() {
            return false;
        }
        let xf = grab.xf(s, self.about_center);
        apply(ctx.ents, &self.targets, xf, false);
        self.last = Some((self.targets.clone(), grab, xf));
        true
    }
}

impl ModifyTool for Scale {
    fn status(&self) -> String {
        let text = match (&self.grab, &self.bbox) {
            (Some(_), _) => "Select second point or enter scale value (2, 2,1,1 or 200mm).",
            (None, Some(_)) => "Select a grip to scale. Ctrl = toggle scale about center.",
            (None, None) => "Select an entity to scale.",
        };
        if self.about_center {
            format!("{text} (About center)")
        } else {
            text.into()
        }
    }

    fn measurement(&self, cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        let (Some(g), Some(c)) = (&self.grab, cur) else {
            return ("Scale", String::new());
        };
        let Some(s) = g.factors_at(c.point, self.about_center) else {
            return ("Scale", String::new());
        };
        let text = if g.uniform() {
            format!("{:.2}", s.x)
        } else {
            g.axes()
                .iter()
                .map(|&i| format!("{:.2}", s[i]))
                .collect::<Vec<_>>()
                .join(",")
        };
        ("Scale", text)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.grab.map(|g| g.handle())
    }

    fn hover(&mut self, h: &Hover) {
        if self.grab.is_some() {
            return;
        }
        if !h.selection.is_empty() {
            self.own.clear();
        }
        self.targets = if h.selection.is_empty() {
            self.own
                .iter()
                .copied()
                .filter(|o| crate::selection::exists(h.ents, o))
                .collect()
        } else {
            h.selection.to_vec()
        };
        self.bbox = grip_box(h.ents, &self.targets, h.instance_bounds);
        self.hot = match (self.bbox, h.cursor) {
            (Some(b), Some(c)) => pick_grip(&b, h.project, c, GRIP_RADIUS_PX),
            _ => None,
        };
    }

    /// The grips, on the scaled box while one is held.
    fn markers(&self, cur: &Inference) -> Vec<Marker> {
        let Some(b) = self.bbox else {
            return Vec::new();
        };
        let b = match self.live(cur) {
            Some((_, xf, _)) => b.transformed(xf),
            None => b,
        };
        let hot = self.grab.map(|g| g.grip).or(self.hot);
        grips(&b)
            .into_iter()
            .map(|g| Marker {
                point: b.at(grip_vec(g)),
                hot: Some(g) == hot,
            })
            .collect()
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        if let Some(g) = self.grab {
            return match g.factors_at(inf.point, self.about_center) {
                Some(s) if s.abs().min_element() > 1e-9 => self.finish(ctx, g, s),
                _ => false,
            };
        }
        if let (Some(grip), Some(bbox)) = (self.hot, self.bbox) {
            self.grab = Some(Grab { grip, bbox });
            self.last = None;
            return false;
        }
        if ctx.selection.is_empty() {
            self.own = ctx.picked.into_iter().collect();
        }
        false
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        _cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let Some(vals) = parse_scale_values(text, units) else {
            return false;
        };
        if let Some(g) = self.grab {
            return match g.factors_typed(&vals) {
                Some(s) => self.finish(ctx, g, s),
                None => false,
            };
        }
        // Right after a scale: the value replaces it.
        if let Some((targets, g, prev)) = &mut self.last
            && let Some(s) = g.factors_typed(&vals)
        {
            let xf = g.xf(s, self.about_center);
            apply(ctx.ents, targets, xf * prev.inverse(), false);
            *prev = xf;
            return true;
        }
        false
    }

    /// The box (scaled with the targets while a grip is held).
    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let Some(b) = self.bbox else {
            return Vec::new();
        };
        let b = match self.live(cur) {
            Some((_, xf, _)) => b.transformed(xf),
            None => b,
        };
        b.edges()
            .into_iter()
            .filter(|[p, q]| p.distance(*q) > mt_kernel::TOL)
            .map(|e| e.to_vec())
            .collect()
    }

    fn cancel(&mut self) {
        if self.grab.take().is_none() {
            self.own.clear();
        }
    }

    fn toggle_ctrl(&mut self) {
        self.about_center = !self.about_center;
    }

    fn live(&self, cur: &Inference) -> Option<(&[Owner], DMat4, bool)> {
        let g = self.grab?;
        let s = g
            .factors_at(cur.point, self.about_center)
            .filter(|s| s.abs().min_element() > 1e-9)?;
        Some((&self.targets, g.xf(s, self.about_center), false))
    }

    fn keeps_selection(&self) -> bool {
        true
    }
}

// ------------------------------------------------------------------ Offset

#[derive(Debug, Default)]
pub struct Offset {
    start: Option<(FaceId, DVec3)>,
}

impl Offset {
    /// Inward distance for the cursor: positive inside the face.
    fn distance(&self, ents: &Entities, p: DVec3) -> Option<f64> {
        let (f, s) = self.start?;
        ents.faces.get(f)?;
        let plane = ents.face_plane(f);
        let q = p - plane.normal * plane.distance(p);
        let d = q.distance(s);
        Some(if ents.face_contains(f, q) { d } else { -d })
    }

    fn finish(&mut self, ctx: &mut Ctx, d: f64) -> bool {
        let Some((f, _)) = self.start.take() else {
            return false;
        };
        ctx.ents.faces.contains_key(f)
            && d.abs() > mt_kernel::TOL
            && !mt_kernel::offset::offset_face(ctx.ents, f, d).is_empty()
    }
}

impl ModifyTool for Offset {
    fn status(&self) -> String {
        if self.start.is_some() {
            "Select offset distance or enter value."
        } else {
            "Select a face to offset."
        }
        .into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("Distance", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(|s| s.1)
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        match self.start {
            None => {
                if let Some(Owner::Face(f)) = ctx.picked {
                    self.start = Some((f, inf.point));
                }
                false
            }
            Some(_) => {
                let d = self.distance(ctx.ents, inf.point).unwrap_or(0.0);
                self.finish(ctx, d)
            }
        }
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let Some(v) = parse_length(text, units) else {
            return false;
        };
        let side = cur
            .and_then(|c| self.distance(ctx.ents, c.point))
            .map_or(1.0, |d| if d < 0.0 { -1.0 } else { 1.0 });
        self.finish(ctx, v.abs() * side)
    }

    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        self.start
            .map(|(_, s)| vec![vec![s, cur.point]])
            .unwrap_or_default()
    }

    fn cancel(&mut self) {
        self.start = None;
    }
}

// ------------------------------------------------------------------ Follow Me

/// Select the path edges first, then click the profile face.
#[derive(Debug)]
pub struct FollowMe;

impl ModifyTool for FollowMe {
    fn status(&self) -> String {
        "Select a profile face to extrude along the selected path.".into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, _inf: &Inference) -> bool {
        let Some(Owner::Face(profile)) = ctx.picked else {
            return false;
        };
        let path: Vec<EdgeId> = ctx
            .selection
            .iter()
            .filter_map(|o| match o {
                Owner::Edge(e) => Some(*e),
                _ => None,
            })
            .collect();
        // A selected face as path: follow its outer loop.
        let path = if path.is_empty() {
            match ctx.selection.iter().find_map(|o| match o {
                Owner::Face(f) if *f != profile => Some(*f),
                _ => None,
            }) {
                Some(f) => ctx.ents.faces[f]
                    .outer
                    .coedges
                    .iter()
                    .map(|c| c.edge)
                    .collect(),
                None => return false,
            }
        } else {
            path
        };
        !mt_kernel::follow_me::follow_me(ctx.ents, profile, &path).is_empty()
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

    fn preview(&self, _ents: &Entities, _cur: &Inference) -> Vec<Vec<DVec3>> {
        Vec::new()
    }

    fn cancel(&mut self) {}
}

// ------------------------------------------------------------------ Paint Bucket

/// Paints the clicked face side, or the clicked group or component. When the clicked
/// entity is selected, the whole selection is painted. Alt-sampling and Ctrl/Shift fill
/// modes are not implemented yet.
#[derive(Debug, Default)]
pub struct PaintBucket {
    material: Option<usize>,
}

impl ModifyTool for PaintBucket {
    fn status(&self) -> String {
        if self.material.is_some() {
            "Select a face or object to paint."
        } else {
            "Pick a material in the Materials panel, then select a face or object to paint."
        }
        .into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        let Some(picked) = ctx.picked else {
            return false;
        };
        let targets: Vec<Owner> = if ctx.selection.contains(&picked) {
            ctx.selection.to_vec()
        } else {
            vec![picked]
        };
        let m = self.material;
        let mut changed = false;
        for t in targets {
            match t {
                Owner::Face(f) => {
                    let Some(face) = ctx.ents.faces.get_mut(f) else {
                        continue;
                    };
                    // The side facing the camera.
                    let front = (ctx.eye - inf.point).dot(face.normal) >= 0.0 || t != picked;
                    let slot = if front {
                        &mut face.attrs.material
                    } else {
                        &mut face.attrs.back_material
                    };
                    changed |= *slot != m;
                    *slot = m;
                }
                Owner::Instance(i) => {
                    if let Some(inst) = ctx.ents.instances.get_mut(i) {
                        changed |= inst.material != m;
                        inst.material = m;
                    }
                }
                Owner::Edge(_) | Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => {}
            }
        }
        changed
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

    fn preview(&self, _ents: &Entities, _cur: &Inference) -> Vec<Vec<DVec3>> {
        Vec::new()
    }

    fn cancel(&mut self) {}

    fn set_material(&mut self, material: Option<usize>) {
        self.material = material;
    }
}

// ------------------------------------------------------------------ Place Component

/// Places instances of a component picked in the Components panel at clicked points
/// (SketchUp glues the component to the cursor at its origin; rotating with the
/// cursor is not implemented). A definition that glues lies in the clicked face's
/// plane when the face accepts it.
#[derive(Debug)]
pub struct PlaceComponent {
    pub definition: mt_kernel::DefinitionId,
    /// The definition's "Glue to": on an accepting face the instance lies in its plane.
    pub glue: mt_kernel::GlueTo,
}

impl ModifyTool for PlaceComponent {
    fn status(&self) -> String {
        "Select a point to place the component.".into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        let transform = match inf.normal {
            Some(n) if self.glue.accepts(n) => crate::component_dialog::glued_frame(inf.point, n),
            _ => DMat4::from_translation(inf.point),
        };
        ctx.ents.instances.push(mt_kernel::Instance {
            definition: self.definition,
            transform,
            name: String::new(),
            hidden: false,
            layer: None,
            material: None,
        });
        true
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
        // A small axis cross at the insertion point.
        let p = cur.point;
        [DVec3::X, DVec3::Y, DVec3::Z]
            .iter()
            .map(|a| vec![p - *a * 6.0, p + *a * 6.0])
            .collect()
    }

    fn cancel(&mut self) {}
}

// ------------------------------------------------------------------ Paste

/// Edit > Paste: the clipboard follows the cursor, held at its grab point, and a click
/// places it into the open context (merging loose geometry like drawing does). The
/// placed entities become the selection and the Select tool comes back; Escape cancels.
///
/// The grab point is the minimum corner of the clipboard's bounding box (unverified:
/// SketchUp's grab point is to be checked in the trial). Going back to the Select tool
/// afterwards is also unverified.
#[derive(Debug)]
pub struct Paste {
    /// The clipboard, in world coordinates.
    clip: Entities,
    /// World-space point of the clipboard held at the cursor.
    grab: DVec3,
    /// The open context's transform (context to world) when the paste started.
    ctx_xf: DMat4,
    placed: Option<Vec<Owner>>,
    done: bool,
}

impl Paste {
    pub fn new(clip: Entities, grab: DVec3, ctx_xf: DMat4) -> Self {
        Self {
            clip,
            grab,
            ctx_xf,
            placed: None,
            done: false,
        }
    }

    /// Clipboard (world) to context space with the grab point at context point `p`.
    fn xf(&self, p: DVec3) -> DMat4 {
        let target = self.ctx_xf.transform_point3(p);
        self.ctx_xf.inverse() * DMat4::from_translation(target - self.grab)
    }
}

/// The entities a paste created, as selectable owners.
pub fn pasted_owners(p: &mt_kernel::clipboard::Pasted) -> Vec<Owner> {
    p.edges
        .iter()
        .map(|&e| Owner::Edge(e))
        .chain(p.faces.iter().map(|&f| Owner::Face(f)))
        .chain(p.instances.iter().map(|&i| Owner::Instance(i)))
        .chain(p.dimensions.iter().map(|&i| Owner::Dimension(i)))
        .chain(p.texts.iter().map(|&i| Owner::Text(i)))
        .collect()
}

impl ModifyTool for Paste {
    fn status(&self) -> String {
        // Draft wording (unverified).
        "Select a point to place the pasted entities. Esc = cancel.".into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        if self.done {
            return false;
        }
        let xf = self.xf(inf.point);
        let pasted = mt_kernel::clipboard::paste(ctx.ents, &self.clip, xf, |_| true);
        self.placed = Some(pasted_owners(&pasted));
        self.done = true;
        true
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

    fn preview(&self, _ents: &Entities, _cur: &Inference) -> Vec<Vec<DVec3>> {
        Vec::new()
    }

    fn cancel(&mut self) {
        self.done = true;
    }

    fn take_selection(&mut self) -> Option<Vec<Owner>> {
        self.placed.take()
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn ghost(&self, cur: &Inference) -> Option<DMat4> {
        (!self.done).then(|| self.xf(cur.point))
    }
}

// ------------------------------------------------------------------ Section Plane

/// Places a section plane on the face under the cursor (facing the way the face does),
/// or horizontally elsewhere, and makes it the active cut. SketchUp 2026 asks for a
/// name and symbol first (not implemented); arrow-key orientation locks are missing.
#[derive(Debug)]
pub struct SectionPlaneTool;

impl SectionPlaneTool {
    fn normal(inf: &Inference) -> DVec3 {
        inf.normal.filter(|n| n.length() > 0.5).unwrap_or(DVec3::Z)
    }
}

impl ModifyTool for SectionPlaneTool {
    fn status(&self) -> String {
        "Select a face to place the section plane on.".into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        let n = ctx.ents.section_planes.len() + 1;
        ctx.ents.section_planes.push(mt_kernel::SectionPlane {
            point: inf.point,
            normal: Self::normal(inf),
            name: format!("Section Plane {n}"),
            hidden: false,
        });
        ctx.ents.active_section = Some(ctx.ents.section_planes.len() - 1);
        true
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
        let n = Self::normal(cur);
        let (u, v) = n.any_orthonormal_pair();
        let c = cur.point;
        let h = 20.0;
        let ring: Vec<DVec3> = [
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
            (-1.0, -1.0),
        ]
        .iter()
        .map(|(a, b)| c + (u * *a + v * *b) * h)
        .collect();
        vec![ring, vec![c, c + n * h * 0.5]]
    }

    fn cancel(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_and_factors() {
        assert_eq!(parse_array("5x"), Some((5, false)));
        assert_eq!(parse_array("x3"), Some((3, false)));
        assert_eq!(parse_array("/4"), Some((4, true)));
        assert_eq!(parse_array("1x"), None);
        assert_eq!(parse_factors("2"), Some(DVec3::splat(2.0)));
        assert_eq!(parse_factors("2,1,0.5"), Some(DVec3::new(2.0, 1.0, 0.5)));
        assert_eq!(parse_factors("0"), None);
    }

    #[test]
    fn rotation_fraction_keeps_center() {
        let c = DVec3::new(5.0, 3.0, 0.0);
        let xf = rotation_about(c, DVec3::Z, 1.2);
        let half = fractional(xf, 0.5, Kind::Rotate);
        assert!((half.transform_point3(c) - c).length() < 1e-9);
        let twice = half * half;
        let p = DVec3::new(9.0, -1.0, 2.0);
        assert!((twice.transform_point3(p) - xf.transform_point3(p)).length() < 1e-9);
    }

    fn inf(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: mt_infer::Kind::Free,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
        }
    }

    fn square(ents: &mut Entities) -> FaceId {
        let p = [
            DVec3::ZERO,
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::new(0.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            mt_kernel::add_line(ents, p[i], p[(i + 1) % 4]);
        }
        ents.faces.keys().next().unwrap()
    }

    #[test]
    fn push_pull_then_copy_array() {
        let mut ents = Entities::default();
        let f = square(&mut ents);
        let n = ents.faces[f].normal;
        let mut pp = PushPull::default();
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: Some(Owner::Face(f)),
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        assert!(!pp.click(&mut ctx, &inf(DVec3::new(5.0, 5.0, 0.0))));
        assert!(pp.typed(
            &mut ctx,
            Some(&inf(DVec3::new(5.0, 5.0, 0.0) + n * 3.0)),
            "4",
            LengthUnit::Inch
        ));
        assert_eq!(ents.faces.len(), 6);

        // Copy the whole box 20 along red, then 3x: four boxes.
        let all: Vec<Owner> = ents.faces.keys().map(Owner::Face).collect();
        let mut mv = Transform::new(Kind::Move);
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &all,
            ctrl: true,
            eye: DVec3::ZERO,
        };
        mv.click(&mut ctx, &inf(DVec3::ZERO));
        assert!(mv.click(&mut ctx, &inf(DVec3::new(20.0, 0.0, 0.0))));
        assert!(mv.typed(&mut ctx, None, "3x", LengthUnit::Inch));
        assert_eq!(ents.faces.len(), 24);
        let max_x = ents
            .vertices
            .values()
            .map(|v| v.pos.x)
            .fold(f64::MIN, f64::max);
        assert!((max_x - 70.0).abs() < 1e-6);
    }

    #[test]
    fn move_typed_distance_and_rotate_angle() {
        let mut ents = Entities::default();
        let f = square(&mut ents);
        let sel = [Owner::Face(f)];
        let mut mv = Transform::new(Kind::Move);
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &sel,
            ctrl: false,
            eye: DVec3::ZERO,
        };
        mv.click(&mut ctx, &inf(DVec3::ZERO));
        assert!(mv.typed(
            &mut ctx,
            Some(&inf(DVec3::new(0.0, 0.0, 1.0))),
            "5",
            LengthUnit::Inch
        ));
        assert!(ents.vertices.values().all(|v| (v.pos.z - 5.0).abs() < 1e-9));

        let mut rot = Transform::new(Kind::Rotate);
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &sel,
            ctrl: false,
            eye: DVec3::ZERO,
        };
        rot.click(&mut ctx, &inf(DVec3::new(0.0, 0.0, 5.0)));
        rot.click(&mut ctx, &inf(DVec3::new(1.0, 0.0, 5.0)));
        assert!(rot.typed(
            &mut ctx,
            Some(&inf(DVec3::new(0.0, 1.0, 5.0))),
            "90",
            LengthUnit::Inch
        ));
        let min_x = ents
            .vertices
            .values()
            .map(|v| v.pos.x)
            .fold(f64::MAX, f64::min);
        assert!((min_x + 10.0).abs() < 1e-9);
    }

    fn unit_box() -> OrientedBox {
        OrientedBox::from_aabb(DVec3::ZERO, DVec3::new(10.0, 20.0, 30.0))
    }

    #[test]
    fn grips_of_solid_and_flat_boxes() {
        assert_eq!(grips(&unit_box()).len(), 26, "8 corners, 12 edges, 6 faces");
        let flat = OrientedBox::from_aabb(DVec3::ZERO, DVec3::new(10.0, 10.0, 0.0));
        let g = grips(&flat);
        assert_eq!(g.len(), 8, "a face: 4 corners and 4 edge midpoints");
        assert!(g.iter().all(|g| g[2] == 0));
    }

    #[test]
    fn grip_picking_takes_the_nearest_within_the_radius() {
        let b = unit_box();
        // Top view: x, y to pixels; z ignored (grips stacked along z overlap).
        let project = |p: DVec3| Some(DVec2::new(p.x * 10.0, p.y * 10.0));
        let g = pick_grip(&b, &project, DVec2::new(101.0, 198.0), 8.0).unwrap();
        assert_eq!([g[0], g[1]], [1, 1]);
        let g = pick_grip(&b, &project, DVec2::new(52.0, 3.0), 8.0).unwrap();
        assert_eq!([g[0], g[1]], [0, -1]);
        assert!(pick_grip(&b, &project, DVec2::new(30.0, 30.0), 8.0).is_none());
    }

    #[test]
    fn scale_about_the_opposite_grip_or_center() {
        let bbox = unit_box();
        // Corner (+,+,+): uniform about (0,0,0).
        let corner = Grab {
            grip: [1, 1, 1],
            bbox,
        };
        assert!(corner.uniform());
        let s = corner
            .factors_at(DVec3::new(20.0, 40.0, 60.0), false)
            .unwrap();
        assert!((s - DVec3::splat(2.0)).length() < 1e-9);
        let xf = corner.xf(s, false);
        assert!(
            xf.transform_point3(DVec3::ZERO).length() < 1e-9,
            "opposite corner stays"
        );
        // Face grip (+x): one axis, about the -x face.
        let face = Grab {
            grip: [1, 0, 0],
            bbox,
        };
        assert!(!face.uniform());
        let s = face.factors_at(DVec3::new(15.0, 3.0, 99.0), false).unwrap();
        assert!((s - DVec3::new(1.5, 1.0, 1.0)).length() < 1e-9);
        // About the center: the grip moves 5 for each 5 from the center.
        let s = face.factors_at(DVec3::new(15.0, 0.0, 0.0), true).unwrap();
        assert!((s.x - 2.0).abs() < 1e-9);
        let c = face.xf(s, true).transform_point3(bbox.center);
        assert!(c.distance(bbox.center) < 1e-9);
        // Edge grip (+x, -y): two axes, independently.
        let edge = Grab {
            grip: [1, -1, 0],
            bbox,
        };
        let s = edge
            .factors_at(DVec3::new(20.0, -20.0, 0.0), false)
            .unwrap();
        assert!((s - DVec3::new(2.0, 2.0, 1.0)).length() < 1e-9);
        let s = edge.factors_at(DVec3::new(5.0, 0.0, 0.0), false).unwrap();
        assert!((s - DVec3::new(0.5, 1.0, 1.0)).length() < 1e-9);
    }

    #[test]
    fn scale_in_component_axes() {
        // A box turned 90 degrees about blue: its red axis is the world's green.
        let bbox = unit_box().transformed(DMat4::from_rotation_z(std::f64::consts::FRAC_PI_2));
        let g = Grab {
            grip: [1, 0, 0],
            bbox,
        };
        let h = g.handle();
        assert!((h - DVec3::new(-10.0, 10.0, 15.0)).length() < 1e-9, "{h}");
        let s = g.factors_at(h + DVec3::Y * 10.0, false).unwrap();
        assert!((s - DVec3::new(2.0, 1.0, 1.0)).length() < 1e-9);
        let xf = g.xf(s, false);
        let far = xf.transform_point3(h);
        assert!(
            (far - DVec3::new(-10.0, 20.0, 15.0)).length() < 1e-9,
            "{far}"
        );
    }

    #[test]
    fn typed_scale_values() {
        let u = LengthUnit::Millimeter;
        let bbox = unit_box();
        let corner = Grab {
            grip: [1, 1, 1],
            bbox,
        };
        let face = Grab {
            grip: [0, 1, 0],
            bbox,
        };
        let v = |t: &str| parse_scale_values(t, u).unwrap();
        assert_eq!(corner.factors_typed(&v("2")), Some(DVec3::splat(2.0)));
        assert_eq!(
            corner.factors_typed(&v("2,1,0.5")),
            Some(DVec3::new(2.0, 1.0, 0.5))
        );
        assert_eq!(face.factors_typed(&v("3")), Some(DVec3::new(1.0, 3.0, 1.0)));
        // 20 inches deep: 1016 mm makes it 2.0.
        let s = face.factors_typed(&v("1016mm")).unwrap();
        assert!((s.y - 2.0).abs() < 1e-9 && s.x == 1.0);
        assert!(face.factors_typed(&v("0")).is_none());
        assert!(parse_scale_values("abc", u).is_none());
    }

    #[test]
    fn scale_tool_drags_a_grip() {
        let mut ents = Entities::default();
        let f = square(&mut ents);
        let sel = [Owner::Face(f)];
        let mut sc = Scale::default();
        let project = |p: DVec3| Some(DVec2::new(p.x * 10.0, p.y * 10.0));
        let no_bounds = |_: usize| None;
        let hover = |sc: &mut Scale, ents: &Entities, cursor: DVec2| {
            sc.hover(&Hover {
                ents,
                selection: &sel,
                instance_bounds: &no_bounds,
                project: &project,
                cursor: Some(cursor),
            })
        };
        hover(&mut sc, &ents, DVec2::new(100.0, 100.0));
        assert_eq!(sc.markers(&inf(DVec3::ZERO)).len(), 8);
        assert!(sc.markers(&inf(DVec3::ZERO)).iter().any(|m| m.hot));
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &sel,
            ctrl: false,
            eye: DVec3::ZERO,
        };
        assert!(!sc.click(&mut ctx, &inf(DVec3::new(10.0, 10.0, 0.0))));
        let (targets, xf, copy) = sc.live(&inf(DVec3::new(20.0, 20.0, 0.0))).unwrap();
        assert_eq!(targets, &sel);
        assert!(!copy);
        assert!(
            (xf.transform_point3(DVec3::new(10.0, 10.0, 0.0)) - DVec3::new(20.0, 20.0, 0.0))
                .length()
                < 1e-9
        );
        assert!(sc.click(&mut ctx, &inf(DVec3::new(20.0, 20.0, 0.0))));
        let max = ents
            .vertices
            .values()
            .map(|v| v.pos.x)
            .fold(f64::MIN, f64::max);
        assert!((max - 20.0).abs() < 1e-9);
        // Retyped right after: the value replaces the scale (factor of the original).
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &sel,
            ctrl: false,
            eye: DVec3::ZERO,
        };
        assert!(sc.typed(&mut ctx, None, "3", LengthUnit::Inch));
        let max = ents
            .vertices
            .values()
            .map(|v| v.pos.x)
            .fold(f64::MIN, f64::max);
        assert!((max - 30.0).abs() < 1e-9, "{max}");
    }

    #[test]
    fn push_pull_double_click_repeats_and_ctrl_toggles() {
        let mut ents = Entities::default();
        let f = square(&mut ents);
        let mut pp = PushPull::default();
        let p = DVec3::new(5.0, 5.0, 0.0);
        let n = ents.faces[f].normal;
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: Some(Owner::Face(f)),
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        pp.click(&mut ctx, &inf(p));
        assert!(pp.typed(&mut ctx, Some(&inf(p + n)), "4", LengthUnit::Inch));
        // A separate square, double-clicked: pulled by the same 4".
        let q = [
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(30.0, 0.0, 0.0),
            DVec3::new(30.0, 10.0, 0.0),
            DVec3::new(20.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            mt_kernel::add_line(&mut ents, q[i], q[(i + 1) % 4]);
        }
        let g = ents
            .faces
            .keys()
            .find(|&k| {
                ents.faces[k].outer.coedges.len() == 4
                    && ents
                        .loop_points(&ents.faces[k].outer)
                        .iter()
                        .all(|p| p.x >= 20.0)
            })
            .unwrap();
        let before = ents.faces.len();
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: Some(Owner::Face(g)),
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        assert!(pp.double_click(&mut ctx, &inf(DVec3::new(25.0, 5.0, 0.0))));
        assert_eq!(
            ents.faces.len(),
            before + 5,
            "a free-standing face keeps its base: four sides and a top"
        );
        let top = ents
            .vertices
            .values()
            .filter(|v| v.pos.x >= 20.0)
            .map(|v| v.pos.z.abs())
            .fold(0.0, f64::max);
        assert!((top - 4.0).abs() < 1e-9);

        let mut mv = Transform::new(Kind::Move);
        assert!(!mv.copy);
        mv.toggle_ctrl();
        assert!(mv.copy && mv.status().ends_with("(Copy)"));
        mv.toggle_ctrl();
        assert!(!mv.copy);
    }
}
