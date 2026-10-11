//! Drawing tools: Line, Rectangle, Rotated Rectangle, Circle, Polygon, Arc, 2 Point Arc,
//! 3 Point Arc, Pie and Freehand, and the construction tools Tape Measure, Protractor,
//! Dimensions and Text. Each takes inferred points (clicks, or press-drag-
//! release) and typed values from the Measurements box, and shows a preview.
//!
//! Shapes lie in the plane of the face clicked first, else on the ground; arrow keys
//! lock the plane (right: red normal, left: green, up: blue), also after the first point
//! of a Rectangle, Circle or Polygon, and Shift held over a face locks it to the
//! face's plane ([`PlaneLock`]; key handling in `locks`). Rectangle sides and the default start angle follow
//! [`plane_axes`]: red/green on level planes, else the level line and the fall line
//! (horizontal and vertical on walls). Behaviors are drafts from
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
    /// Shift held over a face locks the drawing plane to the face's (`Some(normal)`);
    /// `None` when Shift is released. Returns whether the tool has a plane to lock.
    fn lock_face_plane(&mut self, _normal: Option<DVec3>) -> bool {
        false
    }
    /// The tool works in a plane and Alt frees its center point while that plane
    /// stays locked (Rotate, Protractor).
    fn alt_frees_center(&self) -> bool {
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
    /// The cursor snapped to a shape hint (Square, Half Circle...; `mt_infer::hints`).
    fn hint(&self, _cur: &Inference) -> Option<Inference> {
        None
    }
    /// The text being written (Text tool), for the inline editor.
    fn text_draft(&mut self) -> Option<&mut TextDraft> {
        None
    }
    /// Adds the text being written to the model. Returns whether the model changed.
    fn finish_text(&mut self, _ents: &mut Entities) -> bool {
        false
    }
    /// Ctrl pressed (Tape Measure cycles its mode).
    fn toggle_ctrl(&mut self) {}
    /// A double click (after the click it also is). Returns whether the model changed.
    fn double_click(&mut self, _ents: &mut Entities, _inf: &Inference) -> bool {
        false
    }
    /// What is under the cursor before a click (the Text tool's component name).
    fn hover(&mut self, _h: &DrawHover) {}
    /// Ctrl+plus / Ctrl+minus: more or fewer segments (arcs, circles, polygons,
    /// Freehand). Returns whether the tool used it.
    fn step_segments(&mut self, _more: bool) -> bool {
        false
    }
    /// Tape Measure: a length typed after a measurement, to rescale the model: the
    /// measured length and the one wanted (inches).
    fn take_rescale(&mut self) -> Option<(f64, f64)> {
        None
    }
}

/// What a drawing tool sees under the cursor before a click.
pub struct DrawHover {
    /// The group's or component's name under the cursor (instance name, else the
    /// definition's), when the cursor is over one.
    pub instance_name: Option<String>,
    pub units: LengthUnit,
}

/// A typed point: `[x,y,z]` absolute (context coordinates) or `<x,y,z>` relative to
/// the last point (SketchUp's Measurements box syntax for Line and Move [doc]; that
/// absolute coordinates follow the drawing axes rather than the open context's is
/// not done, unverified which SketchUp uses).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coords {
    Absolute(DVec3),
    Relative(DVec3),
}

/// Parses `[x,y,z]` or `<x,y,z>` (`;` also separates, for decimal commas). Each
/// value is a length in the model's units.
pub fn parse_coords(text: &str, units: LengthUnit) -> Option<Coords> {
    let t = text.trim();
    let (inner, abs) = if let Some(x) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        (x, true)
    } else {
        (t.strip_prefix('<')?.strip_suffix('>')?, false)
    };
    let sep = if inner.contains(';') { ';' } else { ',' };
    let v: Vec<f64> = inner
        .split(sep)
        .map(|p| parse_length(p.trim(), units))
        .collect::<Option<_>>()?;
    let [x, y, z] = v[..] else { return None };
    let p = DVec3::new(x, y, z);
    Some(if abs {
        Coords::Absolute(p)
    } else {
        Coords::Relative(p)
    })
}

/// Segment settings of the arc tools: a fixed count (`12s`), or a count taken from
/// a circle of `n` sides (`20c`: an arc gets the share of the 20 its sweep covers
/// [doc]). Ctrl+plus / minus change the count by one (step unverified).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ArcSegments {
    fixed: Option<usize>,
    circle: Option<usize>,
}

impl ArcSegments {
    /// Segments for an arc sweeping `sweep` radians.
    pub fn count(&self, sweep: f64) -> usize {
        match self.circle {
            Some(n) => ((n as f64 * sweep.abs() / TAU).round() as usize).max(1),
            None => self.fixed.unwrap_or(ARC_SEGMENTS),
        }
    }

    /// `12s` or `20c`; returns whether `text` was one.
    pub fn parse(&mut self, text: &str) -> bool {
        if let Some(n) = parse_segments(text) {
            *self = Self {
                fixed: Some(n),
                circle: None,
            };
            return true;
        }
        let circle = text
            .trim()
            .strip_suffix(['c', 'C'])
            .and_then(|n| n.trim().parse::<usize>().ok())
            .filter(|n| (3..=999).contains(n));
        if let Some(n) = circle {
            *self = Self {
                fixed: None,
                circle: Some(n),
            };
        }
        circle.is_some()
    }

    pub fn step(&mut self, more: bool) {
        let step = |n: usize| {
            if more {
                (n + 1).min(999)
            } else {
                n.saturating_sub(1).max(2)
            }
        };
        match &mut self.circle {
            Some(n) => *n = step(*n).max(3),
            None => self.fixed = Some(step(self.fixed.unwrap_or(ARC_SEGMENTS))),
        }
    }

    fn apply(&self, mut a: Arc) -> Arc {
        a.segments = self.count(a.end_angle - a.start_angle);
        a
    }

    fn label(&self) -> String {
        match self.circle {
            Some(n) => format!("{n}c"),
            None => self.fixed.unwrap_or(ARC_SEGMENTS).to_string(),
        }
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
        "tool.protractor" => Box::new(Protractor::default()),
        "tool.dimensions" => Box::new(DimensionTool::default()),
        "tool.text" => Box::new(TextTool::default()),
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

/// A plane tool's locked plane normal: an arrow key's axis (pressed again to unlock)
/// or, while Shift is held over a face, that face's normal, which wins. The face
/// lock is taken into the shape's plane when its first point is placed, so the shape
/// stays on that plane after Shift is released.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PlaneLock {
    axis: Option<DVec3>,
    face: Option<DVec3>,
}

impl PlaneLock {
    pub fn get(&self) -> Option<DVec3> {
        self.face.or(self.axis)
    }

    fn toggle(&mut self, axis: Axis) {
        self.axis = if self.axis == Some(axis.dir()) {
            None
        } else {
            Some(axis.dir())
        };
    }
}

/// Plane normal for a shape started at `inf`: the locked one, the face's, else up.
fn plane_normal(lock: Option<DVec3>, inf: &Inference) -> DVec3 {
    lock.or(inf.normal).unwrap_or(DVec3::Z).normalize()
}

/// In-plane axes `(u, v)` of a drawing plane, with `u x v = normal`.
///
/// SketchUp orients shapes on a face by the model axes: on a horizontal plane `u` is
/// red and `v` green (`-green` facing down); on any other plane `u` is the plane's
/// level line (`blue x normal`, horizontal) and `v` runs up the slope (straight up on
/// a vertical wall). So a rectangle on a pitched roof has two level sides and two
/// sides down the fall line, and on a wall facing red its sides run green and blue.
/// This matches the Ruby API's `Vector3d#axes` (the "arbitrary axis" algorithm) up to
/// its near-vertical threshold; that the Rectangle tool uses exactly these axes on
/// skewed slopes is from memory of SketchUp (unverified).
pub fn plane_axes(normal: DVec3) -> (DVec3, DVec3) {
    let n = normal.normalize_or_zero();
    let level = DVec3::Z.cross(n);
    let u = if level.length() < 1e-6 {
        // Horizontal plane (or a degenerate normal): red.
        (DVec3::X - n * DVec3::X.dot(n)).normalize_or(DVec3::X)
    } else {
        level.normalize()
    };
    (u, n.cross(u))
}

/// Whether the plane is horizontal or vertical, where rectangles may stand up off it.
fn is_level_or_plumb(normal: DVec3) -> bool {
    let z = normal.normalize_or_zero().z.abs();
    z < 1e-6 || z > 1.0 - 1e-9
}

/// Corners of an axis-aligned rectangle with opposite corners `a` and `b`, its sides
/// along [`plane_axes`]. When `b` is off a horizontal or vertical plane (inferred
/// straight up, or out of a wall), the rectangle stands perpendicular to the plane
/// through `a`, as in SketchUp; off a sloped plane `b` is projected onto it, so the
/// rectangle keeps the roof's orientation.
pub fn rectangle_corners(a: DVec3, b: DVec3, normal: DVec3) -> [DVec3; 4] {
    let mut d = b - a;
    let off_plane = d.dot(normal);
    if off_plane.abs() > mt_kernel::TOL {
        let along = d - normal * off_plane;
        if !is_level_or_plumb(normal) {
            d = along;
        } else if along.length() > mt_kernel::TOL {
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
        match parse_coords(text, units) {
            // Without a start point, either form is where the line starts.
            Some(Coords::Absolute(p) | Coords::Relative(p)) if self.anchor.is_none() => {
                self.anchor = Some(p);
                return false;
            }
            Some(Coords::Absolute(p)) => return self.segment(ents, p),
            Some(Coords::Relative(v)) => {
                let a = self.anchor.unwrap_or_default();
                return self.segment(ents, a + v);
            }
            None => {}
        }
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
    /// First corner and the normal of the face it was placed on (up off faces).
    first: Option<(DVec3, DVec3)>,
    lock: PlaneLock,
}

impl Rectangle {
    /// First corner and the drawing plane's normal: the locked axis, else the face's.
    /// An arrow key pressed after the first corner re-orients the rectangle.
    fn plane(&self) -> Option<(DVec3, DVec3)> {
        let (a, n) = self.first?;
        Some((a, self.lock.get().unwrap_or(n).normalize()))
    }

    fn corners(&self, b: DVec3) -> Option<[DVec3; 4]> {
        let (a, n) = self.plane()?;
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
    fn hint(&self, cur: &Inference) -> Option<Inference> {
        let (a, n) = self.plane()?;
        let (u, v) = plane_axes(n);
        mt_infer::hints::rectangle(cur, a, u, v)
    }

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
                self.first = Some((inf.point, plane_normal(self.lock.face, inf)));
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
        let (Some((a, n)), Some((w, h))) = (self.plane(), parse_pair(text, units)) else {
            return false;
        };
        // `w` runs along the plane's first axis (red, or the level line on a slope) and
        // `h` along the second, or along the sides of the rectangle standing up off the
        // plane; signs follow the side the cursor is on, as in SketchUp.
        let (u, v) = plane_axes(n);
        let (mut e1, mut e2) = (u, v);
        if let Some(k) = cur.map(|c| rectangle_corners(a, c.point, n)) {
            e1 = (k[1] - k[0]).normalize_or(u);
            e2 = (k[3] - k[0]).normalize_or(v);
        }
        self.make(ents, [a, a + e1 * w, a + e1 * w + e2 * h, a + e2 * h])
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
        self.lock.toggle(axis);
        true
    }

    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool {
        self.lock.face = normal;
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
    /// Center and the normal of the face it was placed on (up off faces).
    center: Option<(DVec3, DVec3)>,
    lock: PlaneLock,
}

impl Polygon {
    pub fn circle() -> Self {
        Self {
            is_circle: true,
            sides: CIRCLE_SIDES,
            center: None,
            lock: PlaneLock::default(),
        }
    }

    pub fn regular() -> Self {
        Self {
            is_circle: false,
            sides: POLYGON_SIDES,
            center: None,
            lock: PlaneLock::default(),
        }
    }

    /// The start angle follows the cursor (the first vertex points at it); with the
    /// cursor on the center it is the plane's first axis (`plane_axes`).
    fn arc(&self, toward: DVec3, radius: Option<f64>) -> Option<Arc> {
        let (c, n) = self.center?;
        // An arrow key pressed after the center re-orients the shape.
        let n = self.lock.get().unwrap_or(n).normalize();
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
            self.center = Some((inf.point, plane_normal(self.lock.face, inf)));
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
        self.lock.toggle(axis);
        true
    }

    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool {
        self.lock.face = normal;
        true
    }

    fn step_segments(&mut self, more: bool) -> bool {
        self.sides = if more {
            (self.sides + 1).min(999)
        } else {
            self.sides.saturating_sub(1).max(3)
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
    /// Direction of the edge ending at the start point, for "Tangent at Vertex".
    tangent: Option<DVec3>,
    segments: ArcSegments,
    lock: PlaneLock,
    /// Radius of the last arc, repeated by a double click on a corner.
    last_radius: Option<f64>,
}

impl TwoPointArc {
    /// A double click on a corner where two straight edges meet: the last arc's radius
    /// rounds it (a fillet tangent to both edges) and the corner's waste goes, as
    /// SketchUp's "double-click to repeat the 2-point arc" [doc] (exact cleanup
    /// unverified).
    fn fillet(&mut self, ents: &mut Entities, at: DVec3) -> bool {
        let Some(r) = self.last_radius else {
            return false;
        };
        let Some(v) = ents.find_vertex(at) else {
            return false;
        };
        let edges: Vec<_> = ents
            .edges
            .iter()
            .filter(|(_, e)| e.start == v || e.end == v)
            .map(|(id, e)| (id, ents.pos(e.other(v))))
            .collect();
        let [(_, p1), (_, p2)] = edges[..] else {
            return false;
        };
        let Some((t1, t2, arc)) = fillet_arc(ents.pos(v), p1, p2, r, self.segments) else {
            return false;
        };
        add_curve(ents, &arc.points(), Some(arc));
        // The edges from the tangent points to the corner are the waste.
        for t in [t1, t2] {
            if let (Some(a), Some(b)) = (ents.find_vertex(t), ents.find_vertex(at))
                && let Some(e) = ents.edge_between(a, b)
            {
                mt_kernel::erase_edge(ents, e);
            }
        }
        true
    }

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
        arc_from_bulge(a, b, bulge, self.normal, ARC_SEGMENTS).map(|x| self.segments.apply(x))
    }

    fn make(&mut self, ents: &mut Entities, bulge: f64) -> bool {
        let [a, b] = self.points[..] else {
            return false;
        };
        match self.arc(bulge) {
            Some(arc) => {
                self.last_radius = Some(arc.radius);
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
    fn hint(&self, cur: &Inference) -> Option<Inference> {
        let [a, b] = self.points[..] else { return None };
        mt_infer::hints::two_point_arc(cur, a, b, self.normal, self.tangent)
    }

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
                self.normal = plane_normal(self.lock.get(), inf);
                self.tangent = mt_infer::hints::tangent_at(inf);
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
        if self.segments.parse(text) {
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
        self.lock.toggle(axis);
        true
    }

    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool {
        self.lock.face = normal;
        true
    }

    fn step_segments(&mut self, more: bool) -> bool {
        self.segments.step(more);
        true
    }

    fn double_click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        // The double click's first click started an arc at the corner.
        if self.points.len() == 1 && self.points[0].distance(inf.point) <= mt_kernel::TOL {
            self.points.clear();
            return self.fillet(ents, inf.point);
        }
        false
    }
}

/// The fillet of radius `r` at corner `v` between the lines to `p1` and `p2`: the
/// tangent points on each line and the arc between them; `None` when the lines are
/// collinear or too short for the radius.
pub fn fillet_arc(
    v: DVec3,
    p1: DVec3,
    p2: DVec3,
    r: f64,
    segments: ArcSegments,
) -> Option<(DVec3, DVec3, Arc)> {
    let (d1, d2) = ((p1 - v).normalize_or_zero(), (p2 - v).normalize_or_zero());
    let theta = d1.dot(d2).clamp(-1.0, 1.0).acos();
    if !(1e-6..std::f64::consts::PI - 1e-6).contains(&theta) || r <= mt_kernel::TOL {
        return None;
    }
    let along = r / (theta / 2.0).tan();
    if along >= v.distance(p1) - mt_kernel::TOL || along >= v.distance(p2) - mt_kernel::TOL {
        return None;
    }
    let (t1, t2) = (v + d1 * along, v + d2 * along);
    let center = v + (d1 + d2).normalize() * (r / (theta / 2.0).sin());
    let n = (t1 - center).cross(t2 - center).normalize_or_zero();
    if n == DVec3::ZERO {
        return None;
    }
    let x = (t1 - center).normalize();
    let sweep = angle_of(t2, center, x, n);
    let arc = Arc {
        center,
        normal: n,
        x_axis: x,
        radius: r,
        start_angle: 0.0,
        end_angle: sweep,
        segments: ARC_SEGMENTS,
    };
    Some((t1, t2, segments.apply(arc)))
}

/// Arc (center, start, end) or Pie (the same, closed with two radii into a face).
#[derive(Debug)]
pub struct CenterArc {
    pie: bool,
    center: Option<(DVec3, DVec3)>,
    start: Option<DVec3>,
    segments: ArcSegments,
    lock: PlaneLock,
}

impl CenterArc {
    pub fn new(pie: bool) -> Self {
        Self {
            pie,
            center: None,
            start: None,
            segments: ArcSegments::default(),
            lock: PlaneLock::default(),
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
            segments: self.segments.count(sweep),
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
    fn hint(&self, cur: &Inference) -> Option<Inference> {
        let ((c, n), s) = (self.center?, self.start?);
        mt_infer::hints::center_arc(cur, c, s, n)
    }

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
                self.center = Some((inf.point, plane_normal(self.lock.get(), inf)));
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
        if self.segments.parse(text) {
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
                let a = self.segments.apply(a);
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
        self.lock.toggle(axis);
        true
    }

    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool {
        self.lock.face = normal;
        true
    }

    fn step_segments(&mut self, more: bool) -> bool {
        self.segments.step(more);
        true
    }
}

#[derive(Debug, Default)]
pub struct ThreePointArc {
    points: Vec<DVec3>,
    segments: ArcSegments,
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
        ("Segments", self.segments.label())
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
        let Some(arc) = arc_three_points(self.points[0], self.points[1], inf.point, ARC_SEGMENTS)
            .map(|a| self.segments.apply(a))
        else {
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
        if !self.segments.parse(text)
            && let Some(n) = text.trim().parse().ok().filter(|n: &usize| *n >= 2)
        {
            self.segments.parse(&format!("{n}s"));
        }
        false
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match self.points[..] {
            [a] => vec![vec![a, cur.point]],
            [a, b] => {
                match arc_three_points(a, b, cur.point, ARC_SEGMENTS)
                    .map(|x| self.segments.apply(x))
                {
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

    fn step_segments(&mut self, more: bool) -> bool {
        self.segments.step(more);
        true
    }
}

// ------------------------------------------------------------------ Freehand

/// Press and drag to draw a free-form curve. Ctrl+plus / minus make the points
/// closer or farther apart (finer or coarser curves; what SketchUp's Ctrl+/- changes
/// in Freehand is unverified).
#[derive(Debug)]
pub struct Freehand {
    points: Vec<DVec3>,
    step: f64,
}

impl Default for Freehand {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            step: FREEHAND_STEP,
        }
    }
}

/// Minimum spacing between freehand points, in inches (draft).
const FREEHAND_STEP: f64 = 0.5;

impl DrawTool for Freehand {
    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn status(&self) -> String {
        format!(
            "Click and drag to draw. Ctrl+plus / minus: finer or coarser ({:.2}\" spacing).",
            self.step
        )
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
            .is_some_and(|p| p.distance(inf.point) >= self.step)
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

    fn step_segments(&mut self, more: bool) -> bool {
        self.step = if more {
            self.step / 2.0
        } else {
            self.step * 2.0
        }
        .clamp(0.05, 8.0);
        true
    }
}

// ------------------------------------------------------------------ Tape Measure

/// What the Tape Measure makes at its second click; Ctrl cycles the modes (SketchUp
/// 2026.1 [doc]: Guide Line, Guide Point, Measure; the order and the status text are
/// unverified).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TapeMode {
    /// A guide line parallel to the edge it started on, else a guide point.
    #[default]
    GuideLine,
    /// Always a guide point at the end.
    GuidePoint,
    /// Only measures.
    Measure,
}

impl TapeMode {
    fn next(self) -> Self {
        match self {
            Self::GuideLine => Self::GuidePoint,
            Self::GuidePoint => Self::Measure,
            Self::Measure => Self::GuideLine,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::GuideLine => "Guide Line",
            Self::GuidePoint => "Guide Point",
            Self::Measure => "Measure",
        }
    }
}

/// Measures between two points. Started on an edge it makes a guide line parallel to the
/// edge through the end point; otherwise a guide point at the end (see [`TapeMode`]).
/// Typing a distance places the guide that far away (perpendicular to the edge, or
/// towards the cursor); typed after a measurement, it rescales the model by the
/// ratio (the app asks first). A double click drops a guide point.
#[derive(Debug, Default)]
pub struct TapeMeasure {
    /// Start point and, when started on an edge, the edge direction.
    start: Option<(DVec3, Option<DVec3>)>,
    mode: TapeMode,
    /// The last measured length, which a typed length rescales.
    last: Option<f64>,
    rescale: Option<(f64, f64)>,
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
        let step = if self.start.is_some() {
            "Select second point to measure or create a guide."
        } else if self.last.is_some() {
            "Select first point, or type a length to resize the model."
        } else {
            "Select first point."
        };
        format!("{step} Ctrl = mode ({}).", self.mode.label())
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let v = cur
            .and_then(|c| self.offset(c.point))
            .map(|o| units.format(o.length()))
            .or_else(|| self.last.map(|l| units.format(l)));
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
        let measured = self.offset(inf.point).map_or(0.0, |o| o.length());
        self.start = None;
        if s.distance(inf.point) <= mt_kernel::TOL {
            return false;
        }
        self.last = (measured > mt_kernel::TOL).then_some(measured);
        let dir = match self.mode {
            TapeMode::GuideLine => dir,
            TapeMode::GuidePoint => None,
            TapeMode::Measure => return false,
        };
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
        let Some(d) = parse_length(text, units) else {
            return false;
        };
        if self.start.is_none() {
            if let Some(m) = self.last
                && d > mt_kernel::TOL
            {
                self.rescale = Some((m, d));
            }
            return false;
        }
        let Some(c) = cur else {
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

    fn toggle_ctrl(&mut self) {
        self.mode = self.mode.next();
    }

    fn double_click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        // The double click's two clicks measured nothing: drop a guide point.
        self.start = None;
        ents.guides.push(mt_kernel::Guide {
            point: inf.point,
            dir: None,
        });
        true
    }

    fn take_rescale(&mut self) -> Option<(f64, f64)> {
        self.rescale.take()
    }
}

// ------------------------------------------------------------------ Protractor

/// Signed angle from `from` to `to` around `normal` (all in the plane), in
/// `(-pi, pi]`.
pub fn signed_angle(from: DVec3, to: DVec3, normal: DVec3) -> f64 {
    from.cross(to).dot(normal).atan2(from.dot(to))
}

/// A typed protractor angle in radians: degrees (`30`, `-12.5°`) or a slope
/// `rise:run` (`1:2`).
pub fn parse_angle(text: &str) -> Option<f64> {
    let t = text.trim();
    if let Some((rise, run)) = t.split_once(':') {
        let (rise, run): (f64, f64) = (rise.trim().parse().ok()?, run.trim().parse().ok()?);
        return (rise != 0.0 || run != 0.0).then(|| rise.atan2(run));
    }
    let deg: f64 = t
        .trim_end_matches('°')
        .trim()
        .replace(',', ".")
        .parse()
        .ok()?;
    deg.is_finite().then(|| deg.to_radians())
}

/// Protractor: center (the protractor lies on the face under the cursor, else on the
/// ground; arrow keys lock it), a click on the start direction, then a click or a
/// typed angle places a guide line through the center at that angle.
///
/// Unverified against SketchUp: the protractor here is drawn at the radius of the
/// start click (SketchUp's is a fixed screen size), angles do not snap to the 15°
/// ticks, typed angles turn towards the side the cursor is on, and Ctrl (measure
/// only, no guide) is not supported.
#[derive(Debug, Default)]
pub struct Protractor {
    center: Option<(DVec3, DVec3)>,
    /// Unit start direction in the plane, and the protractor radius.
    base: Option<(DVec3, f64)>,
    lock: PlaneLock,
}

/// Protractor ticks every 15 degrees (SketchUp's default angle snap).
const PROTRACTOR_TICK: f64 = std::f64::consts::PI / 12.0;

impl Protractor {
    fn in_plane(&self, p: DVec3) -> Option<DVec3> {
        let (c, n) = self.center?;
        let d = p - c;
        Some(d - n * d.dot(n))
    }

    /// Angle of the cursor from the start direction.
    fn angle(&self, p: DVec3) -> Option<f64> {
        let (_, n) = self.center?;
        let (base, _) = self.base?;
        let d = self.in_plane(p)?;
        (d.length() > mt_kernel::TOL).then(|| signed_angle(base, d, n))
    }

    /// The guide direction at `angle` from the start direction.
    pub fn direction(&self, angle: f64) -> Option<DVec3> {
        let (_, n) = self.center?;
        let (base, _) = self.base?;
        Some(base * angle.cos() + n.cross(base) * angle.sin())
    }

    fn place(&mut self, ents: &mut Entities, angle: f64) -> bool {
        let (Some((c, _)), Some(dir)) = (self.center, self.direction(angle)) else {
            return false;
        };
        ents.guides.push(mt_kernel::Guide {
            point: c,
            dir: Some(dir.normalize()),
        });
        self.cancel();
        true
    }
}

impl DrawTool for Protractor {
    fn anchor(&self) -> Option<DVec3> {
        self.center.map(|c| c.0)
    }

    fn status(&self) -> String {
        match (self.center, self.base) {
            (None, _) => "Place the protractor center. Arrow keys or Shift on a face lock its orientation; Alt frees the center.",
            (Some(_), None) => "Select the start of the angle.",
            _ => "Select the angle or enter value (degrees or rise:run).",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        let v = cur
            .and_then(|c| self.angle(c.point))
            .map(|a| format!("{:.1}", a.to_degrees()));
        ("Angle", v.unwrap_or_default())
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        match (self.center, self.base) {
            (None, _) => {
                self.center = Some((inf.point, plane_normal(self.lock.get(), inf)));
                false
            }
            (Some(_), None) => {
                let d = self.in_plane(inf.point).unwrap_or_default();
                if d.length() > mt_kernel::TOL {
                    self.base = Some((d.normalize(), d.length()));
                }
                false
            }
            _ => match self.angle(inf.point) {
                Some(a) => self.place(ents, a),
                None => false,
            },
        }
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        _units: LengthUnit,
    ) -> bool {
        let (Some(_), Some(a)) = (self.base, parse_angle(text)) else {
            return false;
        };
        let side = cur
            .and_then(|c| self.angle(c.point))
            .map_or(1.0, |c| if c < 0.0 { -1.0 } else { 1.0 });
        self.place(ents, a * side)
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        let Some((c, n)) = self.center else {
            return Vec::new();
        };
        let to_cur = self.in_plane(cur.point).unwrap_or_default();
        let (x, r) = match self.base {
            Some(b) => b,
            None if to_cur.length() > mt_kernel::TOL => (to_cur.normalize(), to_cur.length()),
            None => return Vec::new(),
        };
        let circle = Arc {
            center: c,
            normal: n,
            x_axis: x,
            radius: r,
            start_angle: 0.0,
            end_angle: TAU,
            segments: 48,
        };
        let y = n.cross(x);
        let mut out = vec![circle.points()];
        for i in 0..24 {
            let a = i as f64 * PROTRACTOR_TICK;
            let d = x * a.cos() + y * a.sin();
            let inner = if i % 6 == 0 { 0.8 } else { 0.9 };
            out.push(vec![c + d * r * inner, c + d * r]);
        }
        out.push(vec![c, c + x * r]);
        if let (Some(_), Some(a)) = (self.base, self.angle(cur.point)) {
            let d = self.direction(a).unwrap_or(x);
            out.push(vec![c, c + d * to_cur.length().max(r * 1.5)]);
        }
        out
    }

    fn cancel(&mut self) {
        self.center = None;
        self.base = None;
    }

    fn lock_plane(&mut self, axis: Axis) -> bool {
        if self.base.is_some() {
            return false;
        }
        self.lock.toggle(axis);
        if let Some((c, _)) = self.center {
            self.center = Some((c, self.lock.get().unwrap_or(DVec3::Z)));
        }
        true
    }

    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool {
        if self.base.is_some() {
            return false;
        }
        self.lock.face = normal;
        if let (Some((c, _)), Some(n)) = (self.center, normal) {
            self.center = Some((c, n));
        }
        true
    }

    fn alt_frees_center(&self) -> bool {
        self.center.is_none()
    }
}

// ------------------------------------------------------------------ Dimensions

/// The arc an edge at `segment` belongs to, if it is part of an arc curve.
fn arc_of(ents: &Entities, segment: [DVec3; 2]) -> Option<Arc> {
    let a = ents.find_vertex(segment[0])?;
    let b = ents.find_vertex(segment[1])?;
    let e = ents.edge_between(a, b)?;
    ents.curves.get(ents.edges[e].attrs.curve?)?.arc
}

/// Dimensions: click two points (or one edge), then the offset of the dimension line.
/// A click on an arc makes a radius dimension, on a circle a diameter dimension.
/// Typed text while placing becomes the dimension's text.
///
/// Unverified against SketchUp: the offset is taken perpendicular to the measured
/// segment towards the cursor in 3D (SketchUp picks the dimension plane from the
/// view), and the radial dimension's points (center and the clicked side) are a draft.
#[derive(Debug, Default)]
pub struct DimensionTool {
    first: Option<DVec3>,
    placing: Option<mt_kernel::Dimension>,
}

impl DimensionTool {
    /// `d` with its offset following the cursor.
    fn placed(d: &mt_kernel::Dimension, cursor: DVec3) -> mt_kernel::Dimension {
        let mut d = d.clone();
        d.offset = match d.kind {
            mt_kernel::DimensionKind::Linear => {
                mt_kernel::Dimension::offset_towards(d.a, d.b, cursor)
            }
            _ => cursor - d.b,
        };
        d
    }

    /// The dimension started by a click on an edge.
    fn from_edge(ents: &Entities, seg: [DVec3; 2], at: DVec3) -> mt_kernel::Dimension {
        use mt_kernel::{Dimension, DimensionKind};
        let Some(arc) = arc_of(ents, seg) else {
            return Dimension::linear(seg[0], seg[1], DVec3::ZERO);
        };
        let d = at - arc.center;
        let d = (d - arc.normal * d.dot(arc.normal)).normalize_or_zero();
        let d = if d == DVec3::ZERO { arc.x_axis } else { d };
        let (a, b, kind) = if arc.is_circle() {
            (
                arc.center - d * arc.radius,
                arc.center + d * arc.radius,
                DimensionKind::Diameter,
            )
        } else {
            (
                arc.center,
                arc.center + d * arc.radius,
                DimensionKind::Radius,
            )
        };
        Dimension {
            kind,
            ..Dimension::linear(a, b, DVec3::ZERO)
        }
    }
}

impl DrawTool for DimensionTool {
    fn anchor(&self) -> Option<DVec3> {
        self.first.or(self.placing.as_ref().map(|d| d.b))
    }

    fn status(&self) -> String {
        match (self.first, &self.placing) {
            (_, Some(_)) => "Select the dimension position, or type its text.",
            (Some(_), None) => "Select second point.",
            _ => "Select first point or an edge.",
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let v = match (&self.placing, self.first, cur) {
            (Some(d), _, _) => units.format(d.value()),
            (None, Some(a), Some(c)) => units.format(a.distance(c.point)),
            _ => String::new(),
        };
        ("Length", v)
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        if let Some(d) = self.placing.take() {
            ents.dimensions.push(Self::placed(&d, inf.point));
            return true;
        }
        match self.first {
            Some(a) => {
                if a.distance(inf.point) > mt_kernel::TOL {
                    self.placing = Some(mt_kernel::Dimension::linear(a, inf.point, DVec3::ZERO));
                    self.first = None;
                }
            }
            None => match inf.segment {
                Some(seg) if inf.kind == mt_infer::Kind::OnEdge => {
                    self.placing = Some(Self::from_edge(ents, seg, inf.point));
                }
                _ => self.first = Some(inf.point),
            },
        }
        false
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        cur: Option<&Inference>,
        text: &str,
        _units: LengthUnit,
    ) -> bool {
        let Some(d) = self.placing.take() else {
            return false;
        };
        let mut d = match cur {
            Some(c) => Self::placed(&d, c.point),
            None => d,
        };
        let text = text.trim();
        d.text = (!text.is_empty()).then(|| text.to_owned());
        ents.dimensions.push(d);
        true
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match (&self.placing, self.first) {
            (Some(d), _) => {
                let d = Self::placed(d, cur.point);
                d.segments(0.0).into_iter().map(|s| s.to_vec()).collect()
            }
            (None, Some(a)) => vec![vec![a, cur.point]],
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.first = None;
        self.placing = None;
    }
}

// ------------------------------------------------------------------ Text

/// What a leader text says by default, formatted in the model's units when shown.
#[derive(Debug, Clone, PartialEq)]
pub enum TextLabel {
    /// Edge or curve length.
    Length(f64),
    /// Face area in square inches.
    Area(f64),
    /// Point coordinates.
    Point(DVec3),
    Plain(String),
}

impl TextLabel {
    pub fn format(&self, units: LengthUnit) -> String {
        match self {
            Self::Length(l) => units.format(*l),
            Self::Area(a) => crate::annot::area_label(*a, units),
            Self::Point(p) => crate::annot::point_label(*p, units),
            Self::Plain(s) => s.clone(),
        }
    }

    /// The default text for a click at `inf`: an edge's (or its curve's) length, a
    /// face's area, a point's coordinates; `None` on empty space. A click on a group
    /// or component gives its name instead (see [`TextTool`]; formats unverified).
    pub fn at(ents: &Entities, inf: &Inference) -> Option<Self> {
        use mt_infer::Kind;
        Some(match inf.kind {
            Kind::OnEdge => {
                let [a, b] = inf.segment?;
                let curve = ents
                    .find_vertex(a)
                    .zip(ents.find_vertex(b))
                    .and_then(|(a, b)| ents.edge_between(a, b))
                    .and_then(|e| ents.edges[e].attrs.curve);
                let len = match curve {
                    Some(c) => ents
                        .curve_edges(c)
                        .iter()
                        .map(|&e| {
                            ents.pos(ents.edges[e].start)
                                .distance(ents.pos(ents.edges[e].end))
                        })
                        .sum(),
                    None => a.distance(b),
                };
                Self::Length(len)
            }
            Kind::OnFace => match ents
                .faces
                .keys()
                .find(|&f| ents.face_contains(f, inf.point))
            {
                Some(f) => Self::Area(face_area(ents, f)),
                None => Self::Point(inf.point),
            },
            // Directions and shape hints from the anchor: nothing under the cursor.
            Kind::Free
            | Kind::Along(_)
            | Kind::Parallel
            | Kind::Perpendicular
            | Kind::FromPoint(_) => return None,
            // Endpoints, midpoints, guides, centers, intersections, axes.
            _ => Self::Point(inf.point),
        })
    }
}

/// Area of a face (holes subtracted), in square inches.
pub fn face_area(ents: &Entities, f: mt_kernel::FaceId) -> f64 {
    let plane = ents.face_plane(f);
    let (outer, holes) = ents.face_polygons(f, &plane);
    let area = |p: &[glam::DVec2]| mt_kernel::geom::signed_area2(p).abs() / 2.0;
    area(&outer) - holes.iter().map(|h| area(h)).sum::<f64>()
}

/// A text being written: placed but not yet in the model.
#[derive(Debug, Clone)]
pub struct TextDraft {
    pub text: mt_kernel::Text,
    pub label: TextLabel,
    /// The text as edited; `None` until the editor opens (it starts with `label`).
    pub edited: Option<String>,
    /// A screen text whose position the editor takes from the pointer.
    pub needs_screen_pos: bool,
    /// The editor took keyboard focus.
    pub focused: bool,
}

/// Text: click an entity, then where the label goes, and edit the text (it starts as
/// the entity's length, area or coordinates); a click on empty space makes a screen
/// text. Enter or a click elsewhere finishes, Esc drops it. Text typed into the
/// Measurements box replaces the text and finishes.
///
/// Unverified against SketchUp: the inline editor's look, empty texts being dropped,
/// and the label formats.
#[derive(Debug, Default)]
pub struct TextTool {
    /// The leader's point and the default label, waiting for the label position.
    anchor: Option<(DVec3, TextLabel)>,
    draft: Option<TextDraft>,
    /// The group's or component's name under the cursor: a leader pointing at a
    /// component says its name by default [doc].
    instance_name: Option<String>,
    units: Option<LengthUnit>,
}

impl TextTool {
    /// Adds the draft to the model (unless its text is empty).
    pub fn finish(&mut self, ents: &mut Entities) -> bool {
        let Some(d) = self.draft.take() else {
            return false;
        };
        let text = d.edited.unwrap_or_else(|| d.label.format(LengthUnit::Inch));
        if text.trim().is_empty() || (d.needs_screen_pos && d.text.screen) {
            return false;
        }
        ents.texts.push(mt_kernel::Text { text, ..d.text });
        true
    }
}

impl DrawTool for TextTool {
    fn anchor(&self) -> Option<DVec3> {
        self.anchor.as_ref().map(|a| a.0)
    }

    fn status(&self) -> String {
        match (&self.anchor, &self.draft) {
            (_, Some(_)) => "Type the text. Enter to finish, Esc to cancel.",
            (Some(_), None) => "Select the text position.",
            _ => "Select an entity for leader text, or empty space for screen text.",
        }
        .into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("Text", String::new())
    }

    fn click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        if self.draft.is_some() {
            return self.finish(ents);
        }
        if let Some((point, label)) = self.anchor.take() {
            self.draft = Some(TextDraft {
                text: mt_kernel::Text::leader(point, inf.point - point, String::new()),
                label,
                edited: None,
                needs_screen_pos: false,
                focused: false,
            });
            return false;
        }
        let label = match (inf.container, &self.instance_name) {
            (Some(_), Some(name)) => Some(TextLabel::Plain(name.clone())),
            _ => TextLabel::at(ents, inf),
        };
        match label {
            Some(label) => self.anchor = Some((inf.point, label)),
            None => {
                self.draft = Some(TextDraft {
                    text: mt_kernel::Text::screen(0.0, 0.0, String::new()),
                    label: TextLabel::Plain(String::new()),
                    edited: None,
                    needs_screen_pos: true,
                    focused: false,
                })
            }
        }
        false
    }

    fn typed(
        &mut self,
        ents: &mut Entities,
        _cur: Option<&Inference>,
        text: &str,
        _units: LengthUnit,
    ) -> bool {
        match self.draft.as_mut() {
            Some(d) => {
                d.edited = Some(text.to_owned());
                self.finish(ents)
            }
            None => false,
        }
    }

    fn preview(&self, cur: &Inference) -> Vec<Vec<DVec3>> {
        match (&self.anchor, &self.draft) {
            (Some((p, _)), _) => vec![vec![*p, cur.point]],
            (None, Some(d)) if !d.text.screen => {
                vec![vec![d.text.point, d.text.point + d.text.leader]]
            }
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.anchor = None;
        self.draft = None;
    }

    fn text_draft(&mut self) -> Option<&mut TextDraft> {
        self.draft.as_mut()
    }

    fn finish_text(&mut self, ents: &mut Entities) -> bool {
        self.finish(ents)
    }

    fn hover(&mut self, h: &DrawHover) {
        self.instance_name = h.instance_name.clone();
        self.units = Some(h.units);
    }

    /// A double click on a face: a text with the face's area at the point, without a
    /// leader [doc] (no leader and no arrow is how it is drawn here; unverified).
    fn double_click(&mut self, ents: &mut Entities, inf: &Inference) -> bool {
        let Some(d) = self.draft.as_ref() else {
            return false;
        };
        let TextLabel::Area(area) = d.label else {
            return false;
        };
        if d.text.screen || d.text.leader.length() > mt_kernel::TOL {
            return false;
        }
        self.draft = None;
        ents.texts.push(mt_kernel::Text {
            leader_style: mt_kernel::LeaderStyle::Hidden,
            arrow: mt_kernel::ArrowStyle::None,
            ..mt_kernel::Text::leader(
                inf.point,
                DVec3::ZERO,
                crate::annot::area_label(area, self.units.unwrap_or(LengthUnit::Inch)),
            )
        });
        true
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
            container: None,
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

    fn on_face(p: DVec3, n: DVec3) -> Inference {
        Inference {
            kind: Kind::OnFace,
            normal: Some(n),
            ..inf(p)
        }
    }

    /// Outward normal of a roof pitched 30 degrees whose fall line points `azimuth`
    /// degrees from red, with its level line and up-slope direction.
    fn roof(azimuth: f64) -> (DVec3, DVec3, DVec3) {
        let (p, a) = (30f64.to_radians(), azimuth.to_radians());
        let fall = v(a.cos(), a.sin(), 0.0);
        let n = fall * p.sin() + DVec3::Z * p.cos();
        let level = DVec3::Z.cross(fall);
        (n, level, n.cross(level))
    }

    /// The four edges of the last rectangle drawn, as corner pairs.
    fn rect_edges(e: &Entities) -> Vec<(DVec3, DVec3)> {
        e.edges
            .values()
            .map(|ed| (e.vertices[ed.start].pos, e.vertices[ed.end].pos))
            .collect()
    }

    #[test]
    fn plane_axes_follow_model_axes_and_level_lines() {
        let close = |a: DVec3, b: DVec3| (a - b).length() < 1e-9;
        assert_eq!(plane_axes(DVec3::Z), (DVec3::X, DVec3::Y));
        assert_eq!(plane_axes(-DVec3::Z), (DVec3::X, -DVec3::Y));
        let (u, w) = plane_axes(DVec3::X);
        assert!(close(u, DVec3::Y) && close(w, DVec3::Z));
        let (u, w) = plane_axes(-DVec3::Y);
        assert!(close(u, DVec3::X) && close(w, DVec3::Z));
        for az in [0.0, 30.0, 90.0, 137.0, 200.0, 315.0] {
            let (n, level, up) = roof(az);
            let (u, w) = plane_axes(n);
            assert!(close(u, level) && close(w, up), "azimuth {az}");
            assert!(u.z.abs() < 1e-12 && w.z > 0.0);
            assert!(close(u.cross(w), n));
        }
        // Facing down (an overhang's soffit): still level and right-handed.
        let (u, w) = plane_axes(-roof(40.0).0);
        assert!(u.z.abs() < 1e-12 && close(u.cross(w), -roof(40.0).0));
    }

    #[test]
    fn rectangle_on_a_roof_has_level_sides_and_sides_down_the_slope() {
        for az in [0.0, 35.0, 90.0, 250.0] {
            let (n, level, up) = roof(az);
            let a = v(3.0, -2.0, 5.0);
            let mut e = Entities::new();
            let mut t = Rectangle::default();
            t.click(&mut e, &on_face(a, n));
            assert!(t.click(&mut e, &on_face(a + level * 10.0 - up * 6.0, n)));
            assert_eq!(e.faces.len(), 1);
            let edges = rect_edges(&e);
            assert_eq!(edges.len(), 4);
            let flat = edges.iter().filter(|(p, q)| (p.z - q.z).abs() < 1e-9);
            let sloped = edges
                .iter()
                .filter(|(p, q)| (q - p).normalize().cross(up).length() < 1e-9);
            assert_eq!(flat.clone().count(), 2, "azimuth {az}");
            assert_eq!(sloped.clone().count(), 2, "azimuth {az}");
            assert!(
                flat.map(|(p, q)| p.distance(*q))
                    .all(|l| (l - 10.0).abs() < 1e-9)
            );
            assert!(
                sloped
                    .map(|(p, q)| p.distance(*q))
                    .all(|l| (l - 6.0).abs() < 1e-9)
            );
            // Every corner stays on the roof.
            assert!(e.vertices.values().all(|x| (x.pos - a).dot(n).abs() < 1e-9));
        }
    }

    #[test]
    fn rectangle_on_a_roof_projects_an_off_roof_cursor() {
        let (n, level, up) = roof(20.0);
        let a = v(0.0, 0.0, 10.0);
        let c = rectangle_corners(a, a + level * 4.0 + up * 3.0 + v(0.0, 0.0, -50.0), n);
        assert!(c.iter().all(|p| (p - a).dot(n).abs() < 1e-9));
        assert!((c[1].z - c[0].z).abs() < 1e-9 && (c[2].z - c[3].z).abs() < 1e-9);
    }

    #[test]
    fn rectangle_on_a_wall_runs_horizontal_and_vertical() {
        let mut e = Entities::new();
        let mut t = Rectangle::default();
        t.click(&mut e, &on_face(v(5.0, 0.0, 0.0), DVec3::X));
        assert!(t.click(&mut e, &on_face(v(5.0, -4.0, 3.0), DVec3::X)));
        for (p, q) in rect_edges(&e) {
            let d = (q - p).normalize();
            assert!(d.cross(DVec3::Y).length() < 1e-9 || d.cross(DVec3::Z).length() < 1e-9);
        }
        assert!(e.vertices.values().any(|x| x.pos == v(5.0, -4.0, 3.0)));
        // A skewed wall: one side level, the other plumb.
        let n = v(1.0, 1.0, 0.0).normalize();
        let c = rectangle_corners(v(0.0, 0.0, 0.0), v(-3.0, 3.0, 2.0), n);
        assert!((c[1].z - c[0].z).abs() < 1e-9);
        assert!((c[3] - c[0]).normalize().distance(DVec3::Z) < 1e-9);
    }

    #[test]
    fn typed_rectangle_on_a_roof_follows_the_cursor_quadrant() {
        let (n, level, up) = roof(60.0);
        let a = v(1.0, 2.0, 3.0);
        for (sl, su) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
            let mut e = Entities::new();
            let mut t = Rectangle::default();
            t.click(&mut e, &on_face(a, n));
            let cur = on_face(a + level * 2.0 * sl + up * 0.5 * su, n);
            assert!(t.typed(&mut e, Some(&cur), "8,5", LengthUnit::Inch));
            let far = a + level * 8.0 * sl + up * 5.0 * su;
            assert!(e.vertices.values().any(|x| x.pos.distance(far) < 1e-9));
            assert!(
                e.vertices
                    .values()
                    .any(|x| x.pos.distance(a + level * 8.0 * sl) < 1e-9)
            );
        }
        // On a wall facing red, `w` runs green and `h` blue.
        let mut e = Entities::new();
        let mut t = Rectangle::default();
        t.click(&mut e, &on_face(v(0.0, 0.0, 0.0), DVec3::X));
        let cur = on_face(v(0.0, -1.0, 1.0), DVec3::X);
        assert!(t.typed(&mut e, Some(&cur), "8,5", LengthUnit::Inch));
        assert!(e.vertices.values().any(|x| x.pos == v(0.0, -8.0, 5.0)));
        assert!(e.vertices.values().any(|x| x.pos == v(0.0, -8.0, 0.0)));
    }

    #[test]
    fn arrow_keys_lock_the_rectangle_plane_before_and_after_the_first_corner() {
        let (n, _, _) = roof(45.0);
        // Locked before: blue keeps the rectangle flat on a roof.
        let mut e = Entities::new();
        let mut t = Rectangle::default();
        t.lock_plane(Axis::Blue);
        t.click(&mut e, &on_face(v(0.0, 0.0, 0.0), n));
        assert!(t.click(&mut e, &on_face(v(4.0, 3.0, 0.0), n)));
        assert!(e.vertices.values().all(|x| x.pos.z == 0.0));
        // Locked after: red stands it on the green-blue plane; pressed again, unlocked.
        let mut e = Entities::new();
        let mut t = Rectangle::default();
        t.click(&mut e, &inf(v(0.0, 0.0, 0.0)));
        t.lock_plane(Axis::Red);
        let c = t.corners(v(0.0, 4.0, 3.0)).unwrap();
        assert_eq!(c[1], v(0.0, 4.0, 0.0));
        t.lock_plane(Axis::Red);
        assert!(t.click(&mut e, &inf(v(4.0, 3.0, 0.0))));
        assert!(e.vertices.values().all(|x| x.pos.z == 0.0));
    }

    #[test]
    fn circle_on_a_roof_lies_in_it_and_relocks() {
        let (n, level, _) = roof(110.0);
        let mut t = Polygon::circle();
        let mut e = Entities::new();
        t.click(&mut e, &on_face(v(0.0, 0.0, 0.0), n));
        let arc = t.arc(v(0.0, 0.0, 0.0), Some(5.0)).unwrap();
        assert!((arc.x_axis - level).length() < 1e-9);
        assert!(arc.points().iter().all(|p| p.dot(n).abs() < 1e-9));
        t.lock_plane(Axis::Blue);
        let arc = t.arc(v(3.0, 4.0, 9.0), None).unwrap();
        assert!(arc.points().iter().all(|p| p.z.abs() < 1e-9));
        assert_eq!(arc.radius, 5.0);
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
    fn protractor_places_angled_guide_through_center() {
        let mut e = Entities::new();
        let mut t = Protractor::default();
        t.click(&mut e, &inf(v(1.0, 1.0, 0.0)));
        t.click(&mut e, &inf(v(11.0, 1.0, 0.0)));
        assert!(t.click(&mut e, &inf(v(1.0, 5.0, 0.0))));
        assert_eq!(e.guides[0].point, v(1.0, 1.0, 0.0));
        assert!((e.guides[0].dir.unwrap() - DVec3::Y).length() < 1e-12);
        // Typed angles turn towards the cursor's side.
        t.click(&mut e, &inf(v(0.0, 0.0, 0.0)));
        t.click(&mut e, &inf(v(10.0, 0.0, 0.0)));
        assert!(t.typed(
            &mut e,
            Some(&inf(v(5.0, -1.0, 0.0))),
            "30",
            LengthUnit::Inch
        ));
        let d = e.guides[1].dir.unwrap();
        assert!((d - v(3f64.sqrt() / 2.0, -0.5, 0.0)).length() < 1e-12);
    }

    #[test]
    fn protractor_on_a_wall_and_slopes() {
        let mut e = Entities::new();
        let mut t = Protractor::default();
        let mut wall = inf(v(0.0, 0.0, 0.0));
        wall.normal = Some(DVec3::Y);
        t.click(&mut e, &wall);
        t.click(&mut e, &inf(v(10.0, 0.0, 0.0)));
        assert!(t.typed(
            &mut e,
            Some(&inf(v(5.0, 0.0, 5.0))),
            "1:1",
            LengthUnit::Inch
        ));
        let d = e.guides[0].dir.unwrap();
        assert!((d - v(1.0, 0.0, 1.0).normalize()).length() < 1e-12, "{d}");
        // atan may differ in the last bit between platforms.
        let near = |s: &str, want: f64| (parse_angle(s).unwrap() - want).abs() < 1e-12;
        assert!(near("90°", 90f64.to_radians()));
        assert!(near("1:2", 0.5f64.atan()));
        assert_eq!(parse_angle("abc"), None);
    }

    #[test]
    fn dimension_between_points_and_on_edges() {
        let mut e = Entities::new();
        let mut t = DimensionTool::default();
        t.click(&mut e, &inf(v(0.0, 0.0, 0.0)));
        t.click(&mut e, &inf(v(10.0, 0.0, 0.0)));
        assert!(t.click(&mut e, &inf(v(3.0, 4.0, 0.0))));
        assert_eq!(e.dimensions[0].offset, v(0.0, 4.0, 0.0));
        assert_eq!(e.dimensions[0].value(), 10.0);
        // One click on an edge measures it; typed text overrides the label.
        let mut on_edge = inf(v(5.0, 5.0, 0.0));
        on_edge.kind = mt_infer::Kind::OnEdge;
        on_edge.segment = Some([v(0.0, 5.0, 0.0), v(10.0, 5.0, 0.0)]);
        t.click(&mut e, &on_edge);
        assert!(t.typed(
            &mut e,
            Some(&inf(v(2.0, 8.0, 0.0))),
            "Wall",
            LengthUnit::Inch
        ));
        let d = &e.dimensions[1];
        assert_eq!(
            (d.a, d.b, d.offset),
            (v(0.0, 5.0, 0.0), v(10.0, 5.0, 0.0), v(0.0, 3.0, 0.0))
        );
        assert_eq!(d.text.as_deref(), Some("Wall"));
    }

    #[test]
    fn dimension_on_circle_is_diameter() {
        let mut e = Entities::new();
        let mut c = Polygon::circle();
        c.click(&mut e, &inf(v(0.0, 0.0, 0.0)));
        c.click(&mut e, &inf(v(10.0, 0.0, 0.0)));
        let (a, b) = {
            let edge = e.edges.values().next().unwrap();
            (e.pos(edge.start), e.pos(edge.end))
        };
        let mut on_edge = inf((a + b) / 2.0);
        on_edge.kind = mt_infer::Kind::OnEdge;
        on_edge.segment = Some([a, b]);
        let mut t = DimensionTool::default();
        t.click(&mut e, &on_edge);
        assert!(t.click(&mut e, &inf(v(20.0, 20.0, 0.0))));
        let d = &e.dimensions[0];
        assert_eq!(d.kind, mt_kernel::DimensionKind::Diameter);
        assert!((d.value() - 20.0).abs() < 1e-9);
        assert!((d.a + d.b).length() < 1e-9, "through the center");
    }

    #[test]
    fn text_default_labels() {
        let mut e = Entities::new();
        let mut r = Rectangle::default();
        r.click(&mut e, &inf(v(0.0, 0.0, 0.0)));
        r.click(&mut e, &inf(v(12.0, 24.0, 0.0)));
        let mut on_face = inf(v(5.0, 5.0, 0.0));
        on_face.kind = mt_infer::Kind::OnFace;
        assert_eq!(TextLabel::at(&e, &on_face), Some(TextLabel::Area(288.0)));
        assert_eq!(TextLabel::Area(288.0).format(LengthUnit::Foot), "2 ft²");
        let mut on_edge = inf(v(6.0, 0.0, 0.0));
        on_edge.kind = mt_infer::Kind::OnEdge;
        on_edge.segment = Some([v(0.0, 0.0, 0.0), v(12.0, 0.0, 0.0)]);
        assert_eq!(TextLabel::at(&e, &on_edge), Some(TextLabel::Length(12.0)));
        let mut corner = inf(v(12.0, 24.0, 0.0));
        corner.kind = mt_infer::Kind::Endpoint;
        assert_eq!(
            TextLabel::at(&e, &corner).unwrap().format(LengthUnit::Inch),
            "(12\", 24\", 0\")"
        );
        assert_eq!(TextLabel::at(&e, &inf(v(50.0, 50.0, 0.0))), None);
    }

    #[test]
    fn text_tool_leader_and_screen_text() {
        let mut e = Entities::new();
        let mut t = TextTool::default();
        let mut on_edge = inf(v(6.0, 0.0, 0.0));
        on_edge.kind = mt_infer::Kind::OnEdge;
        on_edge.segment = Some([v(0.0, 0.0, 0.0), v(12.0, 0.0, 0.0)]);
        assert!(!t.click(&mut e, &on_edge));
        assert!(!t.click(&mut e, &inf(v(10.0, 10.0, 5.0))));
        assert!(t.text_draft().is_some());
        // A click elsewhere finishes the text with its default label.
        assert!(t.click(&mut e, &inf(v(0.0, 0.0, 0.0))));
        assert_eq!(e.texts[0].text, "12\"");
        assert_eq!(e.texts[0].leader, v(4.0, 10.0, 5.0));
        // Empty space: a screen text, kept only once positioned and not empty.
        t.click(&mut e, &inf(v(50.0, 50.0, 0.0)));
        assert!(!t.finish_text(&mut e));
        t.click(&mut e, &inf(v(50.0, 50.0, 0.0)));
        let d = t.text_draft().unwrap();
        d.needs_screen_pos = false;
        d.text.point = v(0.1, 0.2, 0.0);
        assert!(t.typed(&mut e, None, "Title", LengthUnit::Inch));
        assert!(e.texts[1].screen);
        assert_eq!(e.texts[1].text, "Title");
    }

    #[test]
    fn segment_counts() {
        assert_eq!(parse_segments("48s"), Some(48));
        assert_eq!(parse_segments("2s"), None);
        assert_eq!(parse_segments("48"), None);
    }
}
