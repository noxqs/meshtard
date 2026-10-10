//! SketchUp's inference engine: where the cursor snaps while drawing.
//!
//! Point inferences (in priority order): endpoint, midpoint, origin, on edge, on axis,
//! on face. With a start point (`anchor`), direction inferences: along the red, green
//! and blue axes, parallel or perpendicular to the reference edge (the last edge
//! hovered), and "from point" tracking from the last endpoint or midpoint hovered.
//! A point inference beats a direction except "on face", which loses to a direction.
//! A locked direction (Shift or arrow keys) constrains the point to that line; point
//! inferences are then projected onto it.
//!
//! Pick distances in pixels are drafts (`spec/parity/STATUS.md`).

pub mod units;

use glam::{DVec2, DVec3};
use mt_kernel::bvh::Bvh;
use mt_render::Camera;

/// Pixels within which the cursor snaps to a point (endpoint, midpoint, origin).
pub const POINT_APERTURE: f64 = 10.0;
/// Pixels within which the cursor snaps onto an edge or axis line.
pub const EDGE_APERTURE: f64 = 6.0;
/// Pixels within which the cursor snaps onto a direction from the anchor.
pub const DIRECTION_APERTURE: f64 = 10.0;
/// Half-length of the model axes considered for "on axis" inferences, in inches.
const AXIS_EXTENT: f64 = 1e6;

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
    /// Along an axis direction from the anchor.
    Along(Axis),
    Parallel,
    Perpendicular,
    /// Lined up with the acquired point along an axis.
    FromPoint(Axis),
    /// No inference: a point on the working plane.
    Free,
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
            Kind::Along(a) => format!("On {} Axis", a.name()),
            Kind::Parallel => "Parallel to Edge".into(),
            Kind::Perpendicular => "Perpendicular to Edge".into(),
            Kind::FromPoint(_) => "From Point".into(),
            Kind::Free => String::new(),
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
            Kind::OnAxis(a) | Kind::Along(a) | Kind::FromPoint(a) => a.color(),
            Kind::Parallel | Kind::Perpendicular => [220, 0, 220],
            Kind::Free => [0, 0, 0],
        }
    }

    fn is_point(self) -> bool {
        matches!(
            self,
            Kind::Endpoint | Kind::Midpoint | Kind::Origin | Kind::OnEdge | Kind::OnAxis(_)
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
}

/// Snappable geometry in world space.
#[derive(Debug, Clone, Default)]
pub struct Geometry {
    pub vertices: Vec<DVec3>,
    pub edges: Vec<[DVec3; 2]>,
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
        Self { vertices, edges }
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

    fn screen_dist(&self, p: DVec3, cursor: DVec2) -> f64 {
        self.project(p)
            .map_or(f64::INFINITY, |q| q.distance(cursor))
    }
}

/// The inference state carried between mouse moves.
#[derive(Debug, Clone, Default)]
pub struct Engine {
    /// Last edge hovered: the reference for parallel and perpendicular.
    pub reference_edge: Option<usize>,
    /// Last endpoint or midpoint hovered: the source for "from point".
    pub acquired: Option<DVec3>,
    /// Locked direction and the inference kind that locked it.
    lock: Option<(DVec3, Kind)>,
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
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Locks the direction to an axis (arrow keys: right = red, left = green, up = blue).
    /// Pressing the same axis again unlocks.
    pub fn toggle_axis_lock(&mut self, axis: Axis) {
        let kind = Kind::Along(axis);
        self.lock = match self.lock {
            Some((_, k)) if k == kind => None,
            _ => Some((axis.dir(), kind)),
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

    pub fn unlock(&mut self) {
        self.lock = None;
    }

    pub fn is_locked(&self) -> bool {
        self.lock.is_some()
    }

    /// Infers the point under `cursor` (pixels), drawing from `anchor` if any.
    pub fn infer(
        &mut self,
        geom: &Geometry,
        bvh: &Bvh,
        view: View,
        cursor: DVec2,
        anchor: Option<DVec3>,
    ) -> Inference {
        let ray = view.camera.ray(cursor, view.viewport);
        let face_t = bvh.ray(ray.origin, ray.dir).map_or(f64::INFINITY, |h| h.t);
        let visible = |p: DVec3| (p - ray.origin).dot(ray.dir) <= face_t * (1.0 + 1e-4) + 1e-2;

        let point = self.point_inference(geom, view, cursor, ray.origin, ray.dir, face_t, visible);

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
                };
            }
            if let Some(p) = point.filter(|p| p.kind.is_point()) {
                self.remember(&p);
                return p;
            }
            if let Some(d) = self.direction_inference(geom, view, cursor, ray.origin, ray.dir, a) {
                return d;
            }
        }
        if let Some(p) = point {
            self.remember(&p);
            return p;
        }
        if let Some(t) = self.tracking(view, cursor, ray.origin, ray.dir, None) {
            return t;
        }
        Inference {
            point: free_point(ray.origin, ray.dir, anchor),
            kind: Kind::Free,
            edge: None,
            guide: None,
            locked: false,
        }
    }

    fn remember(&mut self, p: &Inference) {
        if matches!(p.kind, Kind::OnEdge | Kind::Midpoint | Kind::Endpoint) && p.edge.is_some() {
            self.reference_edge = p.edge;
        }
        if matches!(p.kind, Kind::Endpoint | Kind::Midpoint | Kind::Origin) {
            self.acquired = Some(p.point);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn point_inference(
        &self,
        geom: &Geometry,
        view: View,
        cursor: DVec2,
        ro: DVec3,
        rd: DVec3,
        face_t: f64,
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
        let origin = || best_point(&mut std::iter::once((DVec3::ZERO, None)), Kind::Origin);
        let on_line = |a: DVec3, b: DVec3| -> Option<(f64, DVec3)> {
            let (pa, pb) = (view.project(a)?, view.project(b)?);
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
                })
        };
        let on_axis = || {
            Axis::ALL
                .iter()
                .filter_map(|&ax| {
                    on_line(-ax.dir() * AXIS_EXTENT, ax.dir() * AXIS_EXTENT)
                        .map(|(d, p)| (d, p, ax))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p, ax)| Inference {
                    point: p,
                    kind: Kind::OnAxis(ax),
                    edge: None,
                    guide: None,
                    locked: false,
                })
        };
        let on_face = || {
            face_t.is_finite().then(|| Inference {
                point: ro + rd * face_t,
                kind: Kind::OnFace,
                edge: None,
                guide: None,
                locked: false,
            })
        };
        endpoint
            .or_else(midpoint)
            .or_else(origin)
            .or_else(on_edge)
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
            .map(|&ax| (ax.dir(), Kind::Along(ax)))
            .collect();
        if let Some([p, q]) = self.reference_edge.and_then(|e| geom.edges.get(e)) {
            let e = (*q - *p).normalize();
            let axis_aligned = Axis::ALL.iter().any(|ax| e.cross(ax.dir()).length() < 1e-6);
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
                let p = match along {
                    Some((a, q)) => {
                        let d = (q - a).normalize_or_zero();
                        if d.cross(ax.dir()).length() < 1e-6 {
                            return None;
                        }
                        // Point on the anchor line closest to the tracking line.
                        let x = closest_on_line(a, d, src, ax.dir());
                        let y = src + ax.dir() * (x - src).dot(ax.dir());
                        (x.distance(y) < 1e-6).then_some(x)?
                    }
                    None => closest_on_line(src, ax.dir(), ro, rd),
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
            })
    }
}

/// A point with no inference: on the ground plane, or on the horizontal plane through
/// the anchor; a vertical plane facing the camera when looking almost level.
fn free_point(ro: DVec3, rd: DVec3, anchor: Option<DVec3>) -> DVec3 {
    let base = anchor.unwrap_or(DVec3::ZERO);
    let hit_plane = |n: DVec3| {
        let denom = rd.dot(n);
        (denom.abs() > 1e-9)
            .then(|| (base - ro).dot(n) / denom)
            .filter(|t| *t > 0.0)
            .map(|t| ro + rd * t)
    };
    if rd.z.abs() > 0.1
        && let Some(p) = hit_plane(DVec3::Z)
    {
        return p;
    }
    let facing = DVec3::new(rd.x, rd.y, 0.0).normalize_or_zero();
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

    const VIEWPORT: DVec2 = DVec2::new(800.0, 600.0);

    fn camera() -> Camera {
        Camera {
            eye: DVec3::new(-300.0, -500.0, 300.0),
            target: DVec3::new(50.0, 50.0, 0.0),
            fov_deg: 35.0,
            perspective: true,
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
    fn free_point_lands_on_ground() {
        let g = Geometry::default();
        let b = Bvh::default();
        let mut e = Engine::default();
        let i = infer(&mut e, &g, &b, DVec2::new(600.0, 500.0), None);
        assert_eq!(i.kind, Kind::Free);
        assert!(i.point.z.abs() < 1e-6);
    }
}
