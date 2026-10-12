//! Geometry helpers of the IFC importer ([`super::ifc`]) that need no file access:
//! B-spline curves and surfaces, the elementary curved surfaces of advanced breps,
//! tessellation of a trimmed curved face from its boundary loops, sweeping profile
//! rings along a path, and a small PNG writer for `IfcPixelTexture`.

use std::f64::consts::{PI, TAU};
use std::io::Write as _;

use glam::{DMat4, DVec2, DVec3};

/// A (rational) B-spline curve or one direction of a surface: degree, the full knot
/// vector (multiplicities expanded).
#[derive(Debug, Clone)]
pub(crate) struct Knots {
    pub degree: usize,
    pub knots: Vec<f64>,
}

impl Knots {
    /// The knot vector of IFC's `Knots` and `KnotMultiplicities`, `None` unless it
    /// fits `count` control points.
    pub fn new(degree: usize, mults: &[i64], values: &[f64], count: usize) -> Option<Self> {
        let mut knots = Vec::new();
        for (&m, &k) in mults.iter().zip(values) {
            for _ in 0..m.clamp(0, 64) {
                knots.push(k);
            }
        }
        let ok = degree >= 1
            && count > degree
            && knots.len() == count + degree + 1
            && knots.windows(2).all(|w| w[0] <= w[1]);
        ok.then_some(Self { degree, knots })
    }

    /// The parameter range.
    pub fn domain(&self) -> (f64, f64) {
        let n = self.knots.len() - self.degree - 1;
        (self.knots[self.degree], self.knots[n])
    }

    /// Distinct knot values inside the domain (span ends).
    pub fn breaks(&self) -> Vec<f64> {
        let (a, b) = self.domain();
        let mut out: Vec<f64> = Vec::new();
        for &k in &self.knots {
            if k >= a && k <= b && out.last().is_none_or(|l| k > *l) {
                out.push(k);
            }
        }
        out
    }

    /// The span index and the `degree + 1` non-zero basis functions at `u` (The
    /// NURBS Book, A2.1 and A2.2).
    fn basis(&self, u: f64) -> (usize, Vec<f64>) {
        let p = self.degree;
        let n = self.knots.len() - p - 2;
        let (a, b) = self.domain();
        let u = u.clamp(a, b);
        let span = if u >= self.knots[n + 1] {
            // The last non-empty span.
            let mut s = n;
            while s > p && self.knots[s] >= self.knots[s + 1] {
                s -= 1;
            }
            s
        } else {
            let mut lo = p;
            let mut hi = n + 1;
            let mut mid = (lo + hi) / 2;
            while u < self.knots[mid] || u >= self.knots[mid + 1] {
                if u < self.knots[mid] {
                    hi = mid;
                } else {
                    lo = mid;
                }
                mid = (lo + hi) / 2;
                if hi - lo <= 1 {
                    mid = lo;
                    break;
                }
            }
            mid
        };
        let k = &self.knots;
        let mut out = vec![0.0; p + 1];
        let mut left = vec![0.0; p + 1];
        let mut right = vec![0.0; p + 1];
        out[0] = 1.0;
        for j in 1..=p {
            left[j] = u - k[span + 1 - j];
            right[j] = k[span + j] - u;
            let mut saved = 0.0;
            for r in 0..j {
                let d = right[r + 1] + left[j - r];
                let t = if d.abs() > 0.0 { out[r] / d } else { 0.0 };
                out[r] = saved + right[r + 1] * t;
                saved = left[j - r] * t;
            }
            out[j] = saved;
        }
        (span, out)
    }
}

/// A (rational) B-spline curve.
#[derive(Debug, Clone)]
pub(crate) struct BSplineCurve {
    pub knots: Knots,
    pub points: Vec<DVec3>,
    pub weights: Vec<f64>,
}

impl BSplineCurve {
    pub fn eval(&self, u: f64) -> DVec3 {
        let (span, n) = self.knots.basis(u);
        let p = self.knots.degree;
        let mut acc = DVec3::ZERO;
        let mut w = 0.0;
        for (k, b) in n.iter().enumerate() {
            let i = span - p + k;
            let wi = self.weights.get(i).copied().unwrap_or(1.0);
            acc += self.points[i] * (b * wi);
            w += b * wi;
        }
        if w.abs() > 1e-300 { acc / w } else { acc }
    }

    /// Points along the curve: the knots, and 8 steps per span unless it is linear.
    pub fn sample(&self) -> Vec<DVec3> {
        let breaks = self.knots.breaks();
        let per = if self.knots.degree == 1 && self.weights.is_empty() {
            1
        } else {
            8
        };
        let per = per.min((400 / breaks.len().max(1)).max(1));
        let mut out = Vec::new();
        for w in breaks.windows(2) {
            for s in 0..per {
                out.push(self.eval(w[0] + (w[1] - w[0]) * s as f64 / per as f64));
            }
        }
        if let Some(&last) = breaks.last() {
            out.push(self.eval(last));
        }
        out
    }
}

/// A (rational) B-spline surface; `points[i][j]`: `i` along u, `j` along v.
#[derive(Debug, Clone)]
pub(crate) struct BSplineSurface {
    pub u: Knots,
    pub v: Knots,
    pub points: Vec<Vec<DVec3>>,
    pub weights: Vec<Vec<f64>>,
}

impl BSplineSurface {
    pub fn eval(&self, uv: DVec2) -> DVec3 {
        let (su, nu) = self.u.basis(uv.x);
        let (sv, nv) = self.v.basis(uv.y);
        let (pu, pv) = (self.u.degree, self.v.degree);
        let mut acc = DVec3::ZERO;
        let mut w = 0.0;
        for (a, bu) in nu.iter().enumerate() {
            let i = su - pu + a;
            for (b, bv) in nv.iter().enumerate() {
                let j = sv - pv + b;
                let wij = self
                    .weights
                    .get(i)
                    .and_then(|r| r.get(j))
                    .copied()
                    .unwrap_or(1.0);
                let f = bu * bv * wij;
                acc += self.points[i][j] * f;
                w += f;
            }
        }
        if w.abs() > 1e-300 { acc / w } else { acc }
    }
}

/// A curved face surface, in the coordinates of the item being read.
#[derive(Debug, Clone)]
pub(crate) enum Surface {
    /// `IfcCylindricalSurface`: u angle, v height.
    Cylinder {
        m: DMat4,
        r: f64,
    },
    /// `IfcConicalSurface`: radius `r + v tan`.
    Cone {
        m: DMat4,
        r: f64,
        tan: f64,
    },
    /// `IfcSphericalSurface`: u longitude, v latitude.
    Sphere {
        m: DMat4,
        r: f64,
    },
    /// `IfcToroidalSurface`: u around the axis, v around the tube.
    Torus {
        m: DMat4,
        big: f64,
        r: f64,
    },
    BSpline(BSplineSurface),
}

impl Surface {
    pub fn eval(&self, uv: DVec2) -> DVec3 {
        let (u, v) = (uv.x, uv.y);
        match self {
            Self::Cylinder { m, r } => m.transform_point3(DVec3::new(r * u.cos(), r * u.sin(), v)),
            Self::Cone { m, r, tan } => {
                let rr = r + v * tan;
                m.transform_point3(DVec3::new(rr * u.cos(), rr * u.sin(), v))
            }
            Self::Sphere { m, r } => m.transform_point3(DVec3::new(
                r * v.cos() * u.cos(),
                r * v.cos() * u.sin(),
                r * v.sin(),
            )),
            Self::Torus { m, big, r } => {
                let q = big + r * v.cos();
                m.transform_point3(DVec3::new(q * u.cos(), q * u.sin(), r * v.sin()))
            }
            Self::BSpline(s) => s.eval(uv),
        }
    }

    /// Periods of u and v (`None`: not periodic).
    fn periods(&self) -> [Option<f64>; 2] {
        match self {
            Self::Cylinder { .. } | Self::Cone { .. } | Self::Sphere { .. } => [Some(TAU), None],
            Self::Torus { .. } => [Some(TAU), Some(TAU)],
            Self::BSpline(_) => [None, None],
        }
    }

    /// Largest parameter step between interior points per direction (`None`: the
    /// surface is straight that way and needs none).
    fn steps(&self) -> [Option<f64>; 2] {
        let a = Some(TAU / super::ifc::CIRCLE_SEGMENTS as f64);
        match self {
            Self::Cylinder { .. } | Self::Cone { .. } => [None, None],
            Self::Sphere { .. } | Self::Torus { .. } => [a, a],
            Self::BSpline(s) => {
                let step = |k: &Knots| {
                    let (a, b) = k.domain();
                    let n = ((k.breaks().len().saturating_sub(1)) * 4).clamp(8, 32);
                    if k.degree == 1 {
                        None
                    } else {
                        Some((b - a) / n as f64)
                    }
                };
                [step(&s.u), step(&s.v)]
            }
        }
    }

    /// Lengths per unit parameter near `uv` (to make parameter space roughly
    /// isotropic for triangulation).
    fn metric(&self, uv: DVec2) -> DVec2 {
        let h = 1e-4;
        let d = |a: DVec2, b: DVec2, s: f64| (self.eval(a) - self.eval(b)).length() / s;
        let (du, dv) = match self {
            Self::BSpline(s) => {
                let (a, b) = s.u.domain();
                let (c, e) = s.v.domain();
                ((b - a) * h, (e - c) * h)
            }
            _ => (h, h),
        };
        let mu = d(uv + DVec2::new(du, 0.0), uv - DVec2::new(du, 0.0), 2.0 * du);
        let mv = d(uv + DVec2::new(0.0, dv), uv - DVec2::new(0.0, dv), 2.0 * dv);
        DVec2::new(mu.max(1e-9), mv.max(1e-9))
    }

    /// The parameters of the surface point nearest `p` (closed form for the
    /// elementary surfaces, a grid search and Gauss-Newton steps for B-splines).
    fn inverse(&self, p: DVec3, grid: &[(DVec2, DVec3)]) -> DVec2 {
        let local = |m: &DMat4| m.inverse().transform_point3(p);
        match self {
            Self::Cylinder { m, .. } | Self::Cone { m, .. } => {
                let l = local(m);
                DVec2::new(l.y.atan2(l.x), l.z)
            }
            Self::Sphere { m, .. } => {
                let l = local(m);
                let h = (l.x * l.x + l.y * l.y).sqrt();
                DVec2::new(l.y.atan2(l.x), l.z.atan2(h))
            }
            Self::Torus { m, big, .. } => {
                let l = local(m);
                let q = (l.x * l.x + l.y * l.y).sqrt() - big;
                DVec2::new(l.y.atan2(l.x), l.z.atan2(q))
            }
            Self::BSpline(s) => {
                let (u0, u1) = s.u.domain();
                let (v0, v1) = s.v.domain();
                let mut uv = grid
                    .iter()
                    .min_by(|a, b| {
                        (a.1 - p)
                            .length_squared()
                            .total_cmp(&(b.1 - p).length_squared())
                    })
                    .map_or(DVec2::new(u0, v0), |g| g.0);
                let (hu, hv) = ((u1 - u0) * 1e-6, (v1 - v0) * 1e-6);
                for _ in 0..24 {
                    let q = s.eval(uv);
                    let su = (s.eval(uv + DVec2::new(hu, 0.0)) - q) / hu;
                    let sv = (s.eval(uv + DVec2::new(0.0, hv)) - q) / hv;
                    let r = p - q;
                    let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
                    let det = a * c - b * b;
                    if det.abs() < 1e-300 {
                        break;
                    }
                    let (ru, rv) = (su.dot(r), sv.dot(r));
                    let step = DVec2::new((c * ru - b * rv) / det, (a * rv - b * ru) / det);
                    let next =
                        DVec2::new((uv.x + step.x).clamp(u0, u1), (uv.y + step.y).clamp(v0, v1));
                    let done = (next - uv).length() < 1e-12 * (1.0 + (u1 - u0) + (v1 - v0));
                    uv = next;
                    if done {
                        break;
                    }
                }
                uv
            }
        }
    }

    /// Whether `uv` is a singular point where u is undefined (sphere poles).
    fn singular(&self, uv: DVec2) -> bool {
        matches!(self, Self::Sphere { .. }) && uv.y.cos().abs() < 1e-9
    }

    /// Samples for the B-spline inverse's starting guess.
    fn grid(&self) -> Vec<(DVec2, DVec3)> {
        let Self::BSpline(s) = self else {
            return Vec::new();
        };
        let (u0, u1) = s.u.domain();
        let (v0, v1) = s.v.domain();
        let n = 24;
        let mut out = Vec::with_capacity((n + 1) * (n + 1));
        for i in 0..=n {
            for j in 0..=n {
                let uv = DVec2::new(
                    u0 + (u1 - u0) * i as f64 / n as f64,
                    v0 + (v1 - v0) * j as f64 / n as f64,
                );
                out.push((uv, s.eval(uv)));
            }
        }
        out
    }
}

fn point_in(p: DVec2, poly: &[DVec2]) -> bool {
    crate::geom::point_in_polygon(p, poly)
}

fn segment_distance(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length()
}

/// Triangles covering the face of `surface` bounded by `loops` (3D points on the
/// surface; the outer loop first): the loops' points are kept exactly (so the face
/// meets its neighbors along the same edges), interior points are added where the
/// surface curves. The triangles face along the surface normal (its u x v
/// direction) when `sense`, else against it. `None` when
/// the loops do not map to closed loops in parameter space (e.g. a band around a
/// cylinder bounded by two circles).
pub(crate) fn surface_triangles(
    surface: &Surface,
    loops: &[Vec<DVec3>],
    sense: bool,
) -> Option<Vec<[DVec3; 3]>> {
    let grid = surface.grid();
    let periods = surface.periods();
    let mut uv_loops: Vec<Vec<DVec2>> = Vec::new();
    for l in loops {
        if l.len() < 3 {
            continue;
        }
        let mut uv: Vec<DVec2> = l.iter().map(|&p| surface.inverse(p, &grid)).collect();
        // Poles: u from a neighbor.
        let n = uv.len();
        for k in 0..n {
            if surface.singular(uv[k]) {
                let prev = uv[(k + n - 1) % n];
                let next = uv[(k + 1) % n];
                let pick = if !surface.singular(prev) { prev } else { next };
                uv[k].x = pick.x;
            }
        }
        // Unwrap periodic parameters along the loop.
        for d in 0..2 {
            let Some(period) = periods[d] else { continue };
            for k in 1..n {
                let prev = uv[k - 1][d];
                let mut x = uv[k][d];
                while x - prev > period / 2.0 {
                    x -= period;
                }
                while prev - x > period / 2.0 {
                    x += period;
                }
                uv[k][d] = x;
            }
            if (uv[n - 1][d] - uv[0][d]).abs() > period / 2.0 {
                return None;
            }
        }
        uv_loops.push(uv);
    }
    let outer = uv_loops.first()?.clone();
    // Holes next to the outline's periodic copy.
    for l in uv_loops.iter_mut().skip(1) {
        for d in 0..2 {
            let Some(period) = periods[d] else { continue };
            let c0 = outer.iter().map(|p| p[d]).sum::<f64>() / outer.len() as f64;
            let c = l.iter().map(|p| p[d]).sum::<f64>() / l.len() as f64;
            let shift = ((c0 - c) / period).round() * period;
            for p in l.iter_mut() {
                p[d] += shift;
            }
        }
    }
    // Scale parameters to lengths near the middle.
    let lo = outer
        .iter()
        .copied()
        .fold(DVec2::splat(f64::MAX), DVec2::min);
    let hi = outer
        .iter()
        .copied()
        .fold(DVec2::splat(f64::MIN), DVec2::max);
    let k = surface.metric((lo + hi) / 2.0);
    let scaled: Vec<Vec<DVec2>> = uv_loops
        .iter()
        .map(|l| l.iter().map(|p| *p * k).collect())
        .collect();
    let mut boundary: Vec<DVec2> = Vec::new();
    let mut boundary_3d: Vec<DVec3> = Vec::new();
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut starts = Vec::new();
    let kept: Vec<&Vec<DVec3>> = loops.iter().filter(|l| l.len() >= 3).collect();
    for (l, pts) in scaled.iter().zip(&kept) {
        let base = boundary.len();
        starts.push(base);
        for i in 0..l.len() {
            segments.push((base + i, base + (i + 1) % l.len()));
        }
        boundary.extend(l.iter().copied());
        boundary_3d.extend(pts.iter().copied());
    }
    let size = ((hi - lo) * k).max_element();
    if size.is_nan() || size <= 0.0 || !size.is_finite() {
        return None;
    }
    let inside = |p: DVec2| point_in(p, &scaled[0]) && scaled[1..].iter().all(|h| !point_in(p, h));
    let mut steps = surface.steps();
    // Curved one way only: interior points as close the other way.
    match steps {
        [Some(a), None] => steps[1] = Some(a * k.x / k.y),
        [None, Some(b)] => steps[0] = Some(b * k.y / k.x),
        _ => {}
    }
    let interior = |spacing_scale: f64| -> Vec<DVec2> {
        let (Some(a), Some(b)) = (steps[0], steps[1]) else {
            return Vec::new();
        };
        let (mut su, mut sv) = (a * k.x * spacing_scale, b * k.y * spacing_scale);
        let ext = (hi - lo) * k;
        while (ext.x / su) * (ext.y / sv) > 4000.0 {
            su *= 1.5;
            sv *= 1.5;
        }
        let clear = 0.45 * su.min(sv);
        let (lo_s, nx, ny) = (
            lo * k,
            (ext.x / su).floor() as usize,
            (ext.y / sv).floor() as usize,
        );
        let mut out = Vec::new();
        for i in 1..=nx {
            for j in 1..=ny {
                let p = lo_s + DVec2::new(i as f64 * su, j as f64 * sv);
                if inside(p)
                    && segments
                        .iter()
                        .all(|&(a, b)| segment_distance(p, boundary[a], boundary[b]) > clear)
                {
                    out.push(p);
                }
            }
        }
        out
    };
    let eps = size * 1e-9;
    let try_delaunay = |extra: &[DVec2]| -> Option<Vec<[usize; 3]>> {
        let mut pts = boundary.clone();
        pts.extend_from_slice(extra);
        let tris = crate::delaunay::triangulate(&pts, eps);
        let mut edges = std::collections::HashSet::new();
        for t in &tris {
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                edges.insert((a.min(b), a.max(b)));
            }
        }
        if segments
            .iter()
            .all(|&(a, b)| edges.contains(&(a.min(b), a.max(b))))
        {
            Some(
                tris.into_iter()
                    .filter(|t| inside((pts[t[0]] + pts[t[1]] + pts[t[2]]) / 3.0))
                    .collect(),
            )
        } else {
            None
        }
    };
    let mut extra = interior(1.0);
    let mut tris = try_delaunay(&extra);
    if tris.is_none() && !extra.is_empty() {
        extra = interior(2.0);
        tris = try_delaunay(&extra);
    }
    if tris.is_none() {
        extra.clear();
        let flat: Vec<f64> = boundary.iter().flat_map(|p| [p.x, p.y]).collect();
        let idx = earcutr::earcut(&flat, &starts[1..], 2).ok()?;
        tris = Some(
            idx.as_chunks::<3>()
                .0
                .iter()
                .map(|t| [t[0], t[1], t[2]])
                .collect(),
        );
    }
    let tris = tris?;
    let mut all = boundary.clone();
    all.extend_from_slice(&extra);
    let ccw = sense;
    let at = |i: usize| -> DVec3 {
        if i < boundary_3d.len() {
            boundary_3d[i]
        } else {
            surface.eval(all[i] / k)
        }
    };
    let out = tris
        .into_iter()
        .filter_map(|[a, b, c]| {
            let turn = (all[b] - all[a]).perp_dot(all[c] - all[a]);
            if turn.abs() <= eps * eps {
                return None;
            }
            let (b, c) = if (turn > 0.0) == ccw { (b, c) } else { (c, b) };
            Some([at(a), at(b), at(c)])
        })
        .collect();
    Some(out)
}

/// Rings of the profile `loops` (2D, in the profile plane) swept along `path`: the
/// profile stands at the first point with its x axis along `x0` (made
/// perpendicular to the path) and its normal along the path, and is carried along
/// by projecting each ring along the segment onto the plane halving the corner
/// (mitered). A `closed` path returns to its first point; its last ring is then the
/// first one. `None` for a path with fewer than two distinct points.
pub(crate) fn sweep_rings(
    path: &[DVec3],
    x0: DVec3,
    loops: &[Vec<DVec2>],
    closed: bool,
) -> Option<Vec<Vec<Vec<DVec3>>>> {
    let mut path: Vec<DVec3> = path.to_vec();
    if closed && path.len() > 2 {
        path.push(path[0]);
    }
    let dirs: Vec<DVec3> = path
        .windows(2)
        .map(|w| (w[1] - w[0]).normalize_or_zero())
        .collect();
    if dirs.is_empty() || dirs.contains(&DVec3::ZERO) {
        return None;
    }
    let closed = closed && path.len() > 3;
    let t0 = dirs[0];
    let x = (x0 - t0 * x0.dot(t0))
        .try_normalize()
        .unwrap_or_else(|| t0.any_orthonormal_vector());
    let y = t0.cross(x);
    let place = |o: DVec3| -> Vec<Vec<DVec3>> {
        loops
            .iter()
            .map(|l| l.iter().map(|p| o + x * p.x + y * p.y).collect())
            .collect()
    };
    let project = |ring: &Vec<Vec<DVec3>>, d: DVec3, at: DVec3, n: DVec3, fallback: f64| {
        let denom = d.dot(n);
        ring.iter()
            .map(|l| {
                l.iter()
                    .map(|&q| {
                        let s = if denom.abs() > 0.2 {
                            (at - q).dot(n) / denom
                        } else {
                            fallback
                        };
                        q + d * s
                    })
                    .collect()
            })
            .collect::<Vec<Vec<DVec3>>>()
    };
    let mut first = place(path[0]);
    if closed {
        let last = dirs[dirs.len() - 1];
        let n = (last + t0).try_normalize().unwrap_or(t0);
        first = project(&first, t0, path[0], n, 0.0);
    }
    let mut rings = vec![first];
    for i in 1..path.len() {
        let d_in = dirs[i - 1];
        let d_out = match dirs.get(i) {
            Some(&d) => Some(d),
            None if closed => Some(dirs[0]),
            None => None,
        };
        let n = d_out.map_or(d_in, |d| (d_in + d).try_normalize().unwrap_or(d_in));
        let len = (path[i] - path[i - 1]).length();
        let ring = project(rings.last().expect("a ring"), d_in, path[i], n, len);
        rings.push(ring);
    }
    if closed {
        let first = rings[0].clone();
        *rings.last_mut().expect("a ring") = first;
    }
    Some(rings)
}

/// A circle of `n` points (counter-clockwise) of radius `r`.
pub(crate) fn circle_n(r: f64, n: usize) -> Vec<DVec2> {
    (0..n)
        .map(|k| {
            let a = TAU * k as f64 / n as f64;
            DVec2::new(r * a.cos(), r * a.sin())
        })
        .collect()
}

/// Latitude rings of a sphere of radius `r` (south pole first), `stacks` bands.
pub(crate) fn sphere_rings(r: f64, stacks: usize, slices: usize) -> Vec<Vec<DVec3>> {
    (0..=stacks)
        .map(|i| {
            let lat = -PI / 2.0 + PI * i as f64 / stacks as f64;
            circle_n(r * lat.cos(), slices)
                .into_iter()
                .map(|p| p.extend(r * lat.sin()))
                .collect()
        })
        .collect()
}

/// A PNG of `width` x `height` pixels with `channels` (1 grey, 2 grey and alpha, 3
/// RGB, 4 RGBA) bytes each, rows top to bottom.
pub(crate) fn png(width: u32, height: u32, channels: u8, pixels: &[u8]) -> Option<Vec<u8>> {
    let color_type = match channels {
        1 => 0,
        2 => 4,
        3 => 2,
        4 => 6,
        _ => return None,
    };
    let row = width as usize * channels as usize;
    if width == 0 || height == 0 || pixels.len() != row * height as usize {
        return None;
    }
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for r in pixels.chunks(row) {
        raw.push(0);
        raw.extend_from_slice(r);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).ok()?;
    let data = z.finish().ok()?;
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(body);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &data);
    chunk(b"IEND", &[]);
    Some(out)
}

/// The bytes of a STEP binary (`"0A1B"`: the first digit counts unused leading
/// bits, a multiple of 4 here, which is skipped).
pub(crate) fn binary_bytes(hex: &str) -> Vec<u8> {
    let digits: Vec<u8> = hex
        .bytes()
        .filter_map(|c| (c as char).to_digit(16).map(|d| d as u8))
        .collect();
    let Some((&unused, rest)) = digits.split_first() else {
        return Vec::new();
    };
    // Skip whole unused nibbles.
    let rest = &rest[(unused as usize / 4).min(rest.len())..];
    let rest = if rest.len() % 2 == 1 {
        &rest[1..]
    } else {
        rest
    };
    rest.chunks(2).map(|c| (c[0] << 4) | c[1]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bspline_line_and_quarter_circle() {
        // Degree 1 with two points: a line.
        let k = Knots::new(1, &[2, 2], &[0.0, 1.0], 2).unwrap();
        let c = BSplineCurve {
            knots: k,
            points: vec![DVec3::ZERO, DVec3::X * 2.0],
            weights: vec![],
        };
        assert!((c.eval(0.25) - DVec3::X * 0.5).length() < 1e-12);
        // Rational quadratic quarter circle.
        let k = Knots::new(2, &[3, 3], &[0.0, 1.0], 3).unwrap();
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let c = BSplineCurve {
            knots: k,
            points: vec![DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y],
            weights: vec![1.0, w, 1.0],
        };
        for p in c.sample() {
            assert!((p.length() - 1.0).abs() < 1e-12);
        }
        assert!(Knots::new(2, &[1, 1], &[0.0, 1.0], 3).is_none());
    }

    #[test]
    fn curved_patch_keeps_its_boundary() {
        // A bilinear-in-u, quadratic-in-v patch bulging up; boundary loop sampled.
        let u = Knots::new(1, &[2, 2], &[0.0, 1.0], 2).unwrap();
        let v = Knots::new(2, &[3, 3], &[0.0, 1.0], 3).unwrap();
        let points = vec![
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(0.0, 1.0, 1.0),
                DVec3::new(0.0, 2.0, 0.0),
            ],
            vec![
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 1.0, 1.0),
                DVec3::new(2.0, 2.0, 0.0),
            ],
        ];
        let s = Surface::BSpline(BSplineSurface {
            u,
            v,
            points,
            weights: vec![],
        });
        let mut outline = Vec::new();
        for i in 0..4 {
            outline.push(s.eval(DVec2::new(i as f64 / 4.0, 0.0)));
        }
        for j in 0..8 {
            outline.push(s.eval(DVec2::new(1.0, j as f64 / 8.0)));
        }
        for i in (1..=4).rev() {
            outline.push(s.eval(DVec2::new(i as f64 / 4.0, 1.0)));
        }
        for j in (1..=8).rev() {
            outline.push(s.eval(DVec2::new(0.0, j as f64 / 8.0)));
        }
        let tris = surface_triangles(&s, &[outline.clone()], true).unwrap();
        let area: f64 = tris
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
            .sum();
        // The arch of a parabola z = 2t(1-t)... its length times 2.
        assert!(area > 4.0 && area < 6.0, "{area}");
        // Every boundary point is used as is.
        for p in &outline {
            assert!(tris.iter().flatten().any(|q| q == p));
        }
        // Normals upward (the outline runs counter-clockwise seen from above).
        let n: DVec3 = tris.iter().map(|[a, b, c]| (*b - *a).cross(*c - *a)).sum();
        assert!(n.z > 0.0);
    }

    #[test]
    fn png_and_binary() {
        let p = png(2, 1, 3, &[255, 0, 0, 0, 255, 0]).unwrap();
        assert!(super::super::is_image(&p));
        assert_eq!(binary_bytes("0FF10"), vec![0xff, 0x10]);
        assert_eq!(binary_bytes("\"00A\""), vec![0x0a]);
    }
}
