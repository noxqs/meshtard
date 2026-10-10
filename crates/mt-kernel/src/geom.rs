//! Planes, 2D projections and polygon tests.

use glam::{DVec2, DVec3};

use crate::TOL;

/// Plane `normal . p = d` with a unit normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub normal: DVec3,
    pub d: f64,
}

impl Plane {
    pub fn new(normal: DVec3, point: DVec3) -> Self {
        let normal = normal.normalize();
        Self {
            normal,
            d: normal.dot(point),
        }
    }

    /// Best-fit plane of a closed polygon (Newell's method), or `None` if degenerate.
    pub fn from_polygon(points: &[DVec3]) -> Option<Self> {
        let mut n = DVec3::ZERO;
        let mut c = DVec3::ZERO;
        for (i, p) in points.iter().enumerate() {
            let q = points[(i + 1) % points.len()];
            n += DVec3::new(
                (p.y - q.y) * (p.z + q.z),
                (p.z - q.z) * (p.x + q.x),
                (p.x - q.x) * (p.y + q.y),
            );
            c += *p;
        }
        (n.length() > 1e-12).then(|| Self::new(n, c / points.len() as f64))
    }

    pub fn distance(&self, p: DVec3) -> f64 {
        self.normal.dot(p) - self.d
    }

    pub fn contains(&self, p: DVec3) -> bool {
        self.distance(p).abs() <= TOL
    }

    /// Same plane regardless of normal direction.
    pub fn coincides(&self, other: &Plane) -> bool {
        let dot = self.normal.dot(other.normal);
        if dot.abs() < 1.0 - 1e-9 {
            return false;
        }
        (self.d - dot.signum() * other.d).abs() <= TOL
    }

    /// Orthonormal in-plane axes `(u, v)` with `u x v = normal`.
    pub fn basis(&self) -> (DVec3, DVec3) {
        let n = self.normal;
        let helper = if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
        let u = helper.cross(n).normalize();
        let v = n.cross(u);
        (u, v)
    }

    pub fn project(&self, p: DVec3) -> DVec2 {
        let (u, v) = self.basis();
        DVec2::new(p.dot(u), p.dot(v))
    }

    pub fn flipped(&self) -> Self {
        Self {
            normal: -self.normal,
            d: -self.d,
        }
    }
}

/// Twice the signed area; positive for counter-clockwise.
pub fn signed_area2(poly: &[DVec2]) -> f64 {
    (0..poly.len())
        .map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()]))
        .sum()
}

/// Even-odd point-in-polygon test (boundary points count as inside or outside arbitrarily).
pub fn point_in_polygon(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// Distance from `p` to segment `a-b` and the segment parameter of the closest point.
pub fn point_segment(p: DVec3, a: DVec3, b: DVec3) -> (f64, f64) {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 == 0.0 {
        return (p.distance(a), 0.0);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p.distance(a + ab * t), t)
}

/// Closest points between segments `p0-p1` and `q0-q1`: `(distance, s, t)` with
/// parameters along each. `None` for parallel segments (overlaps are found through
/// their endpoints instead).
pub fn segment_segment(p0: DVec3, p1: DVec3, q0: DVec3, q1: DVec3) -> Option<(f64, f64, f64)> {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let r = p0 - q0;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let c = d1.dot(r);
    let b = d1.dot(d2);
    let denom = a * e - b * b;
    if denom <= 1e-12 * a * e {
        return None;
    }
    let mut s = ((b * f - c * e) / denom).clamp(0.0, 1.0);
    let mut t = (b * s + f) / e;
    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    let dist = (p0 + d1 * s).distance(q0 + d2 * t);
    Some((dist, s, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_basis_is_right_handed() {
        let p = Plane::new(DVec3::new(1.0, 2.0, 3.0), DVec3::ZERO);
        let (u, v) = p.basis();
        assert!((u.cross(v) - p.normal).length() < 1e-12);
    }

    #[test]
    fn newell_normal_follows_winding() {
        let sq = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        let p = Plane::from_polygon(&sq).unwrap();
        assert!((p.normal - DVec3::Z).length() < 1e-12);
    }

    #[test]
    fn crossing_segments() {
        let (d, s, t) = segment_segment(
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(0.0, -1.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
        assert!(d < 1e-12 && (s - 0.5).abs() < 1e-12 && (t - 0.5).abs() < 1e-12);
    }

    #[test]
    fn polygon_helpers() {
        let sq = [DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y];
        assert_eq!(signed_area2(&sq), 2.0);
        assert!(point_in_polygon(DVec2::splat(0.5), &sq));
        assert!(!point_in_polygon(DVec2::splat(1.5), &sq));
    }
}
