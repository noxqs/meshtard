//! SketchUp's inference engine: where the cursor snaps while drawing.
//!
//! Point inferences (in priority order): endpoint, midpoint, center (of the arc or circle
//! hovered last), intersection (of two edges, or of an edge and a face), guide point,
//! origin, on edge, on guide, on axis, on face. SketchUp's order is roughly endpoint >
//! midpoint / center / intersection > on edge > on face (unverified in detail). With a start point (`anchor`), direction inferences: along the red, green
//! and blue axes, parallel or perpendicular to the reference edge (the last edge
//! hovered), and "from point" tracking from the last endpoint or midpoint hovered.
//! A point inference beats a direction except "on face", which loses to a direction.
//! A locked direction (Shift or arrow keys) constrains the point to that line; point
//! inferences are then projected onto it.
//!
//! "From point" acquisition: with [`Engine::infer_at`] a point is acquired only after
//! the cursor has rested on it for [`ACQUIRE_DELAY`] seconds; [`Engine::infer`]
//! acquires at once.
//!
//! Shape hints for the drawing tools (Square, Golden Section, Half Circle, Tangent at
//! Vertex) are in [`hints`].
//!
//! Pick distances in pixels are drafts (`spec/parity/STATUS.md`).

pub mod hints;
pub mod units;

use glam::{DVec2, DVec3};
use mt_kernel::bvh::RayIndex;
use mt_render::Camera;

/// Pixels within which the cursor snaps to a point (endpoint, midpoint, origin).
pub const POINT_APERTURE: f64 = 10.0;
/// Pixels within which the cursor snaps onto an edge or axis line.
pub const EDGE_APERTURE: f64 = 6.0;
/// Pixels within which the cursor snaps onto a direction from the anchor.
pub const DIRECTION_APERTURE: f64 = 10.0;
/// Half-length of the model axes considered for "on axis" inferences, in inches.
const AXIS_EXTENT: f64 = 1e6;
/// Seconds the cursor rests on a point before it is acquired for "from point"
/// tracking (SketchUp: about a second; unverified).
pub const ACQUIRE_DELAY: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Red,
    Green,
    Blue,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::Red, Axis::Green, Axis::Blue];

    pub fn dir(self) -> DVec3 {
        match self {
            Axis::Red => DVec3::X,
            Axis::Green => DVec3::Y,
            Axis::Blue => DVec3::Z,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Axis::Red => "Red",
            Axis::Green => "Green",
            Axis::Blue => "Blue",
        }
    }

    pub fn color(self) -> [u8; 3] {
        match self {
            Axis::Red => [255, 0, 0],
            Axis::Green => [0, 160, 0],
            Axis::Blue => [0, 0, 255],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Endpoint,
    Midpoint,
    Origin,
    OnEdge,
    OnAxis(Axis),
    OnFace,
    /// On a construction line.
    OnGuide,
    /// A construction point.
    GuidePoint,
    /// Center of an arc or circle (shown after hovering the curve).
    Center,
    /// Where two edges (or guides) cross, or an edge pierces a face.
    Intersection,
    /// Along an axis direction from the anchor.
    Along(Axis),
    Parallel,
    Perpendicular,
    /// Lined up with the acquired point along an axis.
    FromPoint(Axis),
    /// No inference: a point on the working plane.
    Free,
    /// Shape hints set by the drawing tools ([`hints`]), never by [`Engine::infer`]:
    /// Rectangle with equal sides.
    Square,
    /// Rectangle with sides in the golden ratio.
    GoldenSection,
    /// Arc of 180 degrees.
    HalfCircle,
    /// Arc tangent to the edge ending at its start point.
    Tangent,
}

impl Kind {
    /// Tooltip text, as SketchUp shows it.
    pub fn label(self) -> String {
        match self {
            Kind::Endpoint => "Endpoint".into(),
            Kind::Midpoint => "Midpoint".into(),
            Kind::Origin => "Origin".into(),
            Kind::OnEdge => "On Edge".into(),
            Kind::OnAxis(a) => format!("On {} Axis", a.name()),
            Kind::OnFace => "On Face".into(),
            Kind::OnGuide => "On Guide".into(),
            Kind::GuidePoint => "Guide Point".into(),
            Kind::Center => "Center".into(),
            Kind::Intersection => "Intersection".into(),
            Kind::Along(a) => format!("On {} Axis", a.name()),
            Kind::Parallel => "Parallel to Edge".into(),
            Kind::Perpendicular => "Perpendicular to Edge".into(),
            Kind::FromPoint(_) => "From Point".into(),
            Kind::Free => String::new(),
            Kind::Square => "Square".into(),
            Kind::GoldenSection => "Golden Section".into(),
            Kind::HalfCircle => "Half Circle".into(),
            Kind::Tangent => "Tangent at Vertex".into(),
        }
    }

    /// Marker or guide color, as SketchUp shows it.
    pub fn color(self) -> [u8; 3] {
        match self {
            Kind::Endpoint => [0, 200, 0],
            Kind::Midpoint => [0, 200, 220],
            Kind::Origin => [230, 200, 0],
            Kind::OnEdge => [230, 0, 0],
            Kind::OnFace => [0, 0, 230],
            Kind::OnGuide | Kind::GuidePoint => [60, 60, 60],
            Kind::OnAxis(a) | Kind::Along(a) | Kind::FromPoint(a) => a.color(),
            Kind::Parallel | Kind::Perpendicular => [220, 0, 220],
            Kind::Free => [0, 0, 0],
            // Unverified: SketchUp's center and intersection marker colors.
            Kind::Center => [0, 170, 0],
            Kind::Intersection => [0, 0, 0],
            // Unverified: the dashed diagonal / arc color of the shape hints; the
            // tangent arc is cyan in SketchUp.
            Kind::Square | Kind::GoldenSection | Kind::HalfCircle => [220, 0, 220],
            Kind::Tangent => [0, 200, 220],
        }
    }

    /// A shape hint from [`hints`].
    pub fn is_hint(self) -> bool {
        matches!(
            self,
            Kind::Square | Kind::GoldenSection | Kind::HalfCircle | Kind::Tangent
        )
    }

    /// Points that can be acquired for "from point" tracking (center and intersection
    /// unverified).
    fn is_acquirable(self) -> bool {
        matches!(
            self,
            Kind::Endpoint | Kind::Midpoint | Kind::Origin | Kind::Center | Kind::Intersection
        )
    }

    fn is_point(self) -> bool {
        matches!(
            self,
            Kind::Endpoint
                | Kind::Midpoint
                | Kind::Origin
                | Kind::GuidePoint
                | Kind::Center
                | Kind::Intersection
                | Kind::OnEdge
                | Kind::OnGuide
                | Kind::OnAxis(_)
        )
    }
}

/// Where the cursor snapped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inference {
    pub point: DVec3,
    pub kind: Kind,
    /// Edge involved (hovered edge, or the reference edge for parallel/perpendicular).
    pub edge: Option<usize>,
    /// Dotted guide line to draw (e.g. from the acquired point for "from point").
    pub guide: Option<(DVec3, DVec3)>,
    /// The direction is locked (Shift or arrow key).
    pub locked: bool,
    /// Normal of the face under the cursor (for "on face"), to orient shapes drawn on it.
    pub normal: Option<DVec3>,
    /// The edge segment `edge` refers to, in world space.
    pub segment: Option<[DVec3; 2]>,
}

/// An arc or circle curve, for the "center" inference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcCurve {
    pub center: DVec3,
    /// Unit normal of the arc's plane.
    pub normal: DVec3,
    pub radius: f64,
}

/// Snappable geometry in world space.
#[derive(Debug, Clone, Default)]
pub struct Geometry {
    pub vertices: Vec<DVec3>,
    pub edges: Vec<[DVec3; 2]>,
    /// Construction lines as (point, unit direction), and construction points.
    pub guide_lines: Vec<(DVec3, DVec3)>,
    pub guide_points: Vec<DVec3>,
    /// Arc and circle curves.
    pub arcs: Vec<ArcCurve>,
    /// The arc (index into `arcs`) each edge is a segment of; may be shorter than
    /// `edges` (missing entries are `None`).
    pub edge_arc: Vec<Option<usize>>,
}

impl Geometry {
    /// Builds from edge segments; vertices are the distinct endpoints.
    pub fn from_edges(edges: Vec<[DVec3; 2]>) -> Self {
        let mut seen = std::collections::HashSet::new();
        let key = |p: DVec3| ((p * 1e4).round().as_i64vec3()).to_array();
        let vertices = edges
            .iter()
            .flatten()
            .copied()
            .filter(|p| seen.insert(key(*p)))
            .collect();
        Self {
            vertices,
            edges,
            ..Self::default()
        }
    }

    /// Adds an arc curve made of `edges` (indices into `edges`); returns its index.
    pub fn add_arc(&mut self, arc: ArcCurve, edges: impl IntoIterator<Item = usize>) -> usize {
        let i = self.arcs.len();
        self.arcs.push(arc);
        for e in edges {
            if self.edge_arc.len() <= e {
                self.edge_arc.resize(e + 1, None);
            }
            self.edge_arc[e] = Some(i);
        }
        i
    }

    /// The arc curve edge `edge` belongs to.
    pub fn arc_of(&self, edge: usize) -> Option<usize> {
        self.edge_arc.get(edge).copied().flatten()
    }
}

/// Camera and viewport size in pixels.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    pub camera: &'a Camera,
    pub viewport: DVec2,
}

impl View<'_> {
    fn project(&self, p: DVec3) -> Option<DVec2> {
        self.camera.project(p, self.viewport)
    }

    /// Projects the part of segment `a b` in front of the camera (long guide and axis
    /// lines reach behind it).
    fn project_segment(&self, a: DVec3, b: DVec3) -> Option<(DVec2, DVec2)> {
        let (mut a, mut b) = (a, b);
        if self.camera.perspective {
            let f = self.camera.forward();
            let near = self.camera.near() * 1.01;
            let (da, db) = (
                (a - self.camera.eye).dot(f) - near,
                (b - self.camera.eye).dot(f) - near,
            );
            if da < 0.0 && db < 0.0 {
                return None;
            }
            if da < 0.0 {
                a = a + (b - a) * (da / (da - db));
            } else if db < 0.0 {
                b = b + (a - b) * (db / (db - da));
            }
        }
        Some((self.project(a)?, self.project(b)?))
    }

    fn screen_dist(&self, p: DVec3, cursor: DVec2) -> f64 {
        self.project(p)
            .map_or(f64::INFINITY, |q| q.distance(cursor))
    }
}

/// The drawing axes (Axes tool): an origin and unit red, green and blue directions.
/// Axis inferences, axis locks, "on axis" and "from point" follow them, and points with
/// no inference land on their red-green plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axes {
    pub origin: DVec3,
    /// Red, green, blue.
    pub dirs: [DVec3; 3],
}

impl Default for Axes {
    fn default() -> Self {
        Self {
            origin: DVec3::ZERO,
            dirs: [DVec3::X, DVec3::Y, DVec3::Z],
        }
    }
}

impl Axes {
    pub fn dir(&self, axis: Axis) -> DVec3 {
        match axis {
            Axis::Red => self.dirs[0],
            Axis::Green => self.dirs[1],
            Axis::Blue => self.dirs[2],
        }
    }
}

/// The inference state carried between mouse moves.
#[derive(Debug, Clone, Default)]
pub struct Engine {
    /// The drawing axes (world axes by default).
    pub axes: Axes,
    /// Last edge hovered: the reference for parallel and perpendicular.
    pub reference_edge: Option<usize>,
    /// Last point acquired (endpoint, midpoint, ... rested on): the source for "from
    /// point".
    pub acquired: Option<DVec3>,
    /// Last arc or circle hovered (index into `Geometry::arcs`): its center is snappable.
    pub arc: Option<usize>,
    /// Locked direction and the inference kind that locked it.
    lock: Option<(DVec3, Kind)>,
    /// The acquirable point under the cursor and since when (seconds), for delayed
    /// acquisition.
    hover: Option<(DVec3, f64)>,
}

/// Point on line `o + t * d` closest to the ray.
fn closest_on_line(o: DVec3, d: DVec3, ray_o: DVec3, ray_d: DVec3) -> DVec3 {
    let w = o - ray_o;
    let (a, b, c) = (d.dot(d), d.dot(ray_d), ray_d.dot(ray_d));
    let (dd, e) = (d.dot(w), ray_d.dot(w));
    let denom = a * c - b * b;
    if denom.abs() < 1e-12 {
        return o;
    }
    let t = (b * e - c * dd) / denom;
    o + d * t
}

impl Engine {
    /// Forgets everything but the drawing axes.
    pub fn reset(&mut self) {
        *self = Self {
            axes: self.axes,
            ..Self::default()
        };
    }

    /// Forgets indices into the old geometry (reference edge, hovered arc) after the
    /// model changed.
    pub fn geometry_changed(&mut self) {
        self.reference_edge = None;
        self.arc = None;
    }

    /// Locks the direction to an axis (arrow keys: right = red, left = green, up = blue).
    /// Pressing the same axis again unlocks.
    pub fn toggle_axis_lock(&mut self, axis: Axis) {
        let kind = Kind::Along(axis);
        self.lock = match self.lock {
            Some((_, k)) if k == kind => None,
            _ => Some((self.axes.dir(axis), kind)),
        };
    }

    /// Locks to parallel of the reference edge (down arrow), if there is one.
    pub fn toggle_parallel_lock(&mut self, geom: &Geometry) {
        self.lock = match (
            self.lock,
            self.reference_edge.and_then(|e| geom.edges.get(e)),
        ) {
            (Some((_, Kind::Parallel)), _) | (_, None) => None,
            (_, Some([a, b])) => Some(((*b - *a).normalize(), Kind::Parallel)),
        };
    }

    /// Shift: locks the direction of the current inference from the anchor.
    pub fn lock_current(&mut self, inference: &Inference, anchor: DVec3) {
        let d = inference.point - anchor;
        if d.length() > 1e-9
            && matches!(
                inference.kind,
                Kind::Along(_) | Kind::Parallel | Kind::Perpendicular
            )
        {
            self.lock = Some((d.normalize(), inference.kind));
        }
    }

    /// Holds the cursor to a line through the anchor (Push/Pull along a face normal).
    pub fn lock_direction(&mut self, dir: DVec3, kind: Kind) {
        if dir.length() > 1e-9 {
            self.lock = Some((dir.normalize(), kind));
        }
    }

    pub fn unlock(&mut self) {
        self.lock = None;
    }

    pub fn is_locked(&self) -> bool {
        self.lock.is_some()
    }

    /// Infers the point under `cursor` (pixels), drawing from `anchor` if any. Points
    /// hovered are acquired for "from point" at once; see [`Engine::infer_at`].
    pub fn infer(
        &mut self,
        geom: &Geometry,
        bvh: &dyn RayIndex,
        view: View,
        cursor: DVec2,
        anchor: Option<DVec3>,
    ) -> Inference {
        let i = self.infer_inner(geom, bvh, view, cursor, anchor);
        if i.kind.is_acquirable() {
            self.acquired = Some(i.point);
        }
        i
    }

    /// Like [`Engine::infer`] at time `now` (seconds, any monotonic origin): a point is
    /// acquired for "from point" once the cursor has rested on it for
    /// [`ACQUIRE_DELAY`] (checked on each call, including the one moving away).
    pub fn infer_at(
        &mut self,
        geom: &Geometry,
        bvh: &dyn RayIndex,
        view: View,
        cursor: DVec2,
        anchor: Option<DVec3>,
        now: f64,
    ) -> Inference {
        let i = self.infer_inner(geom, bvh, view, cursor, anchor);
        let same = |(q, _): (DVec3, f64)| q.distance(i.point) <= 1e-9;
        match self.hover {
            Some(h) if same(h) && i.kind.is_acquirable() => {}
            _ => {
                // Moved off: the point rested on long enough is still acquired.
                self.tick(now);
                self.hover = i.kind.is_acquirable().then_some((i.point, now));
            }
        }
        self.tick(now);
        i
    }

    /// Acquires the hovered point if the cursor has rested on it for
    /// [`ACQUIRE_DELAY`] by `now`; true if it did. For apps that want the acquisition
    /// without a mouse move.
    pub fn tick(&mut self, now: f64) -> bool {
        match self.hover {
            Some((p, since)) if now - since >= ACQUIRE_DELAY && self.acquired != Some(p) => {
                self.acquired = Some(p);
                true
            }
            _ => false,
        }
    }

    /// When the point under the cursor will be acquired (seconds, same clock as
    /// `infer_at`), if it is not yet.
    pub fn acquire_due(&self) -> Option<f64> {
        self.hover
            .filter(|(p, _)| self.acquired != Some(*p))
            .map(|(_, since)| since + ACQUIRE_DELAY)
    }

    fn infer_inner(
        &mut self,
        geom: &Geometry,
        bvh: &dyn RayIndex,
        view: View,
        cursor: DVec2,
        anchor: Option<DVec3>,
    ) -> Inference {
        let mut i = self.infer_point(geom, bvh, view, cursor, anchor);
        i.segment = i.edge.and_then(|e| geom.edges.get(e).copied());
        i
    }

    fn infer_point(
        &mut self,
        geom: &Geometry,
        bvh: &dyn RayIndex,
        view: View,
        cursor: DVec2,
        anchor: Option<DVec3>,
    ) -> Inference {
        let ray = view.camera.ray(cursor, view.viewport);
        let hit = bvh.ray(ray.origin, ray.dir);
        let face_t = hit.map_or(f64::INFINITY, |h| h.t);
        let face_normal = hit.map(|h| {
            let [a, b, c] = bvh.triangle(h.triangle);
            (b - a).cross(c - a).normalize_or_zero()
        });
        let visible = |p: DVec3| (p - ray.origin).dot(ray.dir) <= face_t * (1.0 + 1e-4) + 1e-2;

        let point = self.point_inference(
            geom,
            bvh,
            view,
            cursor,
            ray.origin,
            ray.dir,
            face_t,
            face_normal,
            visible,
        );

        if let Some(a) = anchor {
            if let Some((dir, kind)) = self.lock {
                // Locked: stay on the line; a point inference is projected onto it.
                let p = match point.filter(|p| p.kind.is_point()) {
                    Some(p) => a + dir * (p.point - a).dot(dir),
                    None => closest_on_line(a, dir, ray.origin, ray.dir),
                };
                return Inference {
                    point: p,
                    kind,
                    edge: None,
                    guide: None,
                    locked: true,
                    normal: None,
                    segment: None,
                };
            }
            if let Some(p) = point.filter(|p| p.kind.is_point()) {
                self.remember(geom, &p);
                return p;
            }
            if let Some(d) = self.direction_inference(geom, view, cursor, ray.origin, ray.dir, a) {
                return d;
            }
        }
        if let Some(p) = point {
            self.remember(geom, &p);
            return p;
        }
        if let Some(t) = self.tracking(view, cursor, ray.origin, ray.dir, None) {
            return t;
        }
        Inference {
            point: free_point(ray.origin, ray.dir, anchor, &self.axes),
            kind: Kind::Free,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
        }
    }

    fn remember(&mut self, geom: &Geometry, p: &Inference) {
        if matches!(p.kind, Kind::OnEdge | Kind::Midpoint | Kind::Endpoint) && p.edge.is_some() {
            self.reference_edge = p.edge;
        }
        if let Some(a) = p.edge.and_then(|e| geom.arc_of(e)) {
            self.arc = Some(a);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn point_inference(
        &self,
        geom: &Geometry,
        bvh: &dyn RayIndex,
        view: View,
        cursor: DVec2,
        ro: DVec3,
        rd: DVec3,
        face_t: f64,
        face_normal: Option<DVec3>,
        visible: impl Fn(DVec3) -> bool,
    ) -> Option<Inference> {
        let near = |p: DVec3, r: f64| {
            let d = view.screen_dist(p, cursor);
            (d <= r && visible(p)).then_some(d)
        };
        let best_point = |points: &mut dyn Iterator<Item = (DVec3, Option<usize>)>, kind| {
            points
                .filter_map(|(p, e)| near(p, POINT_APERTURE).map(|d| (d, p, e)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p, e)| Inference {
                    point: p,
                    kind,
                    edge: e,
                    guide: None,
                    locked: false,
                    normal: None,
                    segment: None,
                })
        };

        // Endpoints: remember an edge using the vertex, for parallel inferences.
        let endpoint = best_point(
            &mut geom.vertices.iter().map(|&p| (p, None)),
            Kind::Endpoint,
        )
        .map(|mut i| {
            i.edge = geom
                .edges
                .iter()
                .position(|[a, b]| *a == i.point || *b == i.point);
            i
        });
        let midpoint = || {
            best_point(
                &mut geom
                    .edges
                    .iter()
                    .enumerate()
                    .map(|(i, [a, b])| ((*a + *b) / 2.0, Some(i))),
                Kind::Midpoint,
            )
        };
        let origin = || best_point(&mut std::iter::once((self.axes.origin, None)), Kind::Origin);
        let on_line = |a: DVec3, b: DVec3| -> Option<(f64, DVec3)> {
            let (pa, pb) = view.project_segment(a, b)?;
            let ab = pb - pa;
            let t = ((cursor - pa).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            let d = cursor.distance(pa + ab * t);
            if d > EDGE_APERTURE {
                return None;
            }
            let p = closest_on_line(a, b - a, ro, rd);
            let s = (p - a).dot(b - a) / (b - a).length_squared();
            ((0.0..=1.0).contains(&s) && visible(p)).then_some((d, p))
        };
        let on_edge = || {
            geom.edges
                .iter()
                .enumerate()
                .filter_map(|(i, [a, b])| on_line(*a, *b).map(|(d, p)| (d, p, i)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p, i)| Inference {
                    point: p,
                    kind: Kind::OnEdge,
                    edge: Some(i),
                    guide: None,
                    locked: false,
                    normal: None,
                    segment: None,
                })
        };
        let on_axis = || {
            Axis::ALL
                .iter()
                .filter_map(|&ax| {
                    let (o, d) = (self.axes.origin, self.axes.dir(ax));
                    on_line(o - d * AXIS_EXTENT, o + d * AXIS_EXTENT).map(|(d, p)| (d, p, ax))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p, ax)| Inference {
                    point: p,
                    kind: Kind::OnAxis(ax),
                    edge: None,
                    guide: None,
                    locked: false,
                    normal: None,
                    segment: None,
                })
        };
        let on_face = || {
            face_t.is_finite().then(|| Inference {
                point: ro + rd * face_t,
                kind: Kind::OnFace,
                edge: None,
                guide: None,
                locked: false,
                normal: face_normal,
                segment: None,
            })
        };
        let guide_point = || {
            best_point(
                &mut geom.guide_points.iter().map(|&p| (p, None)),
                Kind::GuidePoint,
            )
        };
        let on_guide = || {
            geom.guide_lines
                .iter()
                .filter_map(|&(p, d)| on_line(p - d * AXIS_EXTENT, p + d * AXIS_EXTENT))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p)| Inference {
                    point: p,
                    kind: Kind::OnGuide,
                    edge: None,
                    guide: None,
                    locked: false,
                    normal: None,
                    segment: None,
                })
        };
        // Center of the arc or circle hovered last.
        let center = || {
            best_point(
                &mut self
                    .arc
                    .and_then(|a| geom.arcs.get(a))
                    .map(|a| (a.center, None))
                    .into_iter(),
                Kind::Center,
            )
        };
        let intersection = || {
            let hit = face_normal
                .filter(|_| face_t.is_finite())
                .map(|n| (ro + rd * face_t, n));
            intersection(geom, bvh, view, cursor, hit, &visible).map(|p| Inference {
                point: p,
                kind: Kind::Intersection,
                edge: None,
                guide: None,
                locked: false,
                normal: None,
                segment: None,
            })
        };
        endpoint
            .or_else(midpoint)
            .or_else(center)
            .or_else(intersection)
            .or_else(guide_point)
            .or_else(origin)
            .or_else(on_edge)
            .or_else(on_guide)
            .or_else(on_axis)
            .or_else(on_face)
    }

    /// Directions from the anchor: axes, parallel and perpendicular to the reference
    /// edge, and lining up with the acquired point.
    fn direction_inference(
        &self,
        geom: &Geometry,
        view: View,
        cursor: DVec2,
        ro: DVec3,
        rd: DVec3,
        a: DVec3,
    ) -> Option<Inference> {
        let mut dirs: Vec<(DVec3, Kind)> = Axis::ALL
            .iter()
            .map(|&ax| (self.axes.dir(ax), Kind::Along(ax)))
            .collect();
        if let Some([p, q]) = self.reference_edge.and_then(|e| geom.edges.get(e)) {
            let e = (*q - *p).normalize();
            let axis_aligned = Axis::ALL
                .iter()
                .any(|&ax| e.cross(self.axes.dir(ax)).length() < 1e-6);
            if !axis_aligned {
                dirs.push((e, Kind::Parallel));
                // Perpendicular within the plane of the edge and the anchor, else horizontal.
                let n = (*p - a).cross(e);
                let perp = if n.length() > 1e-9 {
                    e.cross(n)
                } else {
                    e.cross(DVec3::Z)
                };
                if perp.length() > 1e-9 {
                    dirs.push((perp.normalize(), Kind::Perpendicular));
                }
            }
        }
        let best = dirs
            .into_iter()
            .map(|(d, kind)| {
                let p = closest_on_line(a, d, ro, rd);
                (view.screen_dist(p, cursor), p, kind)
            })
            .filter(|(dist, p, _)| *dist <= DIRECTION_APERTURE && p.distance(a) > 1e-6)
            .min_by(|x, y| x.0.total_cmp(&y.0));
        let (_, p, kind) = best?;
        let edge = matches!(kind, Kind::Parallel | Kind::Perpendicular)
            .then_some(self.reference_edge)
            .flatten();
        let mut inf = Inference {
            point: p,
            kind,
            edge,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
        };
        // Line up with the acquired point along another axis, if close.
        if let (Kind::Along(_), Some(t)) = (kind, self.tracking(view, cursor, ro, rd, Some((a, p))))
        {
            inf.point = t.point;
            inf.guide = t.guide;
        }
        Some(inf)
    }

    /// "From point": a line from the acquired point along an axis. With `along` (the anchor
    /// and the current point on an anchor direction) the result is where that direction
    /// meets the tracking line.
    fn tracking(
        &self,
        view: View,
        cursor: DVec2,
        ro: DVec3,
        rd: DVec3,
        along: Option<(DVec3, DVec3)>,
    ) -> Option<Inference> {
        let src = self.acquired?;
        Axis::ALL
            .iter()
            .filter_map(|&ax| {
                let dir = self.axes.dir(ax);
                let p = match along {
                    Some((a, q)) => {
                        let d = (q - a).normalize_or_zero();
                        if d.cross(dir).length() < 1e-6 {
                            return None;
                        }
                        // Point on the anchor line closest to the tracking line.
                        let x = closest_on_line(a, d, src, dir);
                        let y = src + dir * (x - src).dot(dir);
                        (x.distance(y) < 1e-6).then_some(x)?
                    }
                    None => closest_on_line(src, dir, ro, rd),
                };
                let dist = view.screen_dist(p, cursor);
                (dist <= DIRECTION_APERTURE && p.distance(src) > 1e-6).then_some((dist, p, ax))
            })
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .map(|(_, p, ax)| Inference {
                point: p,
                kind: Kind::FromPoint(ax),
                edge: None,
                guide: Some((src, p)),
                locked: false,
                normal: None,
                segment: None,
            })
    }
}

/// Closest points of segments `p1 q1` and `p2 q2` as parameters `(s, t)` along each,
/// if the segments are not parallel and the closest points lie within both.
fn segment_crossing(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> Option<(f64, f64)> {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, b, e) = (d1.dot(d1), d1.dot(d2), d2.dot(d2));
    let (c, f) = (d1.dot(r), d2.dot(r));
    let denom = a * e - b * b;
    if denom <= 1e-12 * a * e {
        return None;
    }
    let s = (b * f - c * e) / denom;
    let t = (a * f - b * c) / denom;
    let eps = 1e-9;
    ((-eps..=1.0 + eps).contains(&s) && (-eps..=1.0 + eps).contains(&t)).then_some((s, t))
}

/// "Intersection": where two edges or guide lines that pass near the cursor meet in
/// 3D (within the kernel tolerance; a vertex shared by both is an endpoint instead), or
/// where an edge near the cursor pierces the face under it. The nearest such point
/// within [`POINT_APERTURE`] pixels. SketchUp also intersects faces with faces and
/// axes; that is left out (unverified which it shows).
fn intersection(
    geom: &Geometry,
    bvh: &dyn RayIndex,
    view: View,
    cursor: DVec2,
    hit: Option<(DVec3, DVec3)>,
    visible: &impl Fn(DVec3) -> bool,
) -> Option<DVec3> {
    let near_line = |a: DVec3, b: DVec3| {
        let Some((pa, pb)) = view.project_segment(a, b) else {
            return false;
        };
        let ab = pb - pa;
        let t = ((cursor - pa).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        cursor.distance(pa + ab * t) <= POINT_APERTURE
    };
    // Lines near the cursor; `true` for edges (guides do not pierce faces here).
    let lines: Vec<(DVec3, DVec3, bool)> = geom
        .edges
        .iter()
        .map(|&[a, b]| (a, b, true))
        .chain(
            geom.guide_lines
                .iter()
                .map(|&(p, d)| (p - d * AXIS_EXTENT, p + d * AXIS_EXTENT, false)),
        )
        .filter(|&(a, b, _)| near_line(a, b))
        .collect();
    let mut candidates = Vec::new();
    for (i, &(p1, q1, _)) in lines.iter().enumerate() {
        for &(p2, q2, _) in &lines[i + 1..] {
            let Some((s, t)) = segment_crossing(p1, q1, p2, q2) else {
                continue;
            };
            let at_end = |u: f64| !(1e-6..=1.0 - 1e-6).contains(&u);
            if at_end(s) && at_end(t) {
                continue;
            }
            let (x1, x2) = (p1 + (q1 - p1) * s, p2 + (q2 - p2) * t);
            if x1.distance(x2) <= mt_kernel::TOL {
                candidates.push((x1 + x2) / 2.0);
            }
        }
    }
    if let Some((h, n)) = hit {
        for &(a, b, _) in lines.iter().filter(|l| l.2) {
            let d = b - a;
            let denom = d.dot(n);
            if denom.abs() <= 1e-9 * d.length() {
                continue;
            }
            let s = (h - a).dot(n) / denom;
            if !(1e-6..=1.0 - 1e-6).contains(&s) {
                continue;
            }
            let x = a + d * s;
            // On a face: the view ray through `x` hits a triangle at `x`.
            let on_face = view.project(x).is_some_and(|q| {
                let ray = view.camera.ray(q, view.viewport);
                bvh.ray(ray.origin, ray.dir).is_some_and(|hit| {
                    (ray.origin + ray.dir * hit.t).distance(x)
                        <= mt_kernel::TOL + 1e-5 * x.distance(ray.origin)
                })
            });
            if on_face {
                candidates.push(x);
            }
        }
    }
    candidates
        .into_iter()
        .filter(|&x| visible(x))
        .map(|x| (view.screen_dist(x, cursor), x))
        .filter(|(d, _)| *d <= POINT_APERTURE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, x)| x)
}

/// A point with no inference: on the ground plane (the drawing axes' red-green plane),
/// or on the plane parallel to it through the anchor; a plane facing the camera that
/// contains the blue axis when looking almost level.
fn free_point(ro: DVec3, rd: DVec3, anchor: Option<DVec3>, axes: &Axes) -> DVec3 {
    let base = anchor.unwrap_or(axes.origin);
    let up = axes.dirs[2];
    let hit_plane = |n: DVec3| {
        let denom = rd.dot(n);
        (denom.abs() > 1e-9)
            .then(|| (base - ro).dot(n) / denom)
            .filter(|t| *t > 0.0)
            .map(|t| ro + rd * t)
    };
    let rz = rd.dot(up);
    if rz.abs() > 0.1
        && let Some(p) = hit_plane(up)
    {
        return p;
    }
    let facing = (rd - up * rz).normalize_or_zero();
    if facing != DVec3::ZERO
        && let Some(p) = hit_plane(facing)
    {
        return p;
    }
    ro + rd * (base - ro).dot(rd).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::bvh::Bvh;

    const VIEWPORT: DVec2 = DVec2::new(800.0, 600.0);

    fn camera() -> Camera {
        Camera {
            eye: DVec3::new(-300.0, -500.0, 300.0),
            target: DVec3::new(50.0, 50.0, 0.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        }
    }

    /// A 100 x 100 square on the ground with its face triangles.
    fn square() -> (Geometry, Bvh) {
        let c = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(100.0, 0.0, 0.0),
            DVec3::new(100.0, 100.0, 0.0),
            DVec3::new(0.0, 100.0, 0.0),
        ];
        let edges = (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect();
        let bvh = Bvh::build(vec![[c[0], c[1], c[2]], [c[0], c[2], c[3]]]);
        (Geometry::from_edges(edges), bvh)
    }

    fn at(p: DVec3) -> DVec2 {
        camera().project(p, VIEWPORT).unwrap()
    }

    fn infer(
        e: &mut Engine,
        g: &Geometry,
        b: &Bvh,
        cursor: DVec2,
        anchor: Option<DVec3>,
    ) -> Inference {
        let cam = camera();
        e.infer(
            g,
            b,
            View {
                camera: &cam,
                viewport: VIEWPORT,
            },
            cursor,
            anchor,
        )
    }

    #[test]
    fn snaps_to_endpoint_midpoint_edge_face() {
        let (g, b) = square();
        let mut e = Engine::default();
        let off = DVec2::new(3.0, -2.0);
        let i = infer(&mut e, &g, &b, at(DVec3::new(100.0, 0.0, 0.0)) + off, None);
        assert_eq!(
            (i.kind, i.point),
            (Kind::Endpoint, DVec3::new(100.0, 0.0, 0.0))
        );
        let i = infer(&mut e, &g, &b, at(DVec3::new(50.0, 0.0, 0.0)) + off, None);
        assert_eq!(
            (i.kind, i.point),
            (Kind::Midpoint, DVec3::new(50.0, 0.0, 0.0))
        );
        let i = infer(
            &mut e,
            &g,
            &b,
            at(DVec3::new(100.0, 30.0, 0.0)) + DVec2::new(2.0, 0.0),
            None,
        );
        assert_eq!(i.kind, Kind::OnEdge);
        assert!((i.point.x - 100.0).abs() < 1e-6 && i.point.z.abs() < 1e-6);
        let i = infer(&mut e, &g, &b, at(DVec3::new(60.0, 70.0, 0.0)), None);
        assert_eq!(i.kind, Kind::OnFace);
        assert!(i.point.z.abs() < 1e-9);
        assert!(
            i.normal.is_some_and(|n| n.z.abs() > 0.999),
            "face normal reported"
        );
    }

    /// Moved and rotated drawing axes: directions, the axis lock and the origin follow
    /// them.
    #[test]
    fn inferences_follow_moved_axes() {
        let (g, b) = square();
        let mut e = Engine::default();
        let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        e.axes = Axes {
            origin: DVec3::new(300.0, 0.0, 0.0),
            dirs: [DVec3::new(c, s, 0.0), DVec3::new(-s, c, 0.0), DVec3::Z],
        };
        let a = DVec3::new(200.0, 200.0, 0.0);
        let red = e.axes.dirs[0];
        let i = infer(
            &mut e,
            &g,
            &b,
            at(a + red * 60.0) + DVec2::new(0.0, 3.0),
            Some(a),
        );
        assert_eq!(i.kind, Kind::Along(Axis::Red));
        assert!((i.point - a).normalize().dot(red) > 1.0 - 1e-9);
        let o = e.axes.origin;
        let i = infer(&mut e, &g, &b, at(o) + DVec2::new(2.0, 1.0), None);
        assert_eq!((i.kind, i.point), (Kind::Origin, e.axes.origin));
        e.reset();
        assert_eq!(
            e.axes.origin,
            DVec3::new(300.0, 0.0, 0.0),
            "reset keeps the axes"
        );
        e.toggle_axis_lock(Axis::Green);
        assert_eq!(e.lock.map(|l| l.0), Some(e.axes.dirs[1]));
    }

    #[test]
    fn direction_from_anchor_along_axes() {
        let (g, b) = square();
        let mut e = Engine::default();
        let a = DVec3::new(200.0, 200.0, 0.0);
        let i = infer(
            &mut e,
            &g,
            &b,
            at(DVec3::new(200.0, 200.0, 80.0)) + DVec2::new(4.0, 0.0),
            Some(a),
        );
        assert_eq!(i.kind, Kind::Along(Axis::Blue));
        assert!((i.point.x - 200.0).abs() < 1e-6 && (i.point.y - 200.0).abs() < 1e-6);
        let i = infer(
            &mut e,
            &g,
            &b,
            at(DVec3::new(320.0, 200.0, 0.0)) + DVec2::new(0.0, 3.0),
            Some(a),
        );
        assert_eq!(i.kind, Kind::Along(Axis::Red));
    }

    #[test]
    fn lock_projects_points_onto_the_line() {
        let (g, b) = square();
        let mut e = Engine::default();
        let a = DVec3::new(0.0, -100.0, 0.0);
        e.toggle_axis_lock(Axis::Green);
        // Hovering the far corner while locked to green: the point is its projection.
        let i = infer(&mut e, &g, &b, at(DVec3::new(100.0, 100.0, 0.0)), Some(a));
        assert!(i.locked);
        assert!((i.point - DVec3::new(0.0, 100.0, 0.0)).length() < 1e-9);
        e.toggle_axis_lock(Axis::Green);
        assert!(!e.is_locked());
    }

    #[test]
    fn parallel_to_reference_edge() {
        let c = [DVec3::new(0.0, 0.0, 0.0), DVec3::new(100.0, 60.0, 0.0)];
        let g = Geometry::from_edges(vec![c]);
        let b = Bvh::default();
        let mut e = Engine::default();
        // Hover the edge to make it the reference.
        let i = infer(&mut e, &g, &b, at(DVec3::new(30.0, 18.0, 0.0)), None);
        assert_eq!(i.kind, Kind::OnEdge);
        let a = DVec3::new(0.0, 150.0, 0.0);
        let target = a + (c[1] - c[0]) * 0.8;
        let i = infer(&mut e, &g, &b, at(target), Some(a));
        assert_eq!(i.kind, Kind::Parallel);
        assert!((i.point - target).length() < 1e-6);
    }

    #[test]
    fn from_point_lines_up_with_acquired_point() {
        let (g, b) = square();
        let mut e = Engine::default();
        // Acquire the corner (100, 100, 0).
        infer(&mut e, &g, &b, at(DVec3::new(100.0, 100.0, 0.0)), None);
        assert_eq!(e.acquired, Some(DVec3::new(100.0, 100.0, 0.0)));
        // Drawing along green from (300, 0, 0): the point level with the corner.
        let a = DVec3::new(300.0, 0.0, 0.0);
        let i = infer(
            &mut e,
            &g,
            &b,
            at(DVec3::new(300.0, 100.0, 0.0)) + DVec2::new(3.0, 0.0),
            Some(a),
        );
        assert_eq!(i.kind, Kind::Along(Axis::Green));
        assert!(
            (i.point - DVec3::new(300.0, 100.0, 0.0)).length() < 1e-6,
            "{:?}",
            i.point
        );
        assert!(i.guide.is_some());
    }

    #[test]
    fn center_appears_after_hovering_the_arc() {
        let c = DVec3::new(60.0, 40.0, 0.0);
        let r = 30.0;
        let pts: Vec<DVec3> = (0..=24)
            .map(|i| {
                let t = std::f64::consts::TAU * f64::from(i) / 24.0;
                c + DVec3::new(t.cos(), t.sin(), 0.0) * r
            })
            .collect();
        let mut g = Geometry::from_edges(pts.windows(2).map(|w| [w[0], w[1]]).collect());
        let arc = g.add_arc(
            ArcCurve {
                center: c,
                normal: DVec3::Z,
                radius: r,
            },
            0..24,
        );
        assert_eq!(g.arc_of(5), Some(arc));
        let b = Bvh::default();
        let mut e = Engine::default();
        // Not yet: the center is not inferred before the curve is hovered.
        let i = infer(&mut e, &g, &b, at(c) + DVec2::new(2.0, 1.0), None);
        assert_ne!(i.kind, Kind::Center);
        // Hover the circle (on a segment, away from its vertices and midpoint).
        let on = pts[3].lerp(pts[4], 0.3);
        infer(&mut e, &g, &b, at(on), None);
        assert_eq!(e.arc, Some(arc));
        let i = infer(&mut e, &g, &b, at(c) + DVec2::new(2.0, 1.0), None);
        assert_eq!((i.kind, i.point), (Kind::Center, c));
        assert_eq!(e.acquired, Some(c), "the center is acquired for from point");
        e.geometry_changed();
        assert_eq!(e.arc, None);
    }

    #[test]
    fn intersection_of_crossing_edges() {
        let x = DVec3::new(70.0, 30.0, 0.0);
        let g = Geometry::from_edges(vec![
            [DVec3::new(0.0, 30.0, 0.0), DVec3::new(100.0, 30.0, 0.0)],
            [DVec3::new(70.0, 0.0, 0.0), DVec3::new(70.0, 100.0, 0.0)],
        ]);
        let b = Bvh::default();
        let mut e = Engine::default();
        let i = infer(&mut e, &g, &b, at(x) + DVec2::new(3.0, -2.0), None);
        assert_eq!(i.kind, Kind::Intersection);
        assert!((i.point - x).length() < 1e-9, "{:?}", i.point);
        // Endpoints still win over intersections.
        let i = infer(&mut e, &g, &b, at(DVec3::new(100.0, 30.0, 0.0)), None);
        assert_eq!(i.kind, Kind::Endpoint);
        // Edges that only cross on screen (one lifted) do not intersect.
        let lifted = Geometry::from_edges(vec![
            [DVec3::new(0.0, 30.0, 0.0), DVec3::new(100.0, 30.0, 0.0)],
            [DVec3::new(70.0, 0.0, 5.0), DVec3::new(70.0, 100.0, 5.0)],
        ]);
        let i = infer(&mut e, &lifted, &b, at(x), None);
        assert_ne!(i.kind, Kind::Intersection);
        // A guide line crossing an edge.
        let mut g = Geometry::from_edges(vec![[
            DVec3::new(0.0, 30.0, 0.0),
            DVec3::new(100.0, 30.0, 0.0),
        ]]);
        g.guide_lines.push((DVec3::new(70.0, 0.0, 0.0), DVec3::Y));
        let i = infer(&mut e, &g, &b, at(x) + DVec2::new(1.0, 2.0), None);
        assert_eq!(i.kind, Kind::Intersection);
        assert!((i.point - x).length() < 1e-9);
    }

    #[test]
    fn intersection_of_edge_and_face() {
        let (mut g, b) = square();
        let pierce = DVec3::new(30.0, 60.0, 0.0);
        g = Geometry::from_edges(
            g.edges
                .iter()
                .copied()
                .chain([[DVec3::new(30.0, 60.0, -40.0), DVec3::new(30.0, 60.0, 100.0)]])
                .collect(),
        );
        let mut e = Engine::default();
        let i = infer(&mut e, &g, &b, at(pierce) + DVec2::new(2.0, -3.0), None);
        assert_eq!(i.kind, Kind::Intersection);
        assert!((i.point - pierce).length() < 1e-6, "{:?}", i.point);
        // Further up the edge it is just on the edge.
        let i = infer(
            &mut e,
            &g,
            &b,
            at(DVec3::new(30.0, 60.0, 70.0)) + DVec2::new(2.0, 0.0),
            None,
        );
        assert_eq!(i.kind, Kind::OnEdge);
    }

    #[test]
    fn acquisition_waits_for_the_hover_delay() {
        let (g, b) = square();
        let cam = camera();
        let view = View {
            camera: &cam,
            viewport: VIEWPORT,
        };
        let corner = DVec3::new(100.0, 100.0, 0.0);
        let face = at(DVec3::new(60.0, 70.0, 0.0));
        let mut e = Engine::default();
        // Resting on the corner: acquired only after the delay.
        e.infer_at(&g, &b, view, at(corner), None, 10.0);
        assert_eq!(e.acquired, None);
        assert_eq!(e.acquire_due(), Some(10.0 + ACQUIRE_DELAY));
        e.infer_at(&g, &b, view, at(corner) + DVec2::new(1.0, 0.0), None, 10.5);
        assert_eq!(e.acquired, None);
        assert!(e.tick(10.0 + ACQUIRE_DELAY));
        assert_eq!(e.acquired, Some(corner));
        // Passing over another point briefly does not acquire it.
        let mid = DVec3::new(50.0, 0.0, 0.0);
        e.infer_at(&g, &b, view, at(mid), None, 20.0);
        e.infer_at(&g, &b, view, face, None, 20.3);
        assert_eq!(e.acquired, Some(corner));
        // Resting, then moving away: acquired on the move.
        e.infer_at(&g, &b, view, at(mid), None, 30.0);
        e.infer_at(&g, &b, view, face, None, 31.5);
        assert_eq!(e.acquired, Some(mid));
        assert_eq!(e.acquire_due(), None);
    }

    #[test]
    fn free_point_lands_on_ground() {
        let g = Geometry::default();
        let b = Bvh::default();
        let mut e = Engine::default();
        // Away from the axes (the green axis runs near the lower right corner).
        let i = infer(&mut e, &g, &b, at(DVec3::new(160.0, -60.0, 0.0)), None);
        assert_eq!(i.kind, Kind::Free);
        assert!(i.point.z.abs() < 1e-6); // The green axis, which also reaches behind the camera, is still inferred.
        let i = infer(&mut e, &g, &b, at(DVec3::new(0.0, 200.0, 0.0)), None);
        assert_eq!(i.kind, Kind::OnAxis(Axis::Green));
    }
}
