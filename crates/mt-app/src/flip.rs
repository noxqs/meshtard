//! The Flip tool (FEATURES.md 48; help page "Flipping, mirroring, rotating and
//! arrays"): mirrors the selection about one of three planes through its bounding
//! box's center.
//!
//! - The planes are drawn semi-transparent, one per axis, colored by their normal's
//!   axis (red, green, blue); the one under the cursor is highlighted. A click on it
//!   flips the selection about it; a press and drag moves the plane along its normal
//!   first (with inference along that line) and flips about it on release.
//! - Arrow keys pick a plane (left green, right red, up blue; again to let go): a click
//!   anywhere then flips about it.
//! - Ctrl toggles copy: the flip makes a flipped copy. After a copy, `3x` / `3*` makes
//!   three flipped copies at the same spacing and `/3` three between the original and
//!   the copy (the copies all mirrored; how SketchUp arranges flipped arrays is
//!   unverified).
//! - Alt toggles between the object's axes (a single group or component) and the
//!   parent (open context) axes.
//! - Hovering a face away from the planes shows a magenta plane with the face's
//!   orientation through the center; clicking or dragging it flips about it.
//! - Without a selection, a click picks the entity under the cursor.
//!
//! Loose geometry is mirrored with its faces kept facing outward (the kernel reverses
//! the loops a mirror turns around) and merges like Move's; groups and components get a
//! mirrored transform. Each flip is one undo step. Plane size (15 % beyond the box) and
//! opacity are guesses.

use glam::{DMat3, DMat4, DVec3};
use mt_infer::units::{LengthUnit, parse_length};
use mt_infer::{Inference, Kind};
use mt_kernel::Entities;

use crate::document::Owner;
use crate::modify::{Ctx, Hover, Input, ModifyTool, Plane, apply, grip_box, parse_array};
use crate::selection::OrientedBox;

/// Plane colors: red, green, blue normals, and the custom plane.
const AXIS_COLORS: [[u8; 3]; 3] = [[230, 40, 40], [40, 170, 40], [40, 80, 230]];
const CUSTOM_COLOR: [u8; 3] = [230, 0, 230];
/// How far the planes reach beyond the box, as a share of its largest half size.
const MARGIN: f64 = 0.15;

/// The reflection about the plane through `p` with unit normal `n`.
pub fn mirror(p: DVec3, n: DVec3) -> DMat4 {
    let n = n.normalize();
    let m = DMat3::IDENTITY - 2.0 * DMat3::from_cols(n * n.x, n * n.y, n * n.z);
    DMat4::from_translation(n * (2.0 * p.dot(n))) * DMat4::from_mat3(m)
}

/// A rectangle in 3D: center, unit normal, unit in-plane axes and half sizes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect3 {
    c: DVec3,
    n: DVec3,
    u: DVec3,
    v: DVec3,
    hu: f64,
    hv: f64,
}

impl Rect3 {
    fn corners(&self) -> [DVec3; 4] {
        let (u, v) = (self.u * self.hu, self.v * self.hv);
        [
            self.c - u - v,
            self.c + u - v,
            self.c + u + v,
            self.c - u + v,
        ]
    }

    /// Where the ray `ro + t rd` (t > 0) meets the rectangle.
    fn hit(&self, ro: DVec3, rd: DVec3) -> Option<(f64, DVec3)> {
        let denom = rd.dot(self.n);
        if denom.abs() < 1e-12 {
            return None;
        }
        let t = (self.c - ro).dot(self.n) / denom;
        let p = ro + rd * t;
        let d = p - self.c;
        (t > 0.0 && d.dot(self.u).abs() <= self.hu && d.dot(self.v).abs() <= self.hv)
            .then_some((t, p))
    }

    fn moved(self, by: DVec3) -> Self {
        Self {
            c: self.c + by,
            ..self
        }
    }
}

/// The plane of box axis `i` (normal along `b.dirs[i]`), reaching beyond the box.
fn axis_rect(b: &OrientedBox, i: usize) -> Option<Rect3> {
    let m = b.half.max_element() * MARGIN;
    if m <= mt_kernel::TOL * MARGIN {
        return None;
    }
    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
    Some(Rect3 {
        c: b.center,
        n: b.dirs[i],
        u: b.dirs[j],
        v: b.dirs[k],
        hu: b.half[j] + m,
        hv: b.half[k] + m,
    })
}

/// A plane with normal `n` through the box's center, covering it.
fn custom_rect(b: &OrientedBox, n: DVec3) -> Option<Rect3> {
    let n = n.normalize_or_zero();
    let r = b.half.length() * (1.0 + MARGIN);
    if n == DVec3::ZERO || r <= mt_kernel::TOL {
        return None;
    }
    let (u, v) = n.any_orthonormal_pair();
    Some(Rect3 {
        c: b.center,
        n,
        u,
        v,
        hu: r,
        hv: r,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Which {
    Axis(usize),
    Custom,
}

/// A plane being dragged along its normal.
#[derive(Debug, Clone, Copy)]
struct Drag {
    rect: Rect3,
    /// Where the plane was grabbed.
    start: DVec3,
}

impl Drag {
    fn offset(&self, p: DVec3) -> f64 {
        (p - self.start).dot(self.rect.n)
    }
}

#[derive(Debug, Default)]
pub struct Flip {
    /// Targets clicked while nothing is selected.
    own: Vec<Owner>,
    targets: Vec<Owner>,
    /// The box in the object's axes (a single group or component) and along the
    /// parent's axes.
    object_box: Option<OrientedBox>,
    parent_box: Option<OrientedBox>,
    /// Alt toggled: the parent's axes for a single group or component.
    parent_axes: bool,
    alt_was: bool,
    /// This frame's input.
    press: bool,
    ray: Option<(DVec3, DVec3)>,
    face_normal: Option<DVec3>,
    /// The plane under the cursor (or picked by an arrow key) and the point grabbed.
    hot: Option<(Which, DVec3)>,
    /// Arrow key pick.
    locked: Option<usize>,
    /// The magenta plane from the hovered face.
    custom: Option<Rect3>,
    drag: Option<Drag>,
    copy: bool,
    /// The last flipped copy, for arrays: targets, plane through the box center, and
    /// the plane's offset along its normal.
    last_copy: Option<(Vec<Owner>, Rect3, f64)>,
}

impl Flip {
    fn bbox(&self) -> Option<OrientedBox> {
        if self.parent_axes {
            self.parent_box
        } else {
            self.object_box.or(self.parent_box)
        }
    }

    fn rect(&self, w: Which) -> Option<Rect3> {
        match w {
            Which::Axis(i) => axis_rect(&self.bbox()?, i),
            Which::Custom => self.custom,
        }
    }

    /// Flips (or copies) the targets about `rect` moved `offset` along its normal.
    fn finish(&mut self, ctx: &mut Ctx, rect: Rect3, offset: f64) -> bool {
        if self.targets.is_empty() {
            return false;
        }
        let xf = mirror(rect.c + rect.n * offset, rect.n);
        apply(ctx.ents, &self.targets, xf, self.copy);
        self.last_copy = self.copy.then(|| (self.targets.clone(), rect, offset));
        true
    }

    /// The `n`-copy array after a flipped copy (`n` copies at the copy's spacing, or
    /// `n` steps to it when `divide`).
    fn array(&mut self, ctx: &mut Ctx, n: usize, divide: bool) -> bool {
        let Some((targets, rect, d)) = self.last_copy.take() else {
            return false;
        };
        if d.abs() <= mt_kernel::TOL {
            return false;
        }
        let base = mirror(rect.c, rect.n);
        let step = rect.n * (2.0 * d);
        let at = |k: f64| DMat4::from_translation(step * k) * base;
        if divide {
            for k in 1..n {
                apply(ctx.ents, &targets, at(k as f64 / n as f64), true);
            }
        } else {
            for k in 2..=n {
                apply(ctx.ents, &targets, at(k as f64), true);
            }
        }
        true
    }
}

impl ModifyTool for Flip {
    fn status(&self) -> String {
        let text = if self.drag.is_some() {
            "Drag the plane along its axis and release to flip, or type a distance."
        } else if self.targets.is_empty() {
            "Select objects to flip, or click an entity."
        } else if self.last_copy.is_some() {
            "Click a plane to flip. Type 3x or /3 for an array of flipped copies."
        } else {
            "Click a plane to flip, drag it to move it first. Arrow keys = pick plane, \
             Ctrl = copy, Alt = object/parent axes."
        };
        if self.copy {
            format!("{text} (Copy)")
        } else {
            text.into()
        }
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        match (self.drag, cur) {
            (Some(d), Some(c)) => ("Distance", units.format(d.offset(c.point).abs())),
            _ => ("Distance", String::new()),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        self.drag.map(|d| d.start)
    }

    fn constraint(&self) -> Option<DVec3> {
        self.drag.map(|d| d.rect.n)
    }

    fn input(&mut self, i: &Input) {
        if i.held.alt && !self.alt_was {
            self.parent_axes = !self.parent_axes;
        }
        self.alt_was = i.held.alt;
        self.press = i.press;
        self.ray = i.ray;
        self.face_normal = i
            .cur
            .filter(|c| c.kind == Kind::OnFace)
            .and_then(|c| c.normal);
    }

    fn hover(&mut self, h: &Hover) {
        if self.drag.is_some() {
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
        self.object_box = grip_box(h.ents, &self.targets, h.instance_bounds);
        self.parent_box = match (self.object_box, &self.targets[..]) {
            (Some(b), [Owner::Instance(_)]) => OrientedBox::around(b.corners()),
            (b, _) => b,
        };
        self.hot = None;
        self.custom = None;
        let Some(b) = self.bbox() else {
            return;
        };
        if let Some(i) = self.locked {
            self.hot = Some((Which::Axis(i), b.center));
            return;
        }
        if let Some((ro, rd)) = self.ray {
            self.hot = (0..3)
                .filter_map(|i| Some((i, axis_rect(&b, i)?.hit(ro, rd)?)))
                .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
                .map(|(i, (_, p))| (Which::Axis(i), p));
        }
        if self.hot.is_none()
            && let Some(n) = self.face_normal
        {
            self.custom = custom_rect(&b, n);
            if self.custom.is_some() {
                self.hot = Some((Which::Custom, b.center));
            }
        }
    }

    fn arrow(&mut self, axis: mt_infer::Axis) -> bool {
        if self.targets.is_empty() || self.drag.is_some() {
            return false;
        }
        let i = match axis {
            mt_infer::Axis::Red => 0,
            mt_infer::Axis::Green => 1,
            mt_infer::Axis::Blue => 2,
        };
        self.locked = if self.locked == Some(i) {
            None
        } else {
            Some(i)
        };
        true
    }

    fn planes(&self, cur: &Inference) -> Vec<Plane> {
        if let Some(d) = self.drag {
            let color = match self.hot {
                Some((Which::Axis(i), _)) => AXIS_COLORS[i],
                _ => CUSTOM_COLOR,
            };
            return vec![Plane {
                corners: d.rect.moved(d.rect.n * d.offset(cur.point)).corners(),
                color,
                hot: true,
            }];
        }
        let Some(b) = self.bbox() else {
            return Vec::new();
        };
        let hot = self.hot.map(|h| h.0);
        let mut out: Vec<Plane> = (0..3)
            .filter_map(|i| {
                Some(Plane {
                    corners: axis_rect(&b, i)?.corners(),
                    color: AXIS_COLORS[i],
                    hot: hot == Some(Which::Axis(i)),
                })
            })
            .collect();
        if let Some(r) = self.custom {
            out.push(Plane {
                corners: r.corners(),
                color: CUSTOM_COLOR,
                hot: hot == Some(Which::Custom),
            });
        }
        out
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        if let Some(d) = self.drag.take() {
            return self.finish(ctx, d.rect, d.offset(inf.point));
        }
        let hot = self.hot.and_then(|(w, p)| Some((self.rect(w)?, p)));
        let Some((rect, p)) = hot.filter(|_| !self.targets.is_empty()) else {
            // Nothing to flip yet: pick the entity under the cursor.
            if ctx.selection.is_empty() && ctx.picked.is_some() {
                self.own = ctx.picked.into_iter().collect();
                self.last_copy = None;
            }
            return false;
        };
        if self.press {
            self.drag = Some(Drag { rect, start: p });
            return false;
        }
        self.finish(ctx, rect, 0.0)
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        if let Some(d) = self.drag {
            let Some(dist) = parse_length(text, units) else {
                return false;
            };
            let sign = match cur.map(|c| d.offset(c.point)) {
                Some(o) if o < 0.0 => -1.0,
                _ => 1.0,
            };
            self.drag = None;
            return self.finish(ctx, d.rect, dist * sign);
        }
        match parse_array(text) {
            Some((n, divide)) => self.array(ctx, n, divide),
            None => false,
        }
    }

    fn preview(&self, _ents: &Entities, _cur: &Inference) -> Vec<Vec<DVec3>> {
        Vec::new()
    }

    fn cancel(&mut self) {
        self.drag = None;
        self.locked = None;
    }

    fn toggle_ctrl(&mut self) {
        self.copy = !self.copy;
    }

    fn copy_mode(&self) -> bool {
        self.copy
    }

    fn live(&self, cur: &Inference) -> Option<(&[Owner], DMat4, bool)> {
        let d = self.drag?;
        let off = d.offset(cur.point);
        Some((
            &self.targets,
            mirror(d.rect.c + d.rect.n * off, d.rect.n),
            self.copy,
        ))
    }

    /// The flipped entities stay selected (copies leave the originals selected).
    fn keeps_selection(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Mods;
    use glam::DVec2;
    use mt_kernel::{Checks, FaceId, validate};

    fn inf(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: Kind::Free,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
            container: None,
        }
    }

    /// A 10" cube from `o`, with outward normals.
    fn cube(ents: &mut Entities, o: DVec3) {
        let q = [
            o,
            o + DVec3::new(10.0, 0.0, 0.0),
            o + DVec3::new(10.0, 10.0, 0.0),
            o + DVec3::new(0.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            mt_kernel::add_line(ents, q[i], q[(i + 1) % 4]);
        }
        let base: FaceId = ents
            .faces
            .iter()
            .find(|(_, f)| ents.loop_points(&f.outer).iter().all(|p| p.z == o.z))
            .map(|(id, _)| id)
            .unwrap();
        let d = 10.0 * ents.faces[base].normal.z.signum();
        mt_kernel::push_pull(ents, base, d, false).unwrap();
    }

    /// An L-shaped prism (not symmetric about x = 5): flipping it is visible.
    fn ell(ents: &mut Entities) {
        let q = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 4.0, 0.0),
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(4.0, 10.0, 0.0),
            DVec3::new(0.0, 10.0, 0.0),
        ];
        for i in 0..6 {
            mt_kernel::add_line(ents, q[i], q[(i + 1) % 6]);
        }
        let base = ents.faces.keys().next().unwrap();
        let d = 5.0 * ents.faces[base].normal.z.signum();
        mt_kernel::push_pull(ents, base, d, false).unwrap();
    }

    fn all(ents: &Entities) -> Vec<Owner> {
        ents.faces
            .keys()
            .map(Owner::Face)
            .chain(ents.edges.keys().map(Owner::Edge))
            .collect()
    }

    /// Every face's normal points away from the center of the connected solid it
    /// belongs to (cubes are 10" apart or more, so per-face: away from the center of
    /// the 10" cell it lies on).
    fn outward(ents: &Entities, centers: &[DVec3]) {
        for f in ents.faces.values() {
            let pts = ents.loop_points(&f.outer);
            let mid = pts.iter().copied().sum::<DVec3>() / pts.len() as f64;
            let c = centers
                .iter()
                .min_by(|a, b| a.distance(mid).total_cmp(&b.distance(mid)))
                .unwrap();
            assert!(
                (mid - *c).dot(f.normal) > 0.0,
                "face at {mid} looks inward (normal {})",
                f.normal
            );
        }
    }

    fn hover(fl: &mut Flip, ents: &Entities, sel: &[Owner], ray: Option<(DVec3, DVec3)>) {
        fl.input(&Input {
            held: Mods::default(),
            press: false,
            ray,
            cur: None,
            material: None,
        });
        let no_bounds = |_: usize| None;
        let project = |_: DVec3| None::<DVec2>;
        fl.hover(&Hover {
            ents,
            selection: sel,
            instance_bounds: &no_bounds,
            project: &project,
            cursor: None,
        });
    }

    fn ctx<'a>(ents: &'a mut Entities, sel: &'a [Owner]) -> Ctx<'a> {
        Ctx {
            ents,
            picked: None,
            selection: sel,
            ctrl: false,
            eye: DVec3::new(0.0, -100.0, 50.0),
        }
    }

    /// A ray looking down at `p` from above.
    fn down_at(p: DVec3) -> Option<(DVec3, DVec3)> {
        Some((p + DVec3::Z * 100.0, DVec3::NEG_Z))
    }

    fn bounds(ents: &Entities) -> (DVec3, DVec3) {
        let mut it = ents.vertices.values().map(|v| v.pos);
        let f = it.next().unwrap();
        it.fold((f, f), |(a, b), p| (a.min(p), b.max(p)))
    }

    #[test]
    fn mirror_is_a_reflection() {
        let m = mirror(DVec3::new(5.0, 0.0, 0.0), DVec3::X);
        assert!(
            (m.transform_point3(DVec3::new(1.0, 2.0, 3.0)) - DVec3::new(9.0, 2.0, 3.0)).length()
                < 1e-12
        );
        assert!(DMat3::from_mat4(m).determinant() < 0.0);
        let twice = m * m;
        assert!(twice.abs_diff_eq(DMat4::IDENTITY, 1e-12));
    }

    #[test]
    fn hovering_and_clicking_a_plane_flips_loose_geometry() {
        let mut ents = Entities::default();
        ell(&mut ents);
        let sel = all(&ents);
        let mut fl = Flip::default();
        // Looking down: the red and green planes are edge on, the blue one (z = 2.5)
        // is hit.
        hover(&mut fl, &ents, &sel, down_at(DVec3::new(2.0, 2.0, 0.0)));
        assert_eq!(fl.hot.map(|h| h.0), Some(Which::Axis(2)));
        assert_eq!(fl.planes(&inf(DVec3::ZERO)).len(), 3);
        // A ray along red meets the red plane (x = 5) face on, the others edge on.
        hover(
            &mut fl,
            &ents,
            &sel,
            Some((DVec3::new(-50.0, 2.0, 2.0), DVec3::X)),
        );
        assert_eq!(fl.hot.map(|h| h.0), Some(Which::Axis(0)));
        assert!(fl.click(&mut ctx(&mut ents, &sel), &inf(DVec3::ZERO)));
        assert!(
            validate(&ents, Checks::ALL).is_empty(),
            "{:?}",
            validate(&ents, Checks::ALL)
        );
        // The L's arm now sits at x = 6..10 instead of 0..4.
        let has = |p: DVec3| ents.vertices.values().any(|v| v.pos.distance(p) < 1e-6);
        assert!(has(DVec3::new(6.0, 10.0, 0.0)) && has(DVec3::new(10.0, 10.0, 0.0)));
        assert!(!has(DVec3::new(0.0, 10.0, 0.0)));
        assert_eq!(ents.faces.len(), 8);
        // Normals outward: every face normal points away from the L's interior.
        for (id, f) in &ents.faces {
            // A point on the face: the center of one of its triangles.
            let [a, b, c] = mt_kernel::tessellate::triangulate(&ents, id)[0];
            let mid = (a + b + c) / 3.0;
            let inside = mid - f.normal * 0.5;
            let outside = mid + f.normal * 0.5;
            let in_l = |p: DVec3| {
                (0.0..=5.0).contains(&p.z)
                    && (0.0..=10.0).contains(&p.x)
                    && (0.0..=10.0).contains(&p.y)
                    && (p.y <= 4.0 || p.x >= 6.0)
            };
            assert!(
                in_l(inside) && !in_l(outside),
                "face at {mid}, normal {}",
                f.normal
            );
        }
    }

    #[test]
    fn dragging_the_plane_moves_it_first() {
        let mut ents = Entities::default();
        cube(&mut ents, DVec3::ZERO);
        let sel = all(&ents);
        let mut fl = Flip::default();
        hover(
            &mut fl,
            &ents,
            &sel,
            Some((DVec3::new(-50.0, 5.0, 5.0), DVec3::X)),
        );
        assert_eq!(fl.hot.map(|h| h.0), Some(Which::Axis(0)));
        fl.press = true;
        assert!(!fl.click(&mut ctx(&mut ents, &sel), &inf(DVec3::ZERO)));
        assert_eq!(fl.constraint(), Some(DVec3::X));
        let start = fl.anchor().unwrap();
        // Live preview while dragging 7" along red.
        let (_, xf, copy) = fl.live(&inf(start + DVec3::X * 7.0)).unwrap();
        assert!(!copy);
        assert!((xf.transform_point3(DVec3::ZERO) - DVec3::new(24.0, 0.0, 0.0)).length() < 1e-9);
        // Release 5" along red: the plane at x = 10 mirrors the cube to 10..20.
        assert!(fl.click(&mut ctx(&mut ents, &sel), &inf(start + DVec3::X * 5.0)));
        assert!(validate(&ents, Checks::ALL).is_empty());
        let (lo, hi) = bounds(&ents);
        assert!((lo - DVec3::new(10.0, 0.0, 0.0)).length() < 1e-9, "{lo}");
        assert!((hi - DVec3::new(20.0, 10.0, 10.0)).length() < 1e-9, "{hi}");
        outward(&ents, &[DVec3::new(15.0, 5.0, 5.0)]);
    }

    /// Picks the red plane with the arrow key and drags it `d` along red.
    fn drag_red(fl: &mut Flip, ents: &mut Entities, sel: &[Owner], d: f64) -> bool {
        hover(fl, ents, sel, None);
        assert!(fl.arrow(mt_infer::Axis::Red));
        hover(fl, ents, sel, None);
        assert_eq!(fl.hot.map(|h| h.0), Some(Which::Axis(0)));
        fl.press = true;
        assert!(!fl.click(&mut ctx(ents, sel), &inf(DVec3::ZERO)));
        let start = fl.anchor().unwrap();
        fl.click(&mut ctx(ents, sel), &inf(start + DVec3::X * d))
    }

    fn cube_count(ents: &Entities) -> usize {
        ents.faces.len() / 6
    }

    #[test]
    fn ctrl_makes_a_flipped_copy_and_arrays() {
        let mut ents = Entities::default();
        cube(&mut ents, DVec3::ZERO);
        let sel = all(&ents);
        let mut fl = Flip::default();
        fl.toggle_ctrl();
        assert!(fl.copy_mode());
        // The plane moved to x = 15: a flipped copy at 20..30, the original stays.
        assert!(drag_red(&mut fl, &mut ents, &sel, 10.0));
        assert_eq!(cube_count(&ents), 2);
        assert!(validate(&ents, Checks::ALL).is_empty());
        let (lo, hi) = bounds(&ents);
        assert!((lo - DVec3::ZERO).length() < 1e-9 && (hi.x - 30.0).abs() < 1e-9);
        // 3x: three copies at the same spacing (20..30, 40..50, 60..70).
        assert!(fl.typed(&mut ctx(&mut ents, &[]), None, "3x", LengthUnit::Inch));
        assert_eq!(cube_count(&ents), 4);
        assert!(validate(&ents, Checks::ALL).is_empty());
        assert!((bounds(&ents).1.x - 70.0).abs() < 1e-9);
        outward(
            &ents,
            &[5.0, 25.0, 45.0, 65.0].map(|x: f64| DVec3::new(x, 5.0, 5.0)),
        );
        // The array is used up.
        assert!(!fl.typed(&mut ctx(&mut ents, &[]), None, "2x", LengthUnit::Inch));
    }

    #[test]
    fn divided_array_between_the_original_and_the_copy() {
        let mut ents = Entities::default();
        cube(&mut ents, DVec3::ZERO);
        let sel = all(&ents);
        let mut fl = Flip::default();
        fl.toggle_ctrl();
        assert!(drag_red(&mut fl, &mut ents, &sel, 20.0));
        assert!((bounds(&ents).1.x - 50.0).abs() < 1e-9);
        assert!(fl.typed(&mut ctx(&mut ents, &[]), None, "/2", LengthUnit::Inch));
        assert_eq!(cube_count(&ents), 3);
        let has = |p: DVec3| ents.vertices.values().any(|v| v.pos.distance(p) < 1e-6);
        assert!(has(DVec3::new(20.0, 0.0, 0.0)) && has(DVec3::new(30.0, 10.0, 10.0)));
        assert!(validate(&ents, Checks::ALL).is_empty());
        outward(&ents, &[5.0, 25.0, 45.0].map(|x| DVec3::new(x, 5.0, 5.0)));
    }

    #[test]
    fn typed_distance_while_dragging() {
        let mut ents = Entities::default();
        cube(&mut ents, DVec3::ZERO);
        let sel = all(&ents);
        let mut fl = Flip::default();
        hover(&mut fl, &ents, &sel, None);
        fl.arrow(mt_infer::Axis::Blue);
        hover(&mut fl, &ents, &sel, None);
        fl.press = true;
        fl.click(&mut ctx(&mut ents, &sel), &inf(DVec3::ZERO));
        let start = fl.anchor().unwrap();
        // The cursor is below the start: the typed distance goes down.
        let cur = inf(start - DVec3::Z * 3.0);
        assert!(fl.typed(
            &mut ctx(&mut ents, &sel),
            Some(&cur),
            "10",
            LengthUnit::Inch
        ));
        let (lo, hi) = bounds(&ents);
        assert!(
            (lo.z + 20.0).abs() < 1e-9 && (hi.z + 10.0).abs() < 1e-9,
            "{lo} {hi}"
        );
        outward(&ents, &[DVec3::new(5.0, 5.0, -15.0)]);
    }

    #[test]
    fn instances_get_a_mirrored_transform_in_object_or_parent_axes() {
        let mut ents = Entities::default();
        let rot = DMat4::from_translation(DVec3::new(100.0, 0.0, 0.0))
            * DMat4::from_rotation_z(std::f64::consts::FRAC_PI_4);
        ents.instances.push(mt_kernel::Instance {
            definition: Default::default(),
            transform: rot,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
        });
        let sel = [Owner::Instance(0)];
        let bounds = |_: usize| Some((DVec3::ZERO, DVec3::new(10.0, 2.0, 4.0)));
        let project = |_: DVec3| None::<DVec2>;
        let mut fl = Flip::default();
        let frame = |fl: &mut Flip, ents: &Entities, alt: bool| {
            fl.input(&Input {
                held: Mods {
                    alt,
                    ..Mods::default()
                },
                press: false,
                ray: None,
                cur: None,
                material: None,
            });
            fl.hover(&Hover {
                ents,
                selection: &sel,
                instance_bounds: &bounds,
                project: &project,
                cursor: None,
            });
        };
        frame(&mut fl, &ents, false);
        // Object axes: the red plane's normal is the instance's rotated red axis.
        let b = fl.bbox().unwrap();
        assert!((b.dirs[0] - rot.transform_vector3(DVec3::X)).length() < 1e-9);
        // Alt: parent axes.
        frame(&mut fl, &ents, true);
        frame(&mut fl, &ents, false);
        assert!((fl.bbox().unwrap().dirs[0] - DVec3::X).length() < 1e-9);
        // Alt again: object axes. Flip about the object's red plane.
        frame(&mut fl, &ents, true);
        frame(&mut fl, &ents, false);
        fl.arrow(mt_infer::Axis::Red);
        frame(&mut fl, &ents, false);
        let center = fl.bbox().unwrap().center;
        assert!(fl.click(&mut ctx(&mut ents, &sel), &inf(DVec3::ZERO)));
        let t = ents.instances[0].transform;
        assert!(DMat3::from_mat4(t).determinant() < 0.0, "mirrored");
        // The box center stays; the definition's red axis is reversed.
        let c_def = DVec3::new(5.0, 1.0, 2.0);
        assert!((t.transform_point3(c_def) - center).length() < 1e-9);
        let red = rot.transform_vector3(DVec3::X);
        assert!((t.transform_vector3(DVec3::X) + red).length() < 1e-9);
    }

    #[test]
    fn hovering_a_face_gives_a_custom_plane() {
        let mut ents = Entities::default();
        cube(&mut ents, DVec3::ZERO);
        let sel = all(&ents);
        let mut fl = Flip::default();
        let n = DVec3::new(1.0, 1.0, 0.0).normalize();
        let mut face = inf(DVec3::new(50.0, 50.0, 0.0));
        face.kind = Kind::OnFace;
        face.normal = Some(n);
        fl.input(&Input {
            held: Mods::default(),
            press: false,
            // Away from the planes.
            ray: down_at(DVec3::new(50.0, 50.0, 0.0)),
            cur: Some(&face),
            material: None,
        });
        let no_bounds = |_: usize| None;
        let project = |_: DVec3| None::<DVec2>;
        fl.hover(&Hover {
            ents: &ents,
            selection: &sel,
            instance_bounds: &no_bounds,
            project: &project,
            cursor: None,
        });
        assert_eq!(fl.hot.map(|h| h.0), Some(Which::Custom));
        let planes = fl.planes(&face);
        assert_eq!(planes.len(), 4);
        assert_eq!(planes[3].color, CUSTOM_COLOR);
        assert!(planes[3].hot);
        // Flip about the diagonal plane through the center: the cube maps onto itself.
        assert!(fl.click(&mut ctx(&mut ents, &sel), &face));
        assert!(validate(&ents, Checks::ALL).is_empty());
        let (lo, hi) = bounds(&ents);
        assert!(lo.abs_diff_eq(DVec3::ZERO, 1e-9) && hi.abs_diff_eq(DVec3::splat(10.0), 1e-9));
        outward(&ents, &[DVec3::splat(5.0)]);
    }

    #[test]
    fn without_a_selection_a_click_picks_the_target() {
        let mut ents = Entities::default();
        cube(&mut ents, DVec3::ZERO);
        let f = ents.faces.keys().next().unwrap();
        let mut fl = Flip::default();
        hover(&mut fl, &ents, &[], None);
        assert!(fl.planes(&inf(DVec3::ZERO)).is_empty());
        let mut c = ctx(&mut ents, &[]);
        c.picked = Some(Owner::Face(f));
        assert!(!fl.click(&mut c, &inf(DVec3::ZERO)));
        hover(&mut fl, &ents, &[], None);
        assert_eq!(fl.targets, vec![Owner::Face(f)]);
        assert!(!fl.planes(&inf(DVec3::ZERO)).is_empty());
    }
}
