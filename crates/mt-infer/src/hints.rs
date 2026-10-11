//! Shape hints for the drawing tools: SketchUp's "Square" and "Golden Section"
//! diagonals for the Rectangle tool, and "Half Circle", "Quarter Circle",
//! "Three-Quarter Circle" and "Tangent at Vertex" for the arc tools. Each takes the
//! cursor's inference and the shape's state and returns the inference snapped to the
//! hint (with its [`Kind`] for the tooltip), or `None`.
//!
//! Hints apply only to a cursor without a stronger inference ([`accepts_hint`]): a
//! point on the ground or on a face. Tolerances are angles ([`HINT_ANGLE`]) so they do
//! not need the view; SketchUp's own tolerance (likely in pixels) is unverified.

use glam::DVec3;

use crate::{Inference, Kind};

/// Angular snap tolerance of the hints, in radians (2.5 degrees; unverified).
pub const HINT_ANGLE: f64 = 2.5 * std::f64::consts::PI / 180.0;

/// The golden ratio.
pub const GOLDEN_RATIO: f64 = std::f64::consts::GOLDEN_RATIO;

/// Whether a hint may move this inference (free points and points on faces only).
pub fn accepts_hint(kind: Kind) -> bool {
    matches!(kind, Kind::Free | Kind::OnFace)
}

/// Rectangle from corner `a` toward `b` with unit in-plane axes `u`, `v`: when the
/// diagonal is within [`HINT_ANGLE`] of a square's or a golden rectangle's (either
/// orientation), the opposite corner snapped onto that diagonal and the hint kind.
/// `None` when `b` is off the plane or the rectangle is degenerate.
pub fn rectangle_ratio(a: DVec3, b: DVec3, u: DVec3, v: DVec3) -> Option<(DVec3, Kind)> {
    let d = b - a;
    let n = u.cross(v);
    let (w, h) = (d.dot(u), d.dot(v));
    let size = w.abs().max(h.abs());
    if size <= mt_kernel::TOL || w.abs() <= 1e-9 || h.abs() <= 1e-9 {
        return None;
    }
    if d.dot(n).abs() > 1e-6 * size.max(1.0) {
        return None;
    }
    let angle = h.abs().atan2(w.abs());
    let targets = [
        (std::f64::consts::FRAC_PI_4, Kind::Square),
        ((1.0 / GOLDEN_RATIO).atan(), Kind::GoldenSection),
        (GOLDEN_RATIO.atan(), Kind::GoldenSection),
    ];
    let (target, kind) = targets
        .into_iter()
        .map(|(t, k)| ((t - angle).abs(), t, k))
        .filter(|(diff, _, _)| *diff <= HINT_ANGLE)
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, t, k)| (t, k))?;
    let diag = u * w.signum() * target.cos() + v * h.signum() * target.sin();
    Some((a + diag * (w * u + h * v).dot(diag), kind))
}

/// Rectangle tool: `inf` snapped to a "Square" or "Golden Section" diagonal from the
/// first corner `a` (in-plane unit axes `u`, `v`), with the diagonal as its dashed
/// guide (SketchUp draws the diagonal dashed; color unverified).
pub fn rectangle(inf: &Inference, a: DVec3, u: DVec3, v: DVec3) -> Option<Inference> {
    if !accepts_hint(inf.kind) {
        return None;
    }
    let (p, kind) = rectangle_ratio(a, inf.point, u, v)?;
    Some(hinted(inf, p, kind, Some((a, p))))
}

/// The direction into `inf`'s point along the edge it is an endpoint of: the tangent
/// an arc started there continues ("Tangent at Vertex").
pub fn tangent_at(inf: &Inference) -> Option<DVec3> {
    let [s, e] = inf.segment.filter(|_| inf.kind == Kind::Endpoint)?;
    let d = if e.distance(inf.point) <= s.distance(inf.point) {
        e - s
    } else {
        s - e
    };
    let d = d.normalize_or_zero();
    (d != DVec3::ZERO).then_some(d)
}

/// The frame of a 2-point arc from `a` to `b` in the plane with `normal`: chord length,
/// unit chord direction, and the unit bulge direction (`normal x chord`); the same
/// frame as the 2 Point Arc tool's bulge (positive bulge toward `bulge_dir`).
fn chord_frame(a: DVec3, b: DVec3, normal: DVec3) -> Option<(f64, DVec3, DVec3)> {
    let chord = b - a;
    let c = chord.length();
    if c <= mt_kernel::TOL {
        return None;
    }
    let dir = chord / c;
    let n = (normal - dir * normal.dot(dir)).normalize_or_zero();
    if n == DVec3::ZERO {
        return None;
    }
    Some((c, dir, n.cross(dir)))
}

/// Bulge (sagitta, signed as the 2 Point Arc tool's) of the arc from `a` to `b` in the
/// plane with `normal` whose tangent at `a` is `tangent`. `None` when the tangent
/// lies along the chord (a straight line, or no finite arc).
pub fn tangent_bulge(a: DVec3, b: DVec3, normal: DVec3, tangent: DVec3) -> Option<f64> {
    let (c, dir, m) = chord_frame(a, b, normal)?;
    let theta = tangent.dot(m).atan2(tangent.dot(dir));
    if theta.abs() < 1e-6 || theta.abs() > std::f64::consts::PI - 1e-3 {
        return None;
    }
    Some(c / 2.0 * (theta / 2.0).tan())
}

/// 2 Point Arc, bulge step: `inf` snapped so the arc from `a` to `b` (plane `normal`)
/// is a "Half Circle" or, with `tangent` (see [`tangent_at`]; unverified that
/// SketchUp offers it at the bulge step), "Tangent at Vertex". Compared by the arc's
/// tangent angle at `a`; the nearer hint within [`HINT_ANGLE`] wins.
pub fn two_point_arc(
    inf: &Inference,
    a: DVec3,
    b: DVec3,
    normal: DVec3,
    tangent: Option<DVec3>,
) -> Option<Inference> {
    if !accepts_hint(inf.kind) {
        return None;
    }
    let (c, _, m) = chord_frame(a, b, normal)?;
    let bulge = (inf.point - a).dot(m);
    let angle = |s: f64| 2.0 * (2.0 * s / c).atan();
    let cur = angle(bulge);
    let mut targets = Vec::new();
    if bulge.abs() > 1e-9 {
        targets.push((c / 2.0 * bulge.signum(), Kind::HalfCircle));
    }
    if let Some(s) = tangent.and_then(|t| tangent_bulge(a, b, normal, t)) {
        targets.push((s, Kind::Tangent));
    }
    let (s, kind) = targets
        .into_iter()
        .map(|(s, k)| ((angle(s) - cur).abs(), s, k))
        .filter(|(diff, _, _)| *diff <= HINT_ANGLE)
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, s, k)| (s, k))?;
    Some(hinted(inf, inf.point + m * (s - bulge), kind, None))
}

/// Arc (center, start, end), end step: `inf` snapped to a swept angle of 90 ("Quarter
/// Circle"), 180 ("Half Circle") or 270 degrees ("Three-Quarter Circle") when within
/// [`HINT_ANGLE`] of it (the sweep runs counterclockwise about `normal`, as the Arc
/// tool's).
pub fn center_arc(
    inf: &Inference,
    center: DVec3,
    start: DVec3,
    normal: DVec3,
) -> Option<Inference> {
    if !accepts_hint(inf.kind) {
        return None;
    }
    let n = normal.normalize_or_zero();
    let x = (start - center).normalize_or_zero();
    if x == DVec3::ZERO || n == DVec3::ZERO {
        return None;
    }
    let d = inf.point - center;
    let d = d - n * d.dot(n);
    if d.length() <= mt_kernel::TOL {
        return None;
    }
    let y = n.cross(x);
    let sweep = d.dot(y).atan2(d.dot(x)).rem_euclid(std::f64::consts::TAU);
    let q = std::f64::consts::FRAC_PI_2;
    let (target, kind) = [
        (q, Kind::QuarterCircle),
        (2.0 * q, Kind::HalfCircle),
        (3.0 * q, Kind::ThreeQuarterCircle),
    ]
    .into_iter()
    .find(|(t, _)| (t - sweep).abs() <= HINT_ANGLE)?;
    let p = center + (x * target.cos() + y * target.sin()) * d.length();
    Some(hinted(inf, p, kind, None))
}

fn hinted(inf: &Inference, point: DVec3, kind: Kind, guide: Option<(DVec3, DVec3)>) -> Inference {
    Inference {
        point,
        kind,
        guide,
        edge: None,
        segment: None,
        container: None,
        ..*inf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free(p: DVec3) -> Inference {
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

    #[test]
    fn rectangle_square_and_golden() {
        let a = DVec3::new(10.0, 10.0, 0.0);
        let (u, v) = (DVec3::X, DVec3::Y);
        // Nearly square, toward -x: snapped to the 45 degree diagonal.
        let i = rectangle(&free(a + DVec3::new(-50.0, 51.0, 0.0)), a, u, v).unwrap();
        assert_eq!(i.kind, Kind::Square);
        let d = i.point - a;
        assert!((d.x + d.y).abs() < 1e-9 && d.x < 0.0, "{d:?}");
        assert_eq!(i.guide, Some((a, i.point)));
        // Golden in both orientations.
        let i = rectangle(&free(a + DVec3::new(162.0, 100.0, 0.0)), a, u, v).unwrap();
        assert_eq!(i.kind, Kind::GoldenSection);
        let d = i.point - a;
        assert!((d.x / d.y - GOLDEN_RATIO).abs() < 1e-9);
        let i = rectangle(&free(a + DVec3::new(100.0, -160.0, 0.0)), a, u, v).unwrap();
        assert_eq!(i.kind, Kind::GoldenSection);
        let d = i.point - a;
        assert!((-d.y / d.x - GOLDEN_RATIO).abs() < 1e-9);
        // Neither, and not over an endpoint.
        assert!(rectangle(&free(a + DVec3::new(100.0, 70.0, 0.0)), a, u, v).is_none());
        let mut e = free(a + DVec3::new(50.0, 50.0, 0.0));
        e.kind = Kind::Endpoint;
        assert!(rectangle(&e, a, u, v).is_none());
    }

    #[test]
    fn two_point_arc_half_circle_and_tangent() {
        let (a, b) = (DVec3::ZERO, DVec3::new(100.0, 0.0, 0.0));
        let n = DVec3::Z;
        // Bulge direction is +y (z cross x); a bulge of 49 snaps to 50.
        let i = two_point_arc(&free(DVec3::new(40.0, 49.0, 0.0)), a, b, n, None).unwrap();
        assert_eq!(i.kind, Kind::HalfCircle);
        assert!((i.point.y - 50.0).abs() < 1e-9 && (i.point.x - 40.0).abs() < 1e-9);
        let i = two_point_arc(&free(DVec3::new(40.0, -51.0, 0.0)), a, b, n, None).unwrap();
        assert!((i.point.y + 50.0).abs() < 1e-9);
        // An edge arriving at `a` heading 30 degrees below the chord: the tangent arc
        // bulges to -y.
        let t = DVec3::new(30f64.to_radians().cos(), -(30f64.to_radians().sin()), 0.0);
        let s = tangent_bulge(a, b, n, t).unwrap();
        assert!((s - (-50.0 * 15f64.to_radians().tan())).abs() < 1e-9);
        let i = two_point_arc(&free(DVec3::new(50.0, s + 0.5, 0.0)), a, b, n, Some(t)).unwrap();
        assert_eq!(i.kind, Kind::Tangent);
        assert!((i.point.y - s).abs() < 1e-9);
        // The tangent of the resulting arc at `a` is `t`: the center is on the normal
        // to `t` through `a`.
        let r = (2500.0 + s * s) / (2.0 * s.abs());
        let center = DVec3::new(50.0, s - s.signum() * r, 0.0);
        assert!((center - a).dot(t).abs() < 1e-9, "{center:?}");
        // Far from both: no hint.
        assert!(two_point_arc(&free(DVec3::new(50.0, 20.0, 0.0)), a, b, n, Some(t)).is_none());
    }

    #[test]
    fn tangent_from_endpoint_inference() {
        let mut i = free(DVec3::new(5.0, 0.0, 0.0));
        i.kind = Kind::Endpoint;
        i.segment = Some([DVec3::new(5.0, 0.0, 0.0), DVec3::new(5.0, -3.0, 0.0)]);
        assert_eq!(tangent_at(&i), Some(DVec3::Y));
        i.kind = Kind::OnEdge;
        assert_eq!(tangent_at(&i), None);
    }

    #[test]
    fn center_arc_half_circle() {
        let c = DVec3::new(1.0, 1.0, 0.0);
        let s = DVec3::new(11.0, 1.0, 0.0);
        let p = c + DVec3::new(-20.0, 0.5, 0.0);
        let i = center_arc(&free(p), c, s, DVec3::Z).unwrap();
        assert_eq!(i.kind, Kind::HalfCircle);
        assert!((i.point - (c - DVec3::X * DVec3::new(-20.0, 0.5, 0.0).length())).length() < 1e-9);
        assert!(center_arc(&free(c + DVec3::new(-20.0, 5.0, 0.0)), c, s, DVec3::Z).is_none());
    }

    #[test]
    fn center_arc_quarter_and_three_quarter() {
        let c = DVec3::ZERO;
        let s = DVec3::new(10.0, 0.0, 0.0);
        // Counterclockwise about +z: +y is 90 degrees, -y is 270.
        let i = center_arc(&free(DVec3::new(0.5, 20.0, 0.0)), c, s, DVec3::Z).unwrap();
        assert_eq!(i.kind, Kind::QuarterCircle);
        assert!((i.point - DVec3::new(0.0, (400.25f64).sqrt(), 0.0)).length() < 1e-9);
        let i = center_arc(&free(DVec3::new(-0.5, -20.0, 0.0)), c, s, DVec3::Z).unwrap();
        assert_eq!(i.kind, Kind::ThreeQuarterCircle);
        assert!(i.point.x.abs() < 1e-9 && i.point.y < 0.0);
        assert!(i.kind.is_hint());
        assert!(center_arc(&free(DVec3::new(10.0, 10.0, 0.0)), c, s, DVec3::Z).is_none());
    }
}
