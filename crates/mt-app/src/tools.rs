//! Drawing tools: Line, Rectangle, Rotated Rectangle, Circle, Polygon, Arc, 2 Point Arc,
//! 3 Point Arc, Pie and Freehand. Each takes inferred points (clicks, or press-drag-
//! release) and typed values from the Measurements box, and shows a preview.
//!
//! Shapes lie in the plane of the face clicked first, else on the ground; arrow keys
//! lock the plane (right: red normal, left: green, up: blue). Behaviors are drafts from
//! memory of SketchUp (`spec/parity/STATUS.md`).

use std::f64::consts::TAU;

use glam::DVec3;
use mt_infer::units::{LengthUnit, parse_length};
use mt_infer::{Axis, Inference};
use mt_kernel::{Arc, Entities, add_curve, add_line};

/// SketchUp's default segment counts.
pub const CIRCLE_SIDES: usize = 24;
pub const ARC_SEGMENTS: usize = 12;
pub const POLYGON_SIDES: usize = 6;

pub trait DrawTool {
    /// Point that directions are inferred from (the last point placed).
    fn anchor(&self) -> Option<DVec3>;
    fn status(&self) -> String;
    /// Measurements box label and current value.
    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String);
    /// A point placed by a click. Returns whether the model changed.
    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool;
    /// A value typed into the Measurements box. Returns whether the model changed.
    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool;
    /// Polylines to preview for the cursor at `cur`.
    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>>;
    fn cancel(&mut self);
    /// Arrow keys lock the drawing plane; tools without a plane ignore it.
    fn lock_plane(&mut self, _axis: Axis) -> bool {
        false
    }
    /// Pointer pressed and dragged: by default the press and release are clicks.
    fn drag_start(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        self.click(ents, inf)
    }
    fn drag_move(&mut self, _inf: &Inference) {}
    fn drag_end(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        self.click(ents, inf)
    }
}

pub fn for_command(id: &str) -> Option<Box<dyn DrawTool>> {
    Some(match id {
        "tool.line" => Box::new(LineTool::default()),
        "tool.rectangle" => Box::new(Rectangle::default()),
        "tool.rotated_rectangle" => Box::new(RotatedRectangle::default()),
        "tool.circle" => Box::new(Polygon::circle()),
        "tool.polygon" => Box::new(Polygon::regular()),
        "tool.arc" => Box::new(CenterArc::new(false)),
        "tool.pie" => Box::new(CenterArc::new(true)),
        "tool.arc_2pt" => Box::new(TwoPointArc::default()),
        "tool.arc_3pt" => Box::new(ThreePointArc::default()),
        "tool.freehand" => Box::new(Freehand::default()),
        "tool.tape_measure" => Box::new(TapeMeasure::default()),
        _ => return None,
    })
}

/// `"24s"`: a segment or side count.
pub fn parse_segments(text: &str) -> Option<usize> {
    let n: usize = text.trim().strip_suffix(['s', 'S'])?.trim().parse().ok()?;
    (3..=999).contains(&n).then_some(n)
}

/// Two comma- or semicolon-separated lengths.
fn parse_pair(text: &str, units: LengthUnit) -> Option<(f64, f64)> {
    let (a, b) = text.split_once([',', ';'])?;
    Some((parse_length(a, units)?, parse_length(b, units)?))
}

/// Plane normal for a shape started at `inf`: the locked one, the face's, else up.
fn plane_normal(lock: Option<DVec3>, inf: &Inference) -> DVec3 {
    lock.or(inf.normal).unwrap_or(DVec3::Z).normalize()
}

/// In-plane axes: red projected onto the plane (green if red is the normal), and the
/// normal crossed with it.
pub fn plane_axes(normal: DVec3) -> (DVec3, DVec3) {
    let mut u = DVec3::X - normal * DVec3::X.dot(normal);
    if u.length() < 1e-6 {
        u = DVec3::Y - normal * DVec3::Y.dot(normal);
    }
    let u = u.normalize();
    (u, normal.cross(u))
}

/// Corners of an axis-aligned rectangle with opposite corners `a` and `b`. When `b` is
/// off the plane (inferred straight up), the rectangle stands vertical through `a`.
pub fn rectangle_corners(a: DVec3, b: DVec3, normal: DVec3) -> [DVec3; 4] {
    let d = b - a;
    let off_plane = d.dot(normal);
    if off_plane.abs() > mt_kernel::TOL {
        let along = d - normal * off_plane;
        if along.length() > mt_kernel::TOL {
            return [a, a + along, b, a + normal * off_plane];
        }
    }
    let (u, v) = plane_axes(normal);
    let (w, h) = (d.dot(u), d.dot(v));
    [a, a + u * w, a + u * w + v * h, a + v * h]
}

fn closed(mut pts: Vec<DVec3>) -> Vec<DVec3> {
    if let Some(&first) = pts.first() {
        pts.push(first);
    }
    pts
}

// ------------------------------------------------------------------ Line

#[derive(Debug, Default)]
pub struct LineTool {
    anchor: Option<DVec3>,
}

impl LineTool {
    fn segment(&mut self, ents: &mut Entities, end: DVec3) -> bool {
        let Some(a) = self.anchor else {
            self.anchor = Some(end);
            return false;
        };
        let faces = ents.faces.len();
        add_line(ents, a, end);
        // The chain ends when a segment closes a face.
        self.anchor = (ents.faces.len() == faces).then_some(end);
        true
    }
}

impl DrawTool for LineTool {
    fn anchor(&self) -> Option<DVec3> {
        self.anchor
    }

    fn status(&self) -> String {
        if self.anchor.is_some() {
            "Select end point or enter value."
        } else {
            "Select start point."
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let len = self
            .anchor
            .zip(cur)
            .map(|(a, c)| units.format(a.distance(c.point)));
        ("Length", len.unwrap_or_default())
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        self.segment(ents, inf.point)
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let (Some(a), Some(c), Some(len)) = (self.anchor, cur, parse_length(text, units)) else {
            return false;
        };
        let d = (c.point - a).normalize_or_zero();
        d != DVec3::ZERO && self.segment(ents, a + d * len)
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        self.anchor
            .map(|a| vec![vec![a, cur.point]])
            .unwrap_or_default()
    }

    fn cancel(&mut self) {
        self.anchor = None;
    }

    fn drag_start(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        if self.anchor.is_none() {
            self.click(ents, inf)
        } else {
            false
        }
    }
}

// ------------------------------------------------------------------ Rectangle

#[derive(Debug, Default)]
pub struct Rectangle {
    first: Option<(DVec3, DVec3)>,
    lock: Option<DVec3>,
}

impl Rectangle {
    fn corners(&self, b: DVec3) -> Option<[DVec3; 4]> {
        let (a, n) = self.first?;
        Some(rectangle_corners(a, b, n))
    }

    fn make(&mut self, ents: &mut Entities, c: [DVec3; 4]) -> bool {
        let area = (c[1] - c[0]).cross(c[3] - c[0]).length();
        if area <= mt_kernel::TOL * mt_kernel::TOL {
            return false;
        }
        for i in 0..4 {
            add_line(ents, c[i], c[(i + 1) % 4]);
        }
        self.first = None;
        true
    }
}

impl DrawTool for Rectangle {
    fn anchor(&self) -> Option<DVec3> {
        self.first.map(|f| f.0)
    }

    fn status(&self) -> String {
        if self.first.is_some() {
            "Select opposite corner or enter value."
        } else {
            "Select first corner."
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let v = cur.and_then(|c| self.corners(c.point)).map(|c| {
            format!(
                "{}, {}",
                units.format(c[0].distance(c[1])),
                units.format(c[0].distance(c[3]))
            )
        });
        ("Dimensions", v.unwrap_or_default())
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        match self.corners(inf.point) {
            None => {
                self.first = Some((inf.point, plane_normal(self.lock, inf)));
                false
            }
            Some(c) => self.make(ents, c),
        }
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let (Some((a, n)), Some((w, h))) = (self.first, parse_pair(text, units)) else {
            return false;
        };
        // Signs follow the side the cursor is on, as in SketchUp.
        let (u, v) = plane_axes(n);
        let d = cur.map_or(u + v, |c| c.point - a);
        let su = if d.dot(u) < 0.0 { -1.0 } else { 1.0 };
        let sv = if d.dot(v) < 0.0 { -1.0 } else { 1.0 };
        self.make(ents, rectangle_corners(a, a + u * w * su + v * h * sv, n))
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        self.corners(cur.point)
            .map(|c| vec![closed(c.to_vec())])
            .unwrap_or_default()
    }

    fn cancel(&mut self) {
        self.first = None;
    }

    fn lock_plane(&mut self, axis: Axis) -> bool {
        self.lock = if self.lock == Some(axis.dir()) {
            None
        } else {
            Some(axis.dir())
        };
        true
    }
}

// ------------------------------------------------------------------ Rotated rectangle

/// Three points: one side (any direction), then the width perpendicular to it.
#[derive(Debug, Default)]
pub struct RotatedRectangle {
    points: Vec<DVec3>,
}

impl RotatedRectangle {
    fn corners(&self, c: DVec3) -> Option<[DVec3; 4]> {
        let [a, b] = self.points[..] else { return None };
        let side = (b - a).normalize_or_zero();
        let w = (c - b) - side * (c - b).dot(side);
        Some([a, b, b + w, a + w])
    }
}

impl DrawTool for RotatedRectangle {
    fn anchor(&self) -> Option<DVec3> {
        self.points.last().copied()
    }

    fn status(&self) -> String {
        match self.points.len() {
            0 => "Select first corner.",
            1 => "Select second corner.",
            _ => "Select third corner.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let v = match (self.points.len(), cur) {
            (1, Some(c)) => units.format(self.points[0].distance(c.point)),
            (2, Some(c)) => self
                .corners(c.point)
                .map(|k| units.format(k[1].distance(k[2])))
                .unwrap_or_default(),
            _ => String::new(),
        };
        ("Length", v)
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        if self.points.len() < 2 {
            if self
                .points
                .last()
                .is_none_or(|p| p.distance(inf.point) > mt_kernel::TOL)
            {
                self.points.push(inf.point);
            }
            return false;
        }
        let Some(c) = self.corners(inf.point) else {
            return false;
        };
        if c[1].distance(c[2]) <= mt_kernel::TOL {
            return false;
        }
        for i in 0..4 {
            add_line(ents, c[i], c[(i + 1) % 4]);
        }
        self.points.clear();
        true
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let (Some(c), Some(len)) = (cur, parse_length(text, units)) else {
            return false;
        };
        match self.points.len() {
            1 => {
                let d = (c.point - self.points[0]).normalize_or_zero();
                if d != DVec3::ZERO {
                    self.points.push(self.points[0] + d * len);
                }
                false
            }
            2 => {
                let Some(k) = self.corners(c.point) else {
                    return false;
                };
                let w = (k[2] - k[1]).normalize_or_zero();
                let mut fake = *c;
                fake.point = k[1] + w * len;
                w != DVec3::ZERO && self.click(ents, &fake)
            }
            _ => false,
        }
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match self.points.len() {
            1 => vec![vec![self.points[0], cur.point]],
            2 => self
                .corners(cur.point)
                .map(|c| vec![closed(c.to_vec())])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.points.clear();
    }
}

// ------------------------------------------------------------------ Circle and polygon

/// Circle (a 24-sided curve) or regular polygon (6 sides): center, then radius. The
/// first vertex points at the cursor.
#[derive(Debug)]
pub struct Polygon {
    is_circle: bool,
    sides: usize,
    center: Option<(DVec3, DVec3)>,
    lock: Option<DVec3>,
}

impl Polygon {
    pub fn circle() -> Self {
        Self {
            is_circle: true,
            sides: CIRCLE_SIDES,
            center: None,
            lock: None,
        }
    }

    pub fn regular() -> Self {
        Self {
            is_circle: false,
            sides: POLYGON_SIDES,
            center: None,
            lock: None,
        }
    }

    fn arc(&self, toward: DVec3, radius: Option<f64>) -> Option<Arc> {
        let (c, n) = self.center?;
        let d = toward - c;
        let x = (d - n * d.dot(n)).normalize_or_zero();
        let x = if x == DVec3::ZERO { plane_axes(n).0 } else { x };
        let radius = radius.unwrap_or_else(|| (d - n * d.dot(n)).length());
        (radius > mt_kernel::TOL).then_some(Arc {
            center: c,
            normal: n,
            x_axis: x,
            radius,
            start_angle: 0.0,
            end_angle: TAU,
            segments: self.sides,
        })
    }

    fn make(&mut self, ents: &mut Entities, arc: Arc) -> bool {
        add_curve(ents, &arc.points(), self.is_circle.then_some(arc));
        self.center = None;
        true
    }
}

impl DrawTool for Polygon {
    fn anchor(&self) -> Option<DVec3> {
        self.center.map(|c| c.0)
    }

    fn status(&self) -> String {
        if self.center.is_some() {
            "Select edge point or enter value."
        } else {
            "Select center point."
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        match (self.center, cur.and_then(|c| self.arc(c.point, None))) {
            (Some(_), Some(a)) => ("Radius", units.format(a.radius)),
            _ => ("Sides", self.sides.to_string()),
        }
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        if self.center.is_none() {
            self.center = Some((inf.point, plane_normal(self.lock, inf)));
            return false;
        }
        match self.arc(inf.point, None) {
            Some(a) => self.make(ents, a),
            None => false,
        }
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        if let Some(n) = parse_segments(text).or_else(|| {
            // Before the center is placed a plain number is the side count.
            (self.center.is_none())
                .then(|| text.trim().parse().ok())
                .flatten()
                .filter(|n: &usize| *n >= 3)
        }) {
            self.sides = n;
            return false;
        }
        let (Some(c), Some(r)) = (cur, parse_length(text, units)) else {
            return false;
        };
        match self.arc(c.point, Some(r.abs())) {
            Some(a) => self.make(ents, a),
            None => false,
        }
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match self.arc(cur.point, None) {
            Some(a) => vec![a.points(), vec![a.center, cur.point]],
            None => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.center = None;
    }

    fn lock_plane(&mut self, axis: Axis) -> bool {
        self.lock = if self.lock == Some(axis.dir()) {
            None
        } else {
            Some(axis.dir())
        };
        true
    }
}

// ------------------------------------------------------------------ Arcs

/// Angle of `p` around `center` in the frame (`x`, `normal x x`), in `[0, 2pi)`.
fn angle_of(p: DVec3, center: DVec3, x: DVec3, normal: DVec3) -> f64 {
    let d = p - center;
    d.dot(normal.cross(x)).atan2(d.dot(x)).rem_euclid(TAU)
}

/// The arc from `a` to `b` bulging through `apex`'s side by `bulge` (sagitta), in the
/// plane with `normal`.
pub fn arc_from_bulge(
    a: DVec3,
    b: DVec3,
    bulge: f64,
    normal: DVec3,
    segments: usize,
) -> Option<Arc> {
    let chord = b - a;
    let c = chord.length();
    if c <= mt_kernel::TOL || bulge.abs() <= mt_kernel::TOL {
        return None;
    }
    let dir = chord / c;
    let n = (normal - dir * normal.dot(dir)).normalize_or_zero();
    let n = if n == DVec3::ZERO {
        plane_axes(dir).0
    } else {
        n
    };
    let m = n.cross(dir);
    let r = (c * c / 4.0 + bulge * bulge) / (2.0 * bulge.abs());
    let mid = (a + b) / 2.0;
    let center = mid + m * (bulge - bulge.signum() * r);
    arc_through(a, mid + m * bulge, b, center, n, r, segments)
}

/// The arc around `center` from `a` to `b` that passes `through`.
fn arc_through(
    a: DVec3,
    through: DVec3,
    b: DVec3,
    center: DVec3,
    n: DVec3,
    r: f64,
    segments: usize,
) -> Option<Arc> {
    let x = (a - center).normalize_or_zero();
    if x == DVec3::ZERO {
        return None;
    }
    let tb = angle_of(b, center, x, n);
    let tp = angle_of(through, center, x, n);
    let end = if tp <= tb { tb } else { tb - TAU };
    Some(Arc {
        center,
        normal: n,
        x_axis: x,
        radius: r,
        start_angle: 0.0,
        end_angle: end,
        segments,
    })
}

/// The circle through three points, as an arc from `a` through `b` to `c`.
pub fn arc_three_points(a: DVec3, b: DVec3, c: DVec3, segments: usize) -> Option<Arc> {
    let (ab, ac) = (b - a, c - a);
    let n = ab.cross(ac);
    if n.length() <= 1e-9 * ab.length() * ac.length() {
        return None;
    }
    let center = a
        + (n.cross(ab) * ac.length_squared() + ac.cross(n) * ab.length_squared())
            / (2.0 * n.length_squared());
    let n = n.normalize();
    arc_through(a, b, c, center, n, center.distance(a), segments)
}

#[derive(Debug, Default)]
pub struct TwoPointArc {
    points: Vec<DVec3>,
    normal: DVec3,
    segments: Option<usize>,
    lock: Option<DVec3>,
}

impl TwoPointArc {
    fn bulge_of(&self, p: DVec3) -> Option<(DVec3, DVec3, f64)> {
        let [a, b] = self.points[..] else { return None };
        let dir = (b - a).normalize_or_zero();
        let n = (self.normal - dir * self.normal.dot(dir)).normalize_or_zero();
        let n = if n == DVec3::ZERO {
            plane_axes(dir).0
        } else {
            n
        };
        Some((a, b, (p - a).dot(n.cross(dir))))
    }

    fn arc(&self, bulge: f64) -> Option<Arc> {
        let [a, b] = self.points[..] else { return None };
        arc_from_bulge(
            a,
            b,
            bulge,
            self.normal,
            self.segments.unwrap_or(ARC_SEGMENTS),
        )
    }

    fn make(&mut self, ents: &mut Entities, bulge: f64) -> bool {
        let [a, b] = self.points[..] else {
            return false;
        };
        match self.arc(bulge) {
            Some(arc) => {
                add_curve(ents, &arc.points(), Some(arc));
            }
            None => {
                add_line(ents, a, b);
            }
        }
        self.points.clear();
        true
    }
}

impl DrawTool for TwoPointArc {
    fn anchor(&self) -> Option<DVec3> {
        self.points.first().copied()
    }

    fn status(&self) -> String {
        match self.points.len() {
            0 => "Select start point.",
            1 => "Select end point or enter value.",
            _ => "Select bulge amount or enter value.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        match (self.points.len(), cur) {
            (1, Some(c)) => ("Length", units.format(self.points[0].distance(c.point))),
            (2, Some(c)) => (
                "Bulge",
                self.bulge_of(c.point)
                    .map(|b| units.format(b.2.abs()))
                    .unwrap_or_default(),
            ),
            _ => ("Length", String::new()),
        }
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        match self.points.len() {
            0 => {
                self.normal = plane_normal(self.lock, inf);
                self.points.push(inf.point);
                false
            }
            1 => {
                if self.points[0].distance(inf.point) > mt_kernel::TOL {
                    self.points.push(inf.point);
                }
                false
            }
            _ => match self.bulge_of(inf.point) {
                Some((_, _, bulge)) => self.make(ents, bulge),
                None => false,
            },
        }
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        if let Some(n) = parse_segments(text) {
            self.segments = Some(n);
            return false;
        }
        let (Some(c), Some(v)) = (cur, parse_length(text, units)) else {
            return false;
        };
        match self.points.len() {
            1 => {
                let d = (c.point - self.points[0]).normalize_or_zero();
                if d != DVec3::ZERO {
                    self.points.push(self.points[0] + d * v);
                }
                false
            }
            2 => {
                let side = self
                    .bulge_of(c.point)
                    .map_or(1.0, |b| if b.2 < 0.0 { -1.0 } else { 1.0 });
                self.make(ents, v.abs() * side)
            }
            _ => false,
        }
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match self.points.len() {
            1 => vec![vec![self.points[0], cur.point]],
            2 => {
                let bulge = self.bulge_of(cur.point).map_or(0.0, |b| b.2);
                match self.arc(bulge) {
                    Some(a) => vec![a.points()],
                    None => vec![self.points.clone()],
                }
            }
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.points.clear();
    }

    fn lock_plane(&mut self, axis: Axis) -> bool {
        self.lock = if self.lock == Some(axis.dir()) {
            None
        } else {
            Some(axis.dir())
        };
        true
    }
}

/// Arc (center, start, end) or Pie (the same, closed with two radii into a face).
#[derive(Debug)]
pub struct CenterArc {
    pie: bool,
    center: Option<(DVec3, DVec3)>,
    start: Option<DVec3>,
    segments: Option<usize>,
    lock: Option<DVec3>,
}

impl CenterArc {
    pub fn new(pie: bool) -> Self {
        Self {
            pie,
            center: None,
            start: None,
            segments: None,
            lock: None,
        }
    }

    fn arc(&self, end: DVec3) -> Option<Arc> {
        let ((c, n), s) = (self.center?, self.start?);
        let x = (s - c).normalize_or_zero();
        let r = s.distance(c);
        let mut sweep = angle_of(end, c, x, n);
        if sweep < 1e-6 {
            sweep = TAU;
        }
        (r > mt_kernel::TOL && x != DVec3::ZERO).then_some(Arc {
            center: c,
            normal: n,
            x_axis: x,
            radius: r,
            start_angle: 0.0,
            end_angle: sweep,
            segments: self.segments.unwrap_or(ARC_SEGMENTS),
        })
    }

    fn make(&mut self, ents: &mut Entities, arc: Arc) -> bool {
        let pts = arc.points();
        add_curve(ents, &pts, Some(arc));
        if self.pie {
            add_line(ents, arc.center, pts[0]);
            add_line(ents, arc.center, *pts.last().expect("arc has points"));
        }
        self.center = None;
        self.start = None;
        true
    }
}

impl DrawTool for CenterArc {
    fn anchor(&self) -> Option<DVec3> {
        self.center.map(|c| c.0)
    }

    fn status(&self) -> String {
        match (self.center, self.start) {
            (None, _) => "Select center point.",
            (Some(_), None) => "Select start point or enter value.",
            _ => "Select end point or enter value.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        match (self.center, self.start, cur) {
            (Some((c, _)), None, Some(p)) => ("Radius", units.format(c.distance(p.point))),
            (Some(_), Some(_), Some(p)) => (
                "Angle",
                self.arc(p.point)
                    .map(|a| format!("{:.1}", a.end_angle.to_degrees()))
                    .unwrap_or_default(),
            ),
            _ => ("Radius", String::new()),
        }
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        match (self.center, self.start) {
            (None, _) => {
                self.center = Some((inf.point, plane_normal(self.lock, inf)));
                false
            }
            (Some((c, n)), None) => {
                // Keep the start point in the plane.
                let d = inf.point - c;
                let s = c + d - n * d.dot(n);
                if s.distance(c) > mt_kernel::TOL {
                    self.start = Some(s);
                }
                false
            }
            _ => match self.arc(inf.point) {
                Some(a) => self.make(ents, a),
                None => false,
            },
        }
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        if let Some(n) = parse_segments(text) {
            self.segments = Some(n);
            return false;
        }
        let Some(cur) = cur else { return false };
        match (self.center, self.start) {
            (Some((c, n)), None) => {
                let Some(r) = parse_length(text, units) else {
                    return false;
                };
                let d = cur.point - c;
                let x = (d - n * d.dot(n)).normalize_or_zero();
                if x != DVec3::ZERO {
                    self.start = Some(c + x * r.abs());
                }
                false
            }
            (Some(_), Some(_)) => {
                let Some(deg) = text.trim().trim_end_matches('°').parse::<f64>().ok() else {
                    return false;
                };
                let Some(mut a) = self.arc(cur.point) else {
                    return false;
                };
                a.end_angle = deg.to_radians().clamp(-TAU, TAU);
                self.make(ents, a)
            }
            _ => false,
        }
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match (self.center, self.start) {
            (Some((c, _)), None) => vec![vec![c, cur.point]],
            (Some((c, _)), Some(s)) => match self.arc(cur.point) {
                Some(a) => {
                    let mut v = vec![a.points(), vec![c, s]];
                    if self.pie {
                        v.push(vec![c, *a.points().last().expect("arc has points")]);
                    }
                    v
                }
                None => vec![vec![c, s]],
            },
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.center = None;
        self.start = None;
    }

    fn lock_plane(&mut self, axis: Axis) -> bool {
        self.lock = if self.lock == Some(axis.dir()) {
            None
        } else {
            Some(axis.dir())
        };
        true
    }
}

#[derive(Debug, Default)]
pub struct ThreePointArc {
    points: Vec<DVec3>,
    segments: Option<usize>,
}

impl DrawTool for ThreePointArc {
    fn anchor(&self) -> Option<DVec3> {
        self.points.last().copied()
    }

    fn status(&self) -> String {
        match self.points.len() {
            0 => "Select start point.",
            1 => "Select second point.",
            _ => "Select end point.",
        }
        .into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        (
            "Segments",
            self.segments.unwrap_or(ARC_SEGMENTS).to_string(),
        )
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        if self.points.len() < 2 {
            if self
                .points
                .last()
                .is_none_or(|p| p.distance(inf.point) > mt_kernel::TOL)
            {
                self.points.push(inf.point);
            }
            return false;
        }
        let Some(arc) = arc_three_points(
            self.points[0],
            self.points[1],
            inf.point,
            self.segments.unwrap_or(ARC_SEGMENTS),
        ) else {
            return false;
        };
        add_curve(ents, &arc.points(), Some(arc));
        self.points.clear();
        true
    }

    fn typed(
        &mut self,
        _ents: &mut Entities,
        _cur: Option<&Inference>,
        text: &str,
        _units: LengthUnit,
    ) -> bool {
        if let Some(n) =
            parse_segments(text).or_else(|| text.trim().parse().ok().filter(|n: &usize| *n >= 2))
        {
            self.segments = Some(n);
        }
        false
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match self.points[..] {
            [a] => vec![vec![a, cur.point]],
            [a, b] => {
                match arc_three_points(a, b, cur.point, self.segments.unwrap_or(ARC_SEGMENTS)) {
                    Some(arc) => vec![arc.points()],
                    None => vec![vec![a, b, cur.point]],
                }
            }
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.points.clear();
    }
}

// ------------------------------------------------------------------ Freehand

/// Press and drag to draw a free-form curve.
#[derive(Debug, Default)]
pub struct Freehand {
    points: Vec<DVec3>,
}

/// Minimum spacing between freehand points, in inches (draft).
const FREEHAND_STEP: f64 = 0.5;

impl DrawTool for Freehand {
    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn status(&self) -> String {
        "Click and drag to draw.".into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("Length", String::new())
    }

    fn click(&mut self, _ents: &mut Entities, _inf: &Inference) -> bool {
        false
    }

    fn typed(&mut self, _: &mut Entities, _: Option<&Inference>, _: &str, _: LengthUnit) -> bool {
        false
    }

    fn preview(&self, _cur: &Inference) -> Vec<Vec<DVec3>> {
        if self.points.len() > 1 {
            vec![self.points.clone()]
        } else {
            Vec::new()
        }
    }

    fn cancel(&mut self) {
        self.points.clear();
    }

    fn drag_start(&mut self, _ents: &mut Entities, inf: &Inference) -> bool {
        self.points = vec![inf.point];
        false
    }

    fn drag_move(&mut self, inf: &Inference) {
        if self
            .points
            .last()
            .is_some_and(|p| p.distance(inf.point) >= FREEHAND_STEP)
        {
            self.points.push(inf.point);
        }
    }

    fn drag_end(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        self.drag_move(inf);
        let pts = std::mem::take(&mut self.points);
        if pts.len() < 2 {
            return false;
        }
        add_curve(ents, &pts, None);
        true
    }
}

// ------------------------------------------------------------------ Tape Measure

/// Measures between two points. Started on an edge it makes a guide line parallel to the
/// edge through the end point; otherwise a guide point at the end. Typing a distance
/// places the guide that far away (perpendicular to the edge, or towards the cursor).
#[derive(Debug, Default)]
pub struct TapeMeasure {
    /// Start point and, when started on an edge, the edge direction.
    start: Option<(DVec3, Option<DVec3>)>,
}

impl TapeMeasure {
    fn offset(&self, p: DVec3) -> Option<DVec3> {
        let (s, dir) = self.start?;
        let d = p - s;
        Some(match dir {
            Some(e) => d - e * d.dot(e),
            None => d,
        })
    }
}

impl DrawTool for TapeMeasure {
    fn anchor(&self) -> Option<DVec3> {
        self.start.map(|s| s.0)
    }

    fn status(&self) -> String {
        if self.start.is_some() {
            "Select second point to measure or create a guide."
        } else {
            "Select first point."
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let v = cur
            .and_then(|c| self.offset(c.point))
            .map(|o| units.format(o.length()));
        ("Length", v.unwrap_or_default())
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        let Some((s, dir)) = self.start else {
            let dir = inf
                .segment
                .filter(|_| inf.kind == mt_infer::Kind::OnEdge)
                .map(|[a, b]| (b - a).normalize_or_zero())
                .filter(|d| *d != DVec3::ZERO);
            self.start = Some((inf.point, dir));
            return false;
        };
        self.start = None;
        if s.distance(inf.point) <= mt_kernel::TOL {
            return false;
        }
        ents.guides.push(mt_kernel::Guide {
            point: inf.point,
            dir,
        });
        true
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        let (Some(c), Some(d)) = (cur, parse_length(text, units)) else {
            return false;
        };
        let (Some((s, _)), Some(away)) = (self.start, self.offset(c.point)) else {
            return false;
        };
        let away = away.normalize_or_zero();
        if away == DVec3::ZERO {
            return false;
        }
        let mut at = *c;
        at.point = s + away * d;
        self.click(ents, &at)
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        self.start
            .map(|(s, _)| vec![vec![s, cur.point]])
            .unwrap_or_default()
    }

    fn cancel(&mut self) {
        self.start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_infer::Kind;

    fn inf(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: Kind::Free,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
        }
    }

    fn v(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    #[test]
    fn rectangle_on_ground_and_vertical() {
        let c = rectangle_corners(v(0.0, 0.0, 0.0), v(10.0, 5.0, 0.0), DVec3::Z);
        assert_eq!(c[2], v(10.0, 5.0, 0.0));
        assert_eq!(c[1], v(10.0, 0.0, 0.0));
        let c = rectangle_corners(v(0.0, 0.0, 0.0), v(10.0, 0.0, 7.0), DVec3::Z);
        assert_eq!(
            c,
            [
                v(0.0, 0.0, 0.0),
                v(10.0, 0.0, 0.0),
                v(10.0, 0.0, 7.0),
                v(0.0, 0.0, 7.0)
            ]
        );
    }

    #[test]
    fn rectangle_tool_makes_a_face_and_takes_dimensions() {
        let mut e = Entities::new();
        let mut t = Rectangle::default();
        assert!(!t.click(&mut e, &inf(v(0.0, 0.0, 0.0))));
        assert!(t.click(&mut e, &inf(v(10.0, 5.0, 0.0))));
        assert_eq!(e.faces.len(), 1);
        t.click(&mut e, &inf(v(20.0, 0.0, 0.0)));
        assert!(t.typed(
            &mut e,
            Some(&inf(v(25.0, 1.0, 0.0))),
            "4,3",
            LengthUnit::Inch
        ));
        assert!(e.vertices.values().any(|x| x.pos == v(24.0, 3.0, 0.0)));
    }

    #[test]
    fn circle_tool_makes_24_segment_face_and_takes_sides() {
        let mut e = Entities::new();
        let mut t = Polygon::circle();
        t.click(&mut e, &inf(v(0.0, 0.0, 0.0)));
        assert!(t.click(&mut e, &inf(v(10.0, 0.0, 0.0))));
        assert_eq!((e.edges.len(), e.faces.len(), e.curves.len()), (24, 1, 1));
        let mut t = Polygon::regular();
        t.typed(&mut e, None, "8", LengthUnit::Inch);
        t.click(&mut e, &inf(v(100.0, 0.0, 0.0)));
        assert!(t.typed(
            &mut e,
            Some(&inf(v(110.0, 0.0, 0.0))),
            "5",
            LengthUnit::Inch
        ));
        assert_eq!(e.faces.len(), 2);
        assert_eq!(e.edges.len(), 24 + 8);
    }

    #[test]
    fn two_point_arc_bulge() {
        let arc = arc_from_bulge(v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0), 5.0, DVec3::Z, 12).unwrap();
        // Bulge of half the chord: a half circle of radius 5 around (5, 0).
        assert!((arc.radius - 5.0).abs() < 1e-9);
        assert!((arc.center - v(5.0, 0.0, 0.0)).length() < 1e-9);
        let pts = arc.points();
        assert!((pts[0] - v(0.0, 0.0, 0.0)).length() < 1e-9);
        assert!((pts[12] - v(10.0, 0.0, 0.0)).length() < 1e-9);
        assert!(
            (pts[6] - v(5.0, 5.0, 0.0)).length() < 1e-9,
            "apex on the bulge side"
        );
        let flipped =
            arc_from_bulge(v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0), -2.0, DVec3::Z, 12).unwrap();
        assert!(flipped.points()[6].y < 0.0);
    }

    #[test]
    fn three_point_arc_passes_through_middle() {
        let arc =
            arc_three_points(v(10.0, 0.0, 0.0), v(0.0, 10.0, 0.0), v(-10.0, 0.0, 0.0), 12).unwrap();
        assert!(arc.center.length() < 1e-9);
        assert!((arc.radius - 10.0).abs() < 1e-9);
        assert!((arc.points()[6] - v(0.0, 10.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn pie_closes_into_a_face() {
        let mut e = Entities::new();
        let mut t = CenterArc::new(true);
        t.click(&mut e, &inf(v(0.0, 0.0, 0.0)));
        t.click(&mut e, &inf(v(10.0, 0.0, 0.0)));
        assert!(t.click(&mut e, &inf(v(0.0, 10.0, 0.0))));
        assert_eq!(e.faces.len(), 1);
        assert_eq!(e.edges.len(), 12 + 2);
    }

    #[test]
    fn tape_measure_makes_parallel_guide_and_guide_point() {
        let mut e = Entities::new();
        let mut t = TapeMeasure::default();
        let mut on_edge = inf(v(5.0, 0.0, 0.0));
        on_edge.kind = Kind::OnEdge;
        on_edge.segment = Some([v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0)]);
        t.click(&mut e, &on_edge);
        assert!(t.typed(&mut e, Some(&inf(v(7.0, 3.0, 0.0))), "2", LengthUnit::Inch));
        assert_eq!(e.guides.len(), 1);
        assert_eq!(e.guides[0].dir, Some(DVec3::X));
        assert!((e.guides[0].point - v(5.0, 2.0, 0.0)).length() < 1e-12);
        let mut endpoint = inf(v(10.0, 0.0, 0.0));
        endpoint.kind = Kind::Endpoint;
        t.click(&mut e, &endpoint);
        assert!(t.click(&mut e, &inf(v(10.0, 4.0, 0.0))));
        assert_eq!(e.guides[1].dir, None);
    }

    #[test]
    fn segment_counts() {
        assert_eq!(parse_segments("48s"), Some(48));
        assert_eq!(parse_segments("2s"), None);
        assert_eq!(parse_segments("48"), None);
    }
}
