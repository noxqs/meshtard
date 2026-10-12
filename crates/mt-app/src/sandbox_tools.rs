//! Tools > Sandbox: Smoove, Stamp, Drape, Add Detail and Flip Edge (LEFTOVERS.md item
//! 7). The geometry is `mt_kernel::sandbox`; this is the clicking. Drafts from memory of
//! SketchUp and its help pages (unverified):
//!
//! - Smoove: type the radius (default 10' / 3 m), click the terrain (or select part of
//!   it first), move up or down (or type the height), click. Works on the open
//!   context's faces, so a terrain group is opened first, as in SketchUp.
//! - Stamp: type the offset (default 1'), click the object to stamp with (a group or
//!   component, or a face), click the terrain (a group, or the open context's faces),
//!   move up or down to set the pad's height (or type it), click.
//! - Drape: select edges (or groups of edges), then click the terrain.
//! - Add Detail: with faces selected, starting the tool splits them; otherwise click a
//!   face or edge for a new vertex, move up or down (or type the height), click.
//! - Flip Edge: click the edge between two triangles (or a face near the edge; the
//!   diagonals are soft and hidden).

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use glam::{DMat4, DVec2, DVec3};
use mt_infer::Inference;
use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::sandbox::{self, DetailAt, StampError};
use mt_kernel::{Entities, VertexId};

use crate::App;
use crate::document::Owner;
use crate::modify::{Ctx, Hover, ModelEdit, ModifyTool};
use crate::selection::Mode;

/// Most edges drawn in a Smoove preview.
const PREVIEW_EDGES: usize = 20_000;

fn is_metric(units: LengthUnit) -> bool {
    matches!(
        units,
        LengthUnit::Millimeter | LengthUnit::Centimeter | LengthUnit::Meter
    )
}

/// A level circle (polyline) around `c`.
fn circle(c: DVec3, r: f64) -> Vec<DVec3> {
    (0..=48)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / 48.0;
            c + DVec3::new(r * a.cos(), r * a.sin(), 0.0)
        })
        .collect()
}

/// The height of `cur` over `anchor` (the cursor is held to the blue axis).
fn rise(anchor: DVec3, cur: &Inference) -> f64 {
    cur.point.z - anchor.z
}

/// Vertices of the selected faces and edges.
fn selected_vertices(ents: &Entities, selection: &[Owner]) -> BTreeSet<VertexId> {
    let mut out = BTreeSet::new();
    for o in selection {
        match *o {
            Owner::Face(f) => {
                if let Some(face) = ents.faces.get(f) {
                    for l in face.loops() {
                        out.extend(ents.loop_vertices(l));
                    }
                }
            }
            Owner::Edge(e) => {
                if let Some(edge) = ents.edges.get(e) {
                    out.extend([edge.start, edge.end]);
                }
            }
            _ => {}
        }
    }
    out
}

// ------------------------------------------------------------------ Smoove

#[derive(Default)]
pub struct Smoove {
    radius: Option<f64>,
    metric: Cell<bool>,
    /// The clicked point and the vertices selected then.
    start: Option<(DVec3, BTreeSet<VertexId>)>,
}

impl Smoove {
    fn radius(&self) -> f64 {
        self.radius
            .unwrap_or_else(|| crate::sandbox::default_spacing(self.metric.get()))
    }

    fn apply(&mut self, ents: &mut Entities, height: f64) -> bool {
        let Some((c, sel)) = self.start.take() else {
            return false;
        };
        sandbox::smoove(ents, c, &sel, self.radius(), height) > 0
    }
}

impl ModifyTool for Smoove {
    fn status(&self) -> String {
        // Draft wording (unverified).
        match self.start {
            None => "Click the terrain to raise or lower it, or enter the radius.",
            Some(_) => "Move up or down and click to set the height, or enter a value.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        self.metric.set(is_metric(units));
        match (&self.start, cur) {
            (Some((a, _)), Some(c)) => ("Offset", units.format(rise(*a, c))),
            _ => ("Radius", units.format(self.radius())),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.as_ref().map(|s| s.0)
    }

    fn constraint(&self) -> Option<DVec3> {
        self.start.as_ref().map(|_| DVec3::Z)
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        match &self.start {
            None => {
                let on_terrain =
                    matches!(ctx.picked, Some(Owner::Face(f)) if sandbox::is_terrain(ctx.ents, f));
                let sel = selected_vertices(ctx.ents, ctx.selection);
                if on_terrain || !sel.is_empty() {
                    self.start = Some((inf.point, sel));
                }
                false
            }
            Some((a, _)) => {
                let h = rise(*a, inf);
                self.apply(ctx.ents, h)
            }
        }
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        _cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let Some(v) = parse_length(text, units) else {
            return false;
        };
        if self.start.is_some() {
            self.apply(ctx.ents, v)
        } else {
            if v > 0.0 {
                self.radius = Some(v);
            }
            false
        }
    }

    fn preview(&self, ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let r = self.radius();
        let Some((a, sel)) = &self.start else {
            return vec![circle(cur.point, r)];
        };
        let h = rise(*a, cur);
        let offsets = sandbox::smoove_offsets(ents, *a, sel, r, h);
        let dz = |v: VertexId| DVec3::Z * offsets.get(&v).copied().unwrap_or(0.0);
        let mut out = vec![circle(*a, r), vec![*a, *a + DVec3::Z * h]];
        let moved: BTreeSet<_> = offsets
            .keys()
            .flat_map(|&v| ents.vertices[v].edges.iter().copied())
            .take(PREVIEW_EDGES)
            .collect();
        for e in moved {
            let edge = &ents.edges[e];
            out.push(vec![
                ents.pos(edge.start) + dz(edge.start),
                ents.pos(edge.end) + dz(edge.end),
            ]);
        }
        out
    }

    fn cancel(&mut self) {
        self.start = None;
    }

    fn keeps_selection(&self) -> bool {
        true
    }
}

// ------------------------------------------------------------------ Stamp / Drape

/// A group or component of the open context, or the open context's own faces.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Instance(usize),
    Active,
}

/// The terrain under the cursor: a group or component, or a terrain face.
fn picked_terrain(ctx: &Ctx) -> Option<Target> {
    match ctx.picked? {
        Owner::Instance(i) => Some(Target::Instance(i)),
        Owner::Face(f) if sandbox::is_terrain(ctx.ents, f) => Some(Target::Active),
        _ => None,
    }
}

/// Runs `f` on the terrain's entities with the transform from the open context to
/// them.
fn with_terrain<R>(
    model: &mut mt_kernel::Model,
    active: &mut Entities,
    t: Target,
    f: impl FnOnce(&mut Entities, DMat4) -> R,
) -> Option<R> {
    match t {
        Target::Active => Some(f(active, DMat4::IDENTITY)),
        Target::Instance(i) => {
            let inst = active.instances.get(i)?;
            let def = model.definitions.get_mut(inst.definition)?;
            Some(f(&mut def.entities, inst.transform.inverse()))
        }
    }
}

/// What the terrain's owner selects after an edit.
fn owner_of(t: Target) -> Vec<Owner> {
    match t {
        Target::Instance(i) => vec![Owner::Instance(i)],
        Target::Active => Vec::new(),
    }
}

/// What gets stamped.
#[derive(Debug, Clone, PartialEq)]
enum StampObject {
    /// A group or component: its index and transform; its box once known (preview).
    Instance(usize, DMat4, Option<(DVec3, DVec3)>),
    /// A face's outer loop.
    Face(Vec<DVec3>),
}

/// Default Stamp offset: 1' (unverified).
const STAMP_OFFSET: f64 = 12.0;

#[derive(Default)]
pub struct Stamp {
    offset: Option<f64>,
    object: Option<StampObject>,
    /// The terrain and the clicked point on it.
    terrain: Option<(Target, DVec3)>,
    edit: Option<ModelEdit>,
    /// Why the last stamp failed, set by the edit.
    error: Rc<RefCell<Option<String>>>,
}

impl Stamp {
    fn offset(&self) -> f64 {
        self.offset.unwrap_or(STAMP_OFFSET)
    }

    /// The footprint outline (context space, plan) and its lowest height, for the
    /// preview.
    fn outline(&self) -> Option<(Vec<DVec2>, f64)> {
        match self.object.as_ref()? {
            StampObject::Face(pts) => {
                let low = pts.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
                Some((pts.iter().map(|p| p.truncate()).collect(), low))
            }
            StampObject::Instance(_, xf, Some((lo, hi))) => {
                let c = [
                    DVec3::new(lo.x, lo.y, lo.z),
                    DVec3::new(hi.x, lo.y, lo.z),
                    DVec3::new(hi.x, hi.y, lo.z),
                    DVec3::new(lo.x, hi.y, lo.z),
                ]
                .map(|p| xf.transform_point3(p));
                let low = c.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
                Some((c.iter().map(|p| p.truncate()).collect(), low))
            }
            StampObject::Instance(..) => None,
        }
    }

    fn finish(&mut self, rise: f64) {
        let (Some(object), Some((target, _))) = (self.object.take(), self.terrain.take()) else {
            return;
        };
        let offset = self.offset();
        let error = self.error.clone();
        self.edit = Some(Box::new(move |model, active| {
            let fp = match &object {
                StampObject::Face(pts) => {
                    let low = pts.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
                    Some((pts.clone(), low))
                }
                StampObject::Instance(i, ..) => active.instances.get(*i).and_then(|inst| {
                    let def = model.definitions.get(inst.definition)?;
                    let (poly, low) = sandbox::footprint(&def.entities, inst.transform)?;
                    Some((poly.iter().map(|q| q.extend(low)).collect(), low))
                }),
            };
            let Some((pts, low)) = fp else {
                *error.borrow_mut() = Some("The object to stamp with has no footprint.".into());
                return Vec::new();
            };
            let res = with_terrain(model, active, target, |ents, to| {
                let local: Vec<DVec3> = pts
                    .iter()
                    .map(|p| to.transform_point3(DVec3::new(p.x, p.y, low + rise)))
                    .collect();
                let poly: Vec<DVec2> = local.iter().map(|p| p.truncate()).collect();
                sandbox::stamp(ents, &poly, offset, local[0].z)
            });
            match res {
                Some(Ok(pad)) => {
                    *error.borrow_mut() = None;
                    match target {
                        Target::Active => vec![Owner::Face(pad)],
                        t => owner_of(t),
                    }
                }
                Some(Err(e)) => {
                    // Draft wording (unverified).
                    *error.borrow_mut() = Some(
                        match e {
                            StampError::NoFootprint => "The object to stamp with has no footprint.",
                            StampError::NoOffset => "The offset must be more than zero.",
                            StampError::OffTerrain => {
                                "The object and its offset must be over the terrain."
                            }
                            StampError::Failed => "The terrain could not be stamped there.",
                        }
                        .into(),
                    );
                    Vec::new()
                }
                None => Vec::new(),
            }
        }));
    }
}

impl ModifyTool for Stamp {
    fn status(&self) -> String {
        if let Some(e) = self.error.borrow().as_ref()
            && self.object.is_none()
        {
            return e.clone();
        }
        // Draft wording (unverified).
        match (&self.object, &self.terrain) {
            (None, _) => "Click the object to stamp with, or enter the offset.",
            (Some(_), None) => "Click the terrain, or enter the offset.",
            _ => "Move up or down and click to set the height, or enter a value.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        match (&self.terrain, cur) {
            (Some((_, a)), Some(c)) => ("Offset", units.format(rise(*a, c))),
            _ => ("Offset", units.format(self.offset())),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        self.terrain.map(|t| t.1)
    }

    fn constraint(&self) -> Option<DVec3> {
        self.terrain.map(|_| DVec3::Z)
    }

    fn hover(&mut self, h: &Hover) {
        if let Some(StampObject::Instance(i, _, b @ None)) = &mut self.object {
            *b = (h.instance_bounds)(*i);
        }
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        match (&self.object, &self.terrain) {
            (None, _) => {
                *self.error.borrow_mut() = None;
                self.object = match ctx.picked {
                    Some(Owner::Instance(i)) => ctx
                        .ents
                        .instances
                        .get(i)
                        .map(|inst| StampObject::Instance(i, inst.transform, None)),
                    Some(Owner::Face(f)) => Some(StampObject::Face(
                        ctx.ents.loop_points(&ctx.ents.faces[f].outer),
                    )),
                    _ => None,
                };
            }
            (Some(obj), None) => {
                let t = picked_terrain(ctx).filter(|t| match (t, obj) {
                    (Target::Instance(j), StampObject::Instance(i, ..)) => i != j,
                    (Target::Active, StampObject::Face(pts)) => {
                        // Not the stamping face itself.
                        !matches!(ctx.picked, Some(Owner::Face(f))
                            if ctx.ents.loop_points(&ctx.ents.faces[f].outer) == *pts)
                    }
                    _ => true,
                });
                if let Some(t) = t {
                    self.terrain = Some((t, inf.point));
                }
            }
            (Some(_), Some((_, a))) => {
                let r = rise(*a, inf);
                self.finish(r);
            }
        }
        false
    }

    fn typed(
        &mut self,
        _ctx: &mut Ctx,
        _cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let Some(v) = parse_length(text, units) else {
            return false;
        };
        if self.terrain.is_some() {
            self.finish(v);
        } else if v >= 0.0 {
            self.offset = Some(v);
        }
        false
    }

    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let Some((poly, low)) = self.outline() else {
            return Vec::new();
        };
        let z = low + self.terrain.map_or(0.0, |(_, a)| rise(a, cur));
        let ring = sandbox::offset_polygon(&poly, self.offset());
        let closed = |pts: &[DVec2]| -> Vec<DVec3> {
            pts.iter().chain(pts.first()).map(|p| p.extend(z)).collect()
        };
        vec![closed(&poly), closed(&ring)]
    }

    fn cancel(&mut self) {
        self.object = None;
        self.terrain = None;
    }

    fn take_model_edit(&mut self) -> Option<ModelEdit> {
        self.edit.take()
    }
}

#[derive(Default)]
pub struct Drape {
    edit: Option<ModelEdit>,
}

impl ModifyTool for Drape {
    fn status(&self) -> String {
        "Select edges above the terrain, then click the terrain.".into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, _inf: &Inference) -> bool {
        let Some(target) = picked_terrain(ctx) else {
            return false;
        };
        let mut edges = Vec::new();
        let mut groups = Vec::new();
        for o in ctx.selection {
            match *o {
                Owner::Edge(e) => {
                    if let Some(x) = ctx.ents.edges.get(e) {
                        edges.push((ctx.ents.pos(x.start), ctx.ents.pos(x.end)));
                    }
                }
                Owner::Instance(i) if Target::Instance(i) != target => groups.push(i),
                _ => {}
            }
        }
        if edges.is_empty() && groups.is_empty() {
            return false;
        }
        self.edit = Some(Box::new(move |model, active| {
            let mut segs = edges;
            for i in groups {
                let Some(inst) = active.instances.get(i) else {
                    continue;
                };
                let Some(def) = model.definitions.get(inst.definition) else {
                    continue;
                };
                for e in def.entities.edges.values() {
                    let p = |v| inst.transform.transform_point3(def.entities.pos(v));
                    segs.push((p(e.start), p(e.end)));
                }
            }
            let made = with_terrain(model, active, target, |ents, to| {
                let local: Vec<_> = segs
                    .iter()
                    .map(|(a, b)| (to.transform_point3(*a), to.transform_point3(*b)))
                    .collect();
                sandbox::drape(ents, &local)
            })
            .unwrap_or_default();
            match target {
                Target::Active => made.into_iter().map(Owner::Edge).collect(),
                t if made.is_empty() => {
                    let _ = t;
                    Vec::new()
                }
                t => owner_of(t),
            }
        }));
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

    fn preview(&self, _ents: &Entities, _cur: &Inference) -> Vec<Vec<DVec3>> {
        Vec::new()
    }

    fn cancel(&mut self) {}

    fn take_model_edit(&mut self) -> Option<ModelEdit> {
        self.edit.take()
    }
}

// ------------------------------------------------------------------ Add Detail

#[derive(Default)]
pub struct AddDetail {
    /// Where the vertex goes and the clicked point.
    at: Option<(DetailAt, DVec3)>,
}

impl AddDetail {
    fn apply(&mut self, ents: &mut Entities, dz: f64) -> bool {
        let Some((at, p)) = self.at.take() else {
            return false;
        };
        sandbox::add_detail_vertex(ents, at, p, dz).is_some()
    }
}

impl ModifyTool for AddDetail {
    fn status(&self) -> String {
        // Draft wording (unverified).
        match self.at {
            None => "Click a face or edge of the terrain to add a vertex.",
            Some(_) => "Move up or down and click to set the height, or enter a value.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        match (self.at, cur) {
            (Some((_, a)), Some(c)) => ("Offset", units.format(rise(a, c))),
            _ => ("Offset", String::new()),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        self.at.map(|a| a.1)
    }

    fn constraint(&self) -> Option<DVec3> {
        self.at.map(|_| DVec3::Z)
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        match self.at {
            None => {
                self.at = match ctx.picked {
                    Some(Owner::Face(f)) if sandbox::is_terrain(ctx.ents, f) => {
                        Some((DetailAt::Face(f), inf.point))
                    }
                    Some(Owner::Edge(e)) => Some((DetailAt::Edge(e), inf.point)),
                    _ => None,
                };
                false
            }
            Some((_, a)) => self.apply(ctx.ents, rise(a, inf)),
        }
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        _cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        match parse_length(text, units) {
            Some(v) if self.at.is_some() => self.apply(ctx.ents, v),
            _ => false,
        }
    }

    fn preview(&self, ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let Some((at, a)) = self.at else {
            return Vec::new();
        };
        let top = a + DVec3::Z * rise(a, cur);
        let corners: Vec<DVec3> = match at {
            DetailAt::Face(f) => ents
                .faces
                .get(f)
                .map(|x| ents.loop_points(&x.outer))
                .unwrap_or_default(),
            DetailAt::Edge(e) => ents.edges.get(e).map_or_else(Vec::new, |x| {
                let mut v = vec![ents.pos(x.start), ents.pos(x.end)];
                for &f in &x.faces {
                    v.extend(
                        ents.loop_vertices(&ents.faces[f].outer)
                            .into_iter()
                            .filter(|&w| w != x.start && w != x.end)
                            .map(|w| ents.pos(w)),
                    );
                }
                v
            }),
        };
        let mut out = vec![vec![a, top]];
        out.extend(corners.into_iter().map(|c| vec![c, top]));
        out
    }

    fn cancel(&mut self) {
        self.at = None;
    }
}

// ------------------------------------------------------------------ Flip Edge

#[derive(Default)]
pub struct FlipEdge;

impl ModifyTool for FlipEdge {
    fn status(&self) -> String {
        "Click an edge between two triangles to flip it.".into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        let e = match ctx.picked {
            Some(Owner::Edge(e)) => Some(e),
            Some(Owner::Face(f)) => sandbox::flippable_edge_near(ctx.ents, f, inf.point),
            _ => None,
        };
        e.and_then(|e| sandbox::flip_edge(ctx.ents, e)).is_some()
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

pub fn for_command(id: &str) -> Option<Box<dyn ModifyTool>> {
    Some(match id {
        "tool.sandbox_smoove" => Box::new(Smoove::default()),
        "tool.sandbox_stamp" => Box::new(Stamp::default()),
        "tool.sandbox_drape" => Box::new(Drape::default()),
        "tool.sandbox_add_detail" => Box::new(AddDetail::default()),
        "tool.sandbox_flip_edge" => Box::new(FlipEdge),
        _ => return None,
    })
}

impl App {
    /// Add Detail started with faces selected: they are split at once (one undo step).
    pub(crate) fn sandbox_activate(&mut self, frame: &eframe::Frame, id: &str) {
        if id != "tool.sandbox_add_detail" {
            return;
        }
        let faces: Vec<_> = self
            .selection
            .items()
            .iter()
            .filter_map(|o| match *o {
                Owner::Face(f) => Some(f),
                _ => None,
            })
            .collect();
        if faces.is_empty() {
            return;
        }
        let Some(doc) = &mut self.document else {
            return;
        };
        let mut made = Vec::new();
        doc.edit(|ents| {
            made = sandbox::add_detail(ents, &faces);
            !made.is_empty()
        });
        self.selection.clear();
        self.selection
            .apply(Mode::Replace, made.into_iter().map(Owner::Face));
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_infer::Kind;
    use mt_kernel::{Checks, validate};

    fn at(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: Kind::OnFace,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
            container: None,
        }
    }

    fn grid() -> Entities {
        mt_kernel::terrain::grid(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 10.0, 6, 6)
    }

    fn face_at(ents: &Entities, x: f64, y: f64) -> Owner {
        let f = ents
            .faces
            .keys()
            .find(|&f| {
                let pts = ents.loop_points(&ents.faces[f].outer);
                let c = pts.iter().copied().sum::<DVec3>() / pts.len() as f64;
                c.truncate().distance(DVec2::new(x, y)) < 5.0
            })
            .unwrap();
        Owner::Face(f)
    }

    fn ctx<'a>(ents: &'a mut Entities, picked: Option<Owner>, sel: &'a [Owner]) -> Ctx<'a> {
        Ctx {
            ents,
            picked,
            selection: sel,
            ctrl: false,
            eye: DVec3::new(0.0, 0.0, 100.0),
        }
    }

    #[test]
    fn smoove_click_move_click() {
        let mut ents = grid();
        let mut t = Smoove::default();
        assert!(!t.typed(&mut ctx(&mut ents, None, &[]), None, "15", LengthUnit::Inch));
        let p = face_at(&ents, 30.0, 30.0);
        let c = DVec3::new(30.0, 30.0, 0.0);
        assert!(!t.click(&mut ctx(&mut ents, Some(p), &[]), &at(c)));
        assert_eq!(t.constraint(), Some(DVec3::Z));
        assert!(t.preview(&ents, &at(c + DVec3::Z * 6.0)).len() > 2);
        assert!(t.click(&mut ctx(&mut ents, Some(p), &[]), &at(c + DVec3::Z * 6.0)));
        let top = ents.vertices.values().map(|v| v.pos.z).fold(0.0, f64::max);
        assert!((top - 6.0).abs() < 1e-9);
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
        // Off the terrain nothing starts.
        assert!(!t.click(&mut ctx(&mut ents, None, &[]), &at(c)));
        assert!(t.anchor().is_none());
    }

    #[test]
    fn add_detail_click_and_type() {
        let mut ents = grid();
        let n = ents.faces.len();
        let mut t = AddDetail::default();
        let p = face_at(&ents, 23.0, 27.0);
        let c = DVec3::new(23.0, 27.0, 0.0);
        assert!(!t.click(&mut ctx(&mut ents, Some(p), &[]), &at(c)));
        assert!(t.typed(&mut ctx(&mut ents, None, &[]), None, "4", LengthUnit::Inch));
        assert_eq!(ents.faces.len(), n + 2);
        assert!(
            ents.vertices
                .values()
                .any(|v| v.pos == DVec3::new(23.0, 27.0, 4.0))
        );
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn flip_edge_near_click() {
        let mut ents = grid();
        let p = face_at(&ents, 3.0, 7.0);
        let mut t = FlipEdge;
        // Near the square's diagonal from (0,0) to (10,10).
        assert!(t.click(
            &mut ctx(&mut ents, Some(p), &[]),
            &at(DVec3::new(4.0, 5.0, 0.0))
        ));
        assert!(ents.edges.values().any(|e| {
            let (a, b) = (ents.pos(e.start), ents.pos(e.end));
            a.truncate() + b.truncate() == DVec2::new(10.0, 10.0)
                && a.x != b.x
                && a.y != b.y
                && a.x != a.y
        }));
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn stamp_with_a_group_on_a_terrain_group() {
        let mut model = mt_kernel::Model::default();
        let mut root = Entities::new();
        // The terrain group at z = 0, the object group: a 10" square face at z = 20.
        let terrain = model.definitions.insert(mt_kernel::Definition {
            name: "Terrain".into(),
            entities: grid(),
            is_group: true,
            ..Default::default()
        });
        let mut sq = Entities::new();
        let pts = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            mt_kernel::add_line(
                &mut sq,
                DVec3::new(a.0, a.1, 0.0),
                DVec3::new(b.0, b.1, 0.0),
            );
        }
        let object = model.definitions.insert(mt_kernel::Definition {
            name: "Pad".into(),
            entities: sq,
            is_group: true,
            ..Default::default()
        });
        for (d, xf) in [
            (terrain, DMat4::IDENTITY),
            (
                object,
                DMat4::from_translation(DVec3::new(25.0, 25.0, 20.0)),
            ),
        ] {
            root.instances.push(mt_kernel::Instance {
                definition: d,
                transform: xf,
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
        }
        let mut t = Stamp::default();
        t.typed(&mut ctx(&mut root, None, &[]), None, "5", LengthUnit::Inch);
        t.click(
            &mut ctx(&mut root, Some(Owner::Instance(1)), &[]),
            &at(DVec3::new(30.0, 30.0, 20.0)),
        );
        let p = DVec3::new(30.0, 30.0, 0.0);
        t.click(&mut ctx(&mut root, Some(Owner::Instance(0)), &[]), &at(p));
        assert!(t.take_model_edit().is_none());
        // Pad 3" below the object's bottom: 17".
        t.click(&mut ctx(&mut root, None, &[]), &at(p - DVec3::Z * 3.0));
        let placed = t.take_model_edit().expect("edit")(&mut model, &mut root);
        assert_eq!(placed, vec![Owner::Instance(0)]);
        let ents = &model.definitions[terrain].entities;
        assert!(
            ents.vertices
                .values()
                .any(|v| v.pos == DVec3::new(25.0, 25.0, 17.0))
        );
        assert_eq!(validate(ents, Checks::ALL), Vec::<String>::new());
        assert!(t.error.borrow().is_none());
    }

    #[test]
    fn drape_selected_edges_onto_a_terrain_face() {
        let mut ents = grid();
        let sel: Vec<Owner> = mt_kernel::add_line(
            &mut ents,
            DVec3::new(5.0, 5.0, 30.0),
            DVec3::new(45.0, 15.0, 30.0),
        )
        .into_iter()
        .map(Owner::Edge)
        .collect();
        let p = face_at(&ents, 23.0, 27.0);
        let mut t = Drape::default();
        t.click(
            &mut ctx(&mut ents, Some(p), &sel),
            &at(DVec3::new(23.0, 27.0, 0.0)),
        );
        let mut model = mt_kernel::Model::default();
        let placed = t.take_model_edit().expect("edit")(&mut model, &mut ents);
        assert!(placed.len() >= 5);
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
        // Nothing selected: nothing to drape.
        let mut t = Drape::default();
        t.click(&mut ctx(&mut ents, Some(p), &[]), &at(DVec3::ZERO));
        assert!(t.take_model_edit().is_none());
    }
}
