//! 2D Delaunay triangulation (Bowyer-Watson), for terrain from contours (Sandbox).
//!
//! Points are inserted in order of x; a triangle whose circumcircle lies entirely left
//! of the current point can no longer change and is set aside (Paul Bourke's
//! "Triangulate"), which keeps the work well below quadratic for scattered points.

use glam::DVec2;

#[derive(Debug, Clone, Copy)]
struct Tri {
    v: [usize; 3],
    center: DVec2,
    r2: f64,
}

/// Circumcircle of `a b c`, `None` when they are (nearly) collinear.
fn circumcircle(a: DVec2, b: DVec2, c: DVec2) -> Option<(DVec2, f64)> {
    let (b, c) = (b - a, c - a);
    let d = 2.0 * (b.x * c.y - b.y * c.x);
    let scale = b.length_squared().max(c.length_squared());
    if d.abs() <= 1e-12 * scale {
        return None;
    }
    let (bb, cc) = (b.length_squared(), c.length_squared());
    let center = DVec2::new(c.y * bb - b.y * cc, b.x * cc - c.x * bb) / d;
    Some((a + center, center.length_squared()))
}

/// Delaunay triangles of `points` as counter-clockwise index triples into `points`.
/// Points closer than `eps` to an earlier one are skipped (they get no triangle);
/// fewer than three distinct points, or all of them on one line, give no triangles.
pub fn triangulate(points: &[DVec2], eps: f64) -> Vec<[usize; 3]> {
    let mut order: Vec<usize> = (0..points.len())
        .filter(|&i| points[i].is_finite())
        .collect();
    order.sort_by(|&i, &j| {
        points[i]
            .x
            .total_cmp(&points[j].x)
            .then(points[i].y.total_cmp(&points[j].y))
    });
    // Drop duplicates: in x order a duplicate is among the points within eps in x.
    let mut kept: Vec<usize> = Vec::with_capacity(order.len());
    for &i in &order {
        let p = points[i];
        let dup = kept
            .iter()
            .rev()
            .take_while(|&&k| p.x - points[k].x <= eps)
            .any(|&k| points[k].distance(p) <= eps);
        if !dup {
            kept.push(i);
        }
    }
    if kept.len() < 3 {
        return Vec::new();
    }
    let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for &i in &kept {
        lo = lo.min(points[i]);
        hi = hi.max(points[i]);
    }
    let size = (hi - lo).max_element().max(eps).max(1e-9);
    let mid = (lo + hi) / 2.0;
    // Vertices of a triangle far around everything, after the input points. Being
    // finite, it can leave out slivers along nearly straight stretches of the convex
    // hull (a fraction of a percent of the hull's length); the tests check exact hulls
    // of scattered points.
    let n = points.len();
    let sup = [
        mid + DVec2::new(-2.0 * 1000.0 * size, -1000.0 * size),
        mid + DVec2::new(2.0 * 1000.0 * size, -1000.0 * size),
        mid + DVec2::new(0.0, 2.0 * 1000.0 * size),
    ];
    let pos = |i: usize| if i < n { points[i] } else { sup[i - n] };
    let make = |v: [usize; 3]| -> Option<Tri> {
        let (a, b, c) = (pos(v[0]), pos(v[1]), pos(v[2]));
        let (center, r2) = circumcircle(a, b, c)?;
        // Counter-clockwise.
        let v = if (b - a).perp_dot(c - a) > 0.0 {
            v
        } else {
            [v[0], v[2], v[1]]
        };
        Some(Tri { v, center, r2 })
    };
    let mut open: Vec<Tri> = make([n, n + 1, n + 2]).into_iter().collect();
    let mut done: Vec<Tri> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for &i in &kept {
        let p = points[i];
        edges.clear();
        let mut k = 0;
        while k < open.len() {
            let t = open[k];
            let dx = p.x - t.center.x;
            if dx > 0.0 && dx * dx > t.r2 {
                // The circumcircle is entirely left of p and of every later point.
                done.push(open.swap_remove(k));
                continue;
            }
            if p.distance_squared(t.center) < t.r2 * (1.0 - 1e-12) {
                let [a, b, c] = t.v;
                edges.extend([(a, b), (b, c), (c, a)]);
                open.swap_remove(k);
                continue;
            }
            k += 1;
        }
        // The cavity's boundary: edges of one removed triangle only.
        let key = |&(a, b): &(usize, usize)| (a.min(b), a.max(b));
        let mut sorted: Vec<(usize, usize)> = edges.iter().map(key).collect();
        sorted.sort_unstable();
        for &(a, b) in &edges {
            let kk = key(&(a, b));
            let count = sorted.partition_point(|x| *x < kk);
            let shared = sorted.get(count + 1) == Some(&kk);
            if !shared && let Some(t) = make([a, b, i]) {
                open.push(t);
            }
        }
    }
    done.extend(open);
    done.into_iter()
        .filter(|t| t.v.iter().all(|&v| v < n))
        .map(|t| t.v)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random points in [0, 100)^2.
    fn scatter(n: usize, seed: u64) -> Vec<DVec2> {
        let mut s = seed;
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64 * 100.0
        };
        (0..n).map(|_| DVec2::new(next(), next())).collect()
    }

    /// Convex hull vertex count (monotone chain, collinear points left out).
    fn hull_size(points: &[DVec2]) -> usize {
        let mut p = points.to_vec();
        p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        let mut h: Vec<DVec2> = Vec::new();
        for pass in 0..2 {
            let start = h.len();
            for &q in &p {
                while h.len() >= start + 2 {
                    let (a, b) = (h[h.len() - 2], h[h.len() - 1]);
                    if (b - a).perp_dot(q - a) <= 0.0 {
                        h.pop();
                    } else {
                        break;
                    }
                }
                h.push(q);
            }
            h.pop();
            if pass == 0 {
                p.reverse();
            }
        }
        h.len()
    }

    #[test]
    fn empty_circumcircles_ccw_and_euler_count() {
        for (n, seed) in [(3, 1), (10, 2), (200, 3), (1000, 4)] {
            let pts = scatter(n, seed);
            let tris = triangulate(&pts, 1e-9);
            assert_eq!(tris.len(), 2 * n - 2 - hull_size(&pts), "n = {n}");
            for t in &tris {
                let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
                assert!((b - a).perp_dot(c - a) > 0.0, "counter-clockwise");
                let (center, r2) = circumcircle(a, b, c).unwrap();
                for (i, p) in pts.iter().enumerate() {
                    if !t.contains(&i) {
                        assert!(
                            p.distance_squared(center) >= r2 * (1.0 - 1e-9),
                            "point {i} inside the circumcircle of {t:?}"
                        );
                    }
                }
            }
            // Every point is used.
            let mut used = vec![false; n];
            for t in &tris {
                for &v in t {
                    used[v] = true;
                }
            }
            assert!(used.iter().all(|&u| u));
        }
    }

    #[test]
    fn degenerate_inputs() {
        let line: Vec<DVec2> = (0..5).map(|i| DVec2::new(i as f64, 0.0)).collect();
        assert!(triangulate(&line, 1e-9).is_empty());
        assert!(triangulate(&line[..2], 1e-9).is_empty());
        // A square (four cocircular points) and a duplicate corner.
        let sq = [
            DVec2::ZERO,
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
            DVec2::new(1.0, 1.0),
        ];
        let tris = triangulate(&sq, 1e-6);
        assert_eq!(tris.len(), 2);
        let area: f64 = tris
            .iter()
            .map(|t| (sq[t[1]] - sq[t[0]]).perp_dot(sq[t[2]] - sq[t[0]]) / 2.0)
            .sum();
        assert!((area - 1.0).abs() < 1e-12);
    }

    #[test]
    fn regular_grid_covers_its_area() {
        let pts: Vec<DVec2> = (0..11)
            .flat_map(|i| (0..11).map(move |j| DVec2::new(i as f64, j as f64)))
            .collect();
        let tris = triangulate(&pts, 1e-9);
        assert_eq!(tris.len(), 200);
        let area: f64 = tris
            .iter()
            .map(|t| (pts[t[1]] - pts[t[0]]).perp_dot(pts[t[2]] - pts[t[0]]) / 2.0)
            .sum();
        assert!((area - 100.0).abs() < 1e-9);
    }
}
