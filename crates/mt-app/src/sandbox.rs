//! Draw > Sandbox: From Scratch and From Contours (FEATURES.md item 29), plus the app
//! side of tools that change the whole model ([`ModifyTool::take_model_edit`]).
//!
//! From Scratch (drafts from memory of SketchUp, unverified): type the grid spacing
//! first (default 10' in imperial models, 3 m in metric ones), click the start point,
//! click the length of the first side, then the width. Both sides are rounded to whole
//! grid squares (at least one). The grid lies level at the start point's height, in a
//! new group; each square is two triangles with a soft, smooth diagonal.
//!
//! From Contours: the selected edges' vertices are triangulated in plan (Delaunay) into
//! a new group; interior edges are soft and smooth. The contours stay.

use std::cell::Cell;

use glam::{DMat4, DVec3};
use mt_infer::Inference;
use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::{Entities, TOL};

use crate::App;
use crate::document::Owner;
use crate::modify::{Ctx, ModelEdit, ModifyTool};
use crate::selection::Mode;

/// Default grid spacing in inches: 10' imperial, 3 m metric (unverified).
fn default_spacing(metric: bool) -> f64 {
    if metric { 3000.0 / 25.4 } else { 120.0 }
}

/// Most grid lines drawn in the preview per direction.
const PREVIEW_LINES: usize = 200;

/// The Sandbox From Scratch tool.
#[derive(Default)]
pub struct FromScratch {
    spacing: Option<f64>,
    /// The model's units are metric (seen in `measurement`, called every frame).
    metric: Cell<bool>,
    start: Option<DVec3>,
    /// First side: unit direction and length in whole squares.
    first: Option<(DVec3, usize)>,
    edit: Option<ModelEdit>,
}

/// Group of `ents` (in the group's own coordinates) placed by `transform`, selected.
pub fn group_edit(ents: Entities, transform: DMat4) -> ModelEdit {
    Box::new(move |model, active| {
        let name = model.unique_definition_name("Group#1");
        let def = model.definitions.insert(mt_kernel::Definition {
            name,
            entities: ents,
            is_group: true,
            ..Default::default()
        });
        active.instances.push(mt_kernel::Instance {
            definition: def,
            transform,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
        });
        vec![Owner::Instance(active.instances.len() - 1)]
    })
}

impl FromScratch {
    fn spacing(&self) -> f64 {
        self.spacing
            .unwrap_or_else(|| default_spacing(self.metric.get()))
    }

    /// Squares along `len`, at least one.
    fn squares(&self, len: f64) -> usize {
        ((len / self.spacing()).round() as usize).max(1)
    }

    /// The first side's direction and length from the start to `p` (level).
    fn side(start: DVec3, p: DVec3) -> Option<(DVec3, f64)> {
        let d = DVec3::new(p.x - start.x, p.y - start.y, 0.0);
        (d.length() > TOL).then(|| (d.normalize(), d.length()))
    }

    /// The second side's unit direction (level, perpendicular to the first, towards
    /// `p`) and length to `p`.
    fn width(start: DVec3, dir: DVec3, p: DVec3) -> (DVec3, f64) {
        let perp = DVec3::Z.cross(dir);
        let w = (p - start).dot(perp);
        (if w < 0.0 { -perp } else { perp }, w.abs())
    }

    /// Grid lines of a `nu` x `nv` grid (preview).
    fn lines(&self, start: DVec3, u: DVec3, v: DVec3, nu: usize, nv: usize) -> Vec<Vec<DVec3>> {
        let s = self.spacing();
        let (lu, lv) = (u * s * nu as f64, v * s * nv as f64);
        let mut out = Vec::new();
        for i in (0..=nu).step_by(nu.div_ceil(PREVIEW_LINES).max(1)) {
            let p = start + u * s * i as f64;
            out.push(vec![p, p + lv]);
        }
        for j in (0..=nv).step_by(nv.div_ceil(PREVIEW_LINES).max(1)) {
            let p = start + v * s * j as f64;
            out.push(vec![p, p + lu]);
        }
        out.push(vec![start, start + lu, start + lu + lv, start + lv, start]);
        out
    }

    fn finish(&mut self, width_dir: DVec3, width: f64) {
        let (Some(start), Some((dir, nu))) = (self.start, self.first) else {
            return;
        };
        let nv = self.squares(width);
        let s = self.spacing();
        let ents = mt_kernel::terrain::grid(DVec3::ZERO, dir * s, width_dir * s, nu, nv);
        self.edit = Some(group_edit(ents, DMat4::from_translation(start)));
        self.start = None;
        self.first = None;
    }

    /// The point `inf` at the start's height.
    fn level(&self, inf: &Inference) -> DVec3 {
        let z = self.start.map_or(inf.point.z, |s| s.z);
        DVec3::new(inf.point.x, inf.point.y, z)
    }
}

impl ModifyTool for FromScratch {
    fn status(&self) -> String {
        // Draft wording (unverified).
        match (self.start, self.first) {
            (None, _) => "Select a start point or enter the grid spacing.",
            (Some(_), None) => "Select the length of the first side or enter a value.",
            _ => "Select the width or enter a value.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        self.metric.set(matches!(
            units,
            LengthUnit::Millimeter | LengthUnit::Centimeter | LengthUnit::Meter
        ));
        let s = self.spacing();
        match (self.start, self.first, cur) {
            (Some(start), None, Some(c)) => {
                let len = Self::side(start, self.level(c)).map_or(0.0, |(_, l)| l);
                ("Length", units.format(self.squares(len) as f64 * s))
            }
            (Some(start), Some((dir, _)), Some(c)) => {
                let (_, w) = Self::width(start, dir, self.level(c));
                ("Width", units.format(self.squares(w) as f64 * s))
            }
            _ => ("Grid Spacing", units.format(s)),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        let start = self.start?;
        Some(match self.first {
            Some((dir, n)) => start + dir * self.spacing() * n as f64,
            None => start,
        })
    }

    fn click(&mut self, _ctx: &mut Ctx, inf: &Inference) -> bool {
        let p = self.level(inf);
        match (self.start, self.first) {
            (None, _) => self.start = Some(p),
            (Some(start), None) => {
                if let Some((dir, len)) = Self::side(start, p) {
                    self.first = Some((dir, self.squares(len)));
                }
            }
            (Some(start), Some((dir, _))) => {
                let (wd, w) = Self::width(start, dir, p);
                if w > TOL {
                    self.finish(wd, w);
                }
            }
        }
        false
    }

    fn typed(
        &mut self,
        _ctx: &mut Ctx,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let Some(len) = parse_length(text, units).filter(|l| *l > TOL) else {
            return false;
        };
        match (self.start, self.first) {
            (None, _) => self.spacing = Some(len),
            (Some(start), None) => {
                let dir = cur
                    .and_then(|c| Self::side(start, self.level(c)))
                    .map_or(DVec3::X, |(d, _)| d);
                self.first = Some((dir, self.squares(len)));
            }
            (Some(start), Some((dir, _))) => {
                let wd = cur.map_or(DVec3::Z.cross(dir), |c| {
                    Self::width(start, dir, self.level(c)).0
                });
                self.finish(wd, len);
            }
        }
        false
    }

    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let Some(start) = self.start else {
            return Vec::new();
        };
        let p = self.level(cur);
        match self.first {
            None => Self::side(start, p).map_or_else(Vec::new, |(dir, len)| {
                let n = self.squares(len);
                self.lines(start, dir, DVec3::Z.cross(dir), n, 0)
            }),
            Some((dir, nu)) => {
                let (wd, w) = Self::width(start, dir, p);
                self.lines(start, dir, wd, nu, self.squares(w))
            }
        }
    }

    fn cancel(&mut self) {
        self.start = None;
        self.first = None;
    }

    fn take_model_edit(&mut self) -> Option<ModelEdit> {
        self.edit.take()
    }
}

/// From Contours: a TIN group through the vertices of the selected edges, or `None`
/// with fewer than three distinct points in plan.
pub fn from_contours(ents: &Entities, items: &[Owner]) -> Option<(Entities, DMat4)> {
    let edges: Vec<_> = items
        .iter()
        .filter_map(|o| match *o {
            Owner::Edge(e) => Some(e),
            _ => None,
        })
        .collect();
    let points = mt_kernel::terrain::contour_points(ents, &edges);
    let min = points.iter().copied().reduce(DVec3::min)?;
    let local: Vec<DVec3> = points.iter().map(|p| *p - min).collect();
    let tin = mt_kernel::terrain::tin(&local);
    (!tin.faces.is_empty()).then(|| (tin, DMat4::from_translation(min)))
}

impl App {
    /// Commands of FEATURES.md items 29-31 that are not tools.
    pub(crate) fn m13_command(
        &mut self,
        ctx: &eframe::egui::Context,
        frame: &eframe::Frame,
        id: &str,
    ) -> bool {
        match id {
            "tool.sandbox_from_contours" => self.sandbox_from_contours(frame),
            "tool.text_3d" => crate::text3d::open(ctx),
            _ => return false,
        }
        true
    }

    fn sandbox_from_contours(&mut self, frame: &eframe::Frame) {
        let items = self.selection.items().to_vec();
        let Some(doc) = &mut self.document else {
            return;
        };
        let Some((ents, xf)) = from_contours(doc.active(), &items) else {
            // Draft wording (unverified).
            self.message = Some("Select contour lines (edges at different heights).".into());
            return;
        };
        let mut placed = Vec::new();
        doc.edit_model(|m, active| {
            placed = group_edit(ents, xf)(m, active);
            true
        });
        self.selection.clear();
        self.selection.apply(Mode::Replace, placed);
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
    }

    /// Makes the change a modify tool left after a click or typed value, as one undo
    /// step, and selects what it made.
    pub(crate) fn apply_model_edit(&mut self, frame: &eframe::Frame) {
        let Some(edit) = self.modify.as_mut().and_then(|t| t.take_model_edit()) else {
            return;
        };
        let Some(doc) = &mut self.document else {
            return;
        };
        let mut placed = Vec::new();
        doc.edit_model(|m, active| {
            placed = edit(m, active);
            true
        });
        self.engine.unlock();
        self.selection.clear();
        self.selection.apply(Mode::Replace, placed);
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

    fn at(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: Kind::OnFace,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
        }
    }

    #[test]
    fn three_clicks_make_a_grid_group() {
        let mut t = FromScratch::default();
        let mut ents = Entities::new();
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        assert!(!t.typed(&mut ctx, None, "10", LengthUnit::Inch));
        t.click(&mut ctx, &at(DVec3::new(0.0, 0.0, 5.0)));
        // 38" rounds to 4 squares; the cursor height is ignored.
        t.click(&mut ctx, &at(DVec3::new(38.0, 0.0, 9.0)));
        assert!(t.take_model_edit().is_none());
        t.click(&mut ctx, &at(DVec3::new(10.0, -21.0, 0.0)));
        let edit = t.take_model_edit().expect("grid");
        let mut model = mt_kernel::Model::default();
        let mut root = Entities::new();
        let placed = edit(&mut model, &mut root);
        assert_eq!(placed, vec![Owner::Instance(0)]);
        let inst = &root.instances[0];
        assert_eq!(inst.transform.w_axis.truncate(), DVec3::new(0.0, 0.0, 5.0));
        let def = &model.definitions[inst.definition];
        assert!(def.is_group);
        // 4 x 2 squares, two triangles each.
        assert_eq!(def.entities.faces.len(), 16);
        let (lo, hi) = def
            .entities
            .vertices
            .values()
            .fold((DVec3::MAX, DVec3::MIN), |(a, b), v| {
                (a.min(v.pos), b.max(v.pos))
            });
        assert!(lo.abs_diff_eq(DVec3::new(0.0, -20.0, 0.0), 1e-9), "{lo}");
        assert!(hi.abs_diff_eq(DVec3::new(40.0, 0.0, 0.0), 1e-9), "{hi}");
    }

    #[test]
    fn default_spacing_follows_units() {
        let t = FromScratch::default();
        assert_eq!(
            t.measurement(None, LengthUnit::Foot).1,
            LengthUnit::Foot.format(120.0)
        );
        let (_, m) = t.measurement(None, LengthUnit::Meter);
        assert_eq!(m, LengthUnit::Meter.format(3000.0 / 25.4));
    }

    #[test]
    fn contours_to_tin() {
        let mut ents = Entities::new();
        let mut items = Vec::new();
        for (z, half) in [(0.0, 50.0), (10.0, 20.0)] {
            let c = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(x, y)| DVec3::new(x * half, y * half, z));
            // Three sides of each square: open contours, no faces.
            for i in 0..3 {
                for e in mt_kernel::add_line(&mut ents, c[i], c[i + 1]) {
                    items.push(Owner::Edge(e));
                }
            }
        }
        let (tin, xf) = from_contours(&ents, &items).expect("tin");
        assert_eq!(tin.faces.len(), 2 * 8 - 2 - 4);
        assert_eq!(xf.w_axis.truncate(), DVec3::new(-50.0, -50.0, 0.0));
        assert!(from_contours(&ents, &items[..1]).is_none());
    }
}
