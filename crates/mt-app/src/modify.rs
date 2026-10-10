//! Modify tools: Push/Pull, Move, Rotate, Scale, Offset, Follow Me. They act on the
//! entity under the cursor or the selection, and take typed values like SketchUp
//! (distance, angle, factor, `5x` / `/5` arrays after a copy).
//!
//! Behaviors are drafts from memory of SketchUp (`spec/parity/STATUS.md`). Scale is
//! simplified: three points (fixed, reference, new) instead of SketchUp's grips.

use glam::{DMat4, DQuat, DVec3};
use mt_infer::Inference;
use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::{EdgeId, Entities, FaceId};

use crate::document::Owner;

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
}

pub fn for_command(id: &str) -> Option<Box<dyn ModifyTool>> {
    Some(match id {
        "tool.push_pull" => Box::new(PushPull::default()),
        "tool.move" => Box::new(Transform::new(Kind::Move)),
        "tool.rotate" => Box::new(Transform::new(Kind::Rotate)),
        "tool.scale" => Box::new(Transform::new(Kind::Scale)),
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
            Owner::Section(_) => {}
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

/// Points along the targets' edges, for previews.
fn outline(ents: &Entities, targets: &[Owner]) -> Vec<Vec<DVec3>> {
    let (edges, faces, _) = split(targets);
    let mut out: Vec<Vec<DVec3>> = edges
        .iter()
        .filter_map(|e| ents.edges.get(*e))
        .map(|e| vec![ents.pos(e.start), ents.pos(e.end)])
        .collect();
    for f in faces.iter().filter(|f| ents.faces.contains_key(**f)) {
        for l in ents.faces[*f].loops() {
            let mut pts = ents.loop_points(l);
            pts.push(pts[0]);
            out.push(pts);
        }
    }
    out
}

fn transform_lines(lines: Vec<Vec<DVec3>>, xf: DMat4) -> Vec<Vec<DVec3>> {
    lines
        .into_iter()
        .map(|l| l.into_iter().map(|p| xf.transform_point3(p)).collect())
        .collect()
}

// ------------------------------------------------------------------ Push/Pull

#[derive(Debug, Default)]
pub struct PushPull {
    /// Face, the clicked point on it, its normal.
    start: Option<(FaceId, DVec3, DVec3)>,
    /// Ctrl at the first click: keep the start face (new faces).
    new_faces: bool,
    /// Last distance, repeated by a double click in SketchUp (not yet).
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
                    self.new_faces = ctx.ctrl;
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
}

// ------------------------------------------------------------------ Move, Rotate, Scale

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
}

/// Move: base point, destination. Rotate: center, start direction, end direction.
/// Scale: fixed point, reference point, new point (uniform factor).
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
            Kind::Rotate | Kind::Scale => 3,
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
            (Kind::Scale, [f, r]) => {
                let (dr, dp) = ((*r - *f).length(), (p - *f).length());
                (dr > mt_kernel::TOL && dp > mt_kernel::TOL)
                    .then(|| scale_about(*f, DVec3::splat(dp / dr)))
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
        let copy = (self.copy || ctx.ctrl) && self.kind != Kind::Scale;
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

fn scale_about(c: DVec3, s: DVec3) -> DMat4 {
    DMat4::from_translation(c) * DMat4::from_scale(s) * DMat4::from_translation(-c)
}

impl ModifyTool for Transform {
    fn status(&self) -> String {
        match (self.kind, self.points.len()) {
            (Kind::Move, 0) => "Select object to move. Ctrl = copy.",
            (Kind::Move, _) => {
                "Select destination or enter distance. After a copy: 5x or /5 for arrays."
            }
            (Kind::Rotate, 0) => "Select center of rotation. Ctrl = copy.",
            (Kind::Rotate, 1) => "Select start point of the angle.",
            (Kind::Rotate, _) => "Select end point or enter angle.",
            (Kind::Scale, 0) => "Select fixed point.",
            (Kind::Scale, 1) => "Select reference point.",
            (Kind::Scale, _) => "Select new point or enter scale factor.",
        }
        .into()
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
            (Kind::Scale, [f, r], Some(p)) => {
                let dr = (*r - *f).length();
                (
                    "Scale",
                    if dr > 0.0 {
                        format!("{:.3}", (p - *f).length() / dr)
                    } else {
                        String::new()
                    },
                )
            }
            (Kind::Move, ..) => ("Distance", String::new()),
            (Kind::Rotate, ..) => ("Angle", String::new()),
            (Kind::Scale, ..) => ("Scale", String::new()),
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
            self.copy = ctx.ctrl;
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
            (Kind::Scale, [f, _]) => parse_factors(text).map(|s| scale_about(*f, s)),
            _ => None,
        };
        match xf {
            Some(xf) => self.finish(ctx, xf),
            None => false,
        }
    }

    fn preview(&self, ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let mut out = match self.xf(cur.point) {
            Some(xf) if self.points.len() + 1 == self.needed() => {
                transform_lines(outline(ents, &self.targets), xf)
            }
            _ => Vec::new(),
        };
        let mut guide = self.points.clone();
        guide.push(cur.point);
        if guide.len() > 1 {
            out.push(guide);
        }
        out
    }

    fn cancel(&mut self) {
        self.points.clear();
        self.targets.clear();
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
                Owner::Edge(_) | Owner::Section(_) => {}
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
}
