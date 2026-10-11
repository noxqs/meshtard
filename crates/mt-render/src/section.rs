//! Section planes: where a cutting plane crosses the model.
//!
//! The viewport clips away everything on the side the plane's normal points to (see
//! [`crate::Viewport::set_section`]); [`section_lines`] gives the cut outline, which
//! SketchUp draws as thick black "section cut" edges; [`section_fill`] gives triangles
//! capping the cut where it crosses closed geometry (View > Section Fill).

use glam::DVec3;

/// An active section plane. Geometry on the side `normal` points to is hidden.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionCut {
    pub point: [f32; 3],
    /// Need not be unit length; zero disables the cut.
    pub normal: [f32; 3],
}

/// Segments where the plane through `point` with `normal` cuts `triangles`.
///
/// One segment per crossed triangle, not merged or deduplicated. Triangles lying in the
/// plane give nothing (their outline is the model's own edges). A triangle edge lying in
/// the plane is returned only when the rest of the triangle is on the kept side
/// (opposite `normal`), so two triangles folding across the plane give it once. Points
/// within a small tolerance (relative to the triangle's size) count as on the plane.
pub fn section_lines(triangles: &[[DVec3; 3]], point: DVec3, normal: DVec3) -> Vec<[DVec3; 2]> {
    let Some(n) = normal.try_normalize() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tri in triangles {
        let scale = (tri[1] - tri[0])
            .abs()
            .max((tri[2] - tri[0]).abs())
            .max_element()
            .max(tri[0].abs().max_element() * 1e-6);
        let eps = scale * 1e-9;
        let d = tri.map(|p| {
            let d = n.dot(p - point);
            if d.abs() <= eps { 0.0 } else { d }
        });
        let on: Vec<usize> = (0..3).filter(|&i| d[i] == 0.0).collect();
        match on.len() {
            3 => {}
            2 => {
                let other = 3 - on[0] - on[1];
                if d[other] < 0.0 {
                    out.push([tri[on[0]], tri[on[1]]]);
                }
            }
            _ => {
                // Vertices on the plane and crossings of edges whose ends are on
                // opposite sides.
                let mut pts: Vec<DVec3> = on.iter().map(|&i| tri[i]).collect();
                for i in 0..3 {
                    let j = (i + 1) % 3;
                    if d[i] * d[j] < 0.0 {
                        let t = d[i] / (d[i] - d[j]);
                        pts.push(tri[i].lerp(tri[j], t));
                    }
                }
                if let [a, b] = pts[..] {
                    out.push([a, b]);
                }
            }
        }
    }
    out
}

/// Triangles filling the closed loops the plane cuts out of `triangles` (SketchUp's
/// section fill), all lying in the plane.
///
/// The cut segments of [`section_lines`] are welded at their ends and chained into
/// loops; chains that do not close (open, non-solid geometry) are dropped. Loops are
/// nested by containment: a loop inside an odd number of others is a hole of the
/// smallest loop containing it, so a cut tube gives a ring. SketchUp fills only solids
/// (unverified how it treats closed loops of non-solid geometry; here any closed loop
/// is filled).
pub fn section_fill(triangles: &[[DVec3; 3]], point: DVec3, normal: DVec3) -> Vec<[DVec3; 3]> {
    let Some(n) = normal.try_normalize() else {
        return Vec::new();
    };
    let segments = section_lines(triangles, point, n);
    if segments.len() < 3 {
        return Vec::new();
    }
    // Plane coordinates.
    let u = n.any_orthonormal_vector();
    let v = n.cross(u);
    let to_2d = |p: DVec3| glam::DVec2::new(u.dot(p - point), v.dot(p - point));
    let extent = segments
        .iter()
        .flatten()
        .map(|p| (*p - point).abs().max_element())
        .fold(0.0, f64::max)
        .max(1.0);
    let tol = extent * 1e-9;

    // Weld endpoints (a grid of `tol` cells, checking neighbours).
    let mut points: Vec<glam::DVec2> = Vec::new();
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    let mut weld = |p: glam::DVec2| -> usize {
        let cell = ((p.x / tol).floor() as i64, (p.y / tol).floor() as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ids) = grid.get(&(cell.0 + dx, cell.1 + dy)) {
                    for &i in ids {
                        if points[i].distance(p) <= tol {
                            return i;
                        }
                    }
                }
            }
        }
        points.push(p);
        grid.entry(cell).or_default().push(points.len() - 1);
        points.len() - 1
    };
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for [a, b] in &segments {
        let (a, b) = (weld(to_2d(*a)), weld(to_2d(*b)));
        if a != b && seen.insert((a.min(b), a.max(b))) {
            edges.push((a, b));
        }
    }
    let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (k, &(a, b)) in edges.iter().enumerate() {
        adjacent[a].push(k);
        adjacent[b].push(k);
    }

    // Walk closed loops.
    let mut used = vec![false; edges.len()];
    let mut loops: Vec<Vec<usize>> = Vec::new();
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (first, mut at) = edges[start];
        let mut chain = vec![first];
        let closed = loop {
            if at == first {
                break true;
            }
            chain.push(at);
            let Some(&k) = adjacent[at].iter().find(|&&k| !used[k]) else {
                break false;
            };
            used[k] = true;
            let (a, b) = edges[k];
            at = if a == at { b } else { a };
        };
        if closed && chain.len() >= 3 {
            loops.push(chain);
        }
    }

    let area = |l: &[usize]| -> f64 {
        (0..l.len())
            .map(|i| points[l[i]].perp_dot(points[l[(i + 1) % l.len()]]))
            .sum::<f64>()
            / 2.0
    };
    let min_area = tol * extent;
    loops.retain(|l| area(l).abs() > min_area);
    let inside = |p: glam::DVec2, l: &[usize]| -> bool {
        let mut c = false;
        for i in 0..l.len() {
            let (a, b) = (points[l[i]], points[l[(i + 1) % l.len()]]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                c = !c;
            }
        }
        c
    };
    // A point inside loop `i`, off its boundary: the midpoint of its first edge nudged
    // inwards, falling back to that midpoint.
    let probe = |l: &[usize]| -> glam::DVec2 {
        let (a, b) = (points[l[0]], points[l[1]]);
        let mid = (a + b) / 2.0;
        let inward = (b - a).perp().normalize_or_zero() * area(l).signum() * tol * 10.0;
        if inside(mid + inward, l) {
            mid + inward
        } else {
            mid
        }
    };
    let probes: Vec<glam::DVec2> = loops.iter().map(|l| probe(l)).collect();
    // Containing loops of each loop, and its nesting depth.
    let containers: Vec<Vec<usize>> = (0..loops.len())
        .map(|i| {
            (0..loops.len())
                .filter(|&j| j != i && area(&loops[j]).abs() > area(&loops[i]).abs())
                .filter(|&j| inside(probes[i], &loops[j]))
                .collect()
        })
        .collect();

    let mut out = Vec::new();
    for (i, outer) in loops.iter().enumerate() {
        if containers[i].len() % 2 == 1 {
            continue;
        }
        // Holes: loops one level deeper whose containers include this one.
        let holes = (0..loops.len()).filter(|&h| {
            containers[h].len() == containers[i].len() + 1 && containers[h].contains(&i)
        });
        let mut flat: Vec<f64> = Vec::new();
        let mut starts = Vec::new();
        let mut ids: Vec<usize> = Vec::new();
        for (k, l) in std::iter::once(outer)
            .chain(holes.map(|h| &loops[h]))
            .enumerate()
        {
            if k > 0 {
                starts.push(ids.len());
            }
            for &p in l {
                flat.extend([points[p].x, points[p].y]);
                ids.push(p);
            }
        }
        let Ok(tris) = earcutr::earcut(&flat, &starts, 2) else {
            continue;
        };
        let to_3d = |q: glam::DVec2| point + u * q.x + v * q.y;
        for t in tris.as_chunks::<3>().0 {
            out.push([0, 1, 2].map(|k| to_3d(points[ids[t[k]]])));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: DVec3, b: DVec3) -> bool {
        a.distance(b) < 1e-9
    }

    fn same_segment(s: [DVec3; 2], a: DVec3, b: DVec3) -> bool {
        (approx(s[0], a) && approx(s[1], b)) || (approx(s[0], b) && approx(s[1], a))
    }

    #[test]
    fn triangle_crossing_plane_gives_one_segment() {
        let tri = [
            DVec3::new(0.0, 0.0, -1.0),
            DVec3::new(2.0, 0.0, 1.0),
            DVec3::new(0.0, 2.0, 1.0),
        ];
        let lines = section_lines(&[tri], DVec3::ZERO, DVec3::Z);
        assert_eq!(lines.len(), 1);
        assert!(same_segment(
            lines[0],
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0)
        ));
    }

    #[test]
    fn triangles_on_one_side_or_in_plane_give_nothing() {
        let above = [
            DVec3::Z,
            DVec3::new(1.0, 0.0, 2.0),
            DVec3::new(0.0, 1.0, 3.0),
        ];
        let flat = [DVec3::ZERO, DVec3::X, DVec3::Y];
        // Touching the plane at a single vertex.
        let touch = [
            DVec3::ZERO,
            DVec3::new(1.0, 0.0, 1.0),
            DVec3::new(0.0, 1.0, 1.0),
        ];
        assert!(section_lines(&[above, flat, touch], DVec3::ZERO, DVec3::Z).is_empty());
    }

    #[test]
    fn vertex_on_plane_and_crossing_edge() {
        let tri = [
            DVec3::ZERO,
            DVec3::new(2.0, 0.0, -1.0),
            DVec3::new(2.0, 0.0, 1.0),
        ];
        let lines = section_lines(&[tri], DVec3::ZERO, DVec3::Z);
        assert_eq!(lines.len(), 1);
        assert!(same_segment(
            lines[0],
            DVec3::ZERO,
            DVec3::new(2.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn edge_in_plane_counted_once_for_a_fold() {
        let a = DVec3::ZERO;
        let b = DVec3::X;
        let below = [a, b, DVec3::new(0.5, 1.0, -1.0)];
        let above = [a, b, DVec3::new(0.5, 1.0, 1.0)];
        let lines = section_lines(&[below, above], DVec3::ZERO, DVec3::Z);
        assert_eq!(lines.len(), 1);
        assert!(same_segment(lines[0], a, b));
    }

    #[test]
    fn unit_cube_cut_through_middle() {
        // A 2x2x2 cube centred on the origin as 12 triangles, cut by x = 0.5 with the
        // normal along -X (keeping the +X side).
        let mut tris = Vec::new();
        for axis in 0..3 {
            for sign in [-1.0, 1.0] {
                let n = DVec3::AXES[axis] * sign;
                let u = DVec3::AXES[(axis + 1) % 3];
                let v = DVec3::AXES[(axis + 2) % 3];
                let c = [n - u - v, n + u - v, n + u + v, n - u + v];
                tris.push([c[0], c[1], c[2]]);
                tris.push([c[0], c[2], c[3]]);
            }
        }
        let lines = section_lines(&tris, DVec3::new(0.5, 0.0, 0.0), -DVec3::X);
        // Four side faces of two triangles each; each triangle is crossed.
        assert_eq!(lines.len(), 8);
        let total: f64 = lines.iter().map(|[a, b]| a.distance(*b)).sum();
        assert!((total - 8.0).abs() < 1e-9, "perimeter {total}");
        assert!(lines.iter().flatten().all(|p| (p.x - 0.5).abs() < 1e-12));
    }

    #[test]
    fn zero_normal_gives_nothing() {
        let tri = [DVec3::ZERO, DVec3::X, DVec3::Y];
        assert!(section_lines(&[tri], DVec3::ZERO, DVec3::ZERO).is_empty());
    }

    fn cube(min: DVec3, max: DVec3) -> Vec<[DVec3; 3]> {
        let mut tris = Vec::new();
        let c = (min + max) / 2.0;
        let h = (max - min) / 2.0;
        for axis in 0..3 {
            for sign in [-1.0, 1.0] {
                let n = DVec3::AXES[axis] * sign;
                let u = DVec3::AXES[(axis + 1) % 3];
                let v = DVec3::AXES[(axis + 2) % 3];
                let q = [n - u - v, n + u - v, n + u + v, n - u + v].map(|p| c + p * h);
                tris.push([q[0], q[1], q[2]]);
                tris.push([q[0], q[2], q[3]]);
            }
        }
        tris
    }

    fn tri_area(t: &[DVec3; 3]) -> f64 {
        (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0
    }

    #[test]
    fn fill_of_cut_cube_is_a_square() {
        let tris = cube(DVec3::splat(-1.0), DVec3::splat(1.0));
        let fill = section_fill(&tris, DVec3::new(0.25, 0.0, 0.0), DVec3::X);
        let area: f64 = fill.iter().map(tri_area).sum();
        assert!((area - 4.0).abs() < 1e-9, "area {area}");
        assert!(fill.iter().flatten().all(|p| (p.x - 0.25).abs() < 1e-12));
    }

    #[test]
    fn fill_has_holes_and_islands() {
        // A 4x4x4 box with a 2x2x2 box inside it (a hollow solid), plus a separate box.
        let mut tris = cube(DVec3::splat(-2.0), DVec3::splat(2.0));
        tris.extend(cube(DVec3::splat(-1.0), DVec3::splat(1.0)));
        tris.extend(cube(DVec3::new(5.0, -1.0, -1.0), DVec3::new(6.0, 1.0, 1.0)));
        let fill = section_fill(&tris, DVec3::ZERO, DVec3::Z);
        let area: f64 = fill.iter().map(tri_area).sum();
        // 16 - 4 (hole) + 2 (separate box).
        assert!((area - 14.0).abs() < 1e-9, "area {area}");
    }

    #[test]
    fn open_geometry_is_not_filled() {
        // A cube without its +Y side: the cut is an open chain.
        let mut tris = cube(DVec3::splat(-1.0), DVec3::splat(1.0));
        tris.drain(6..8);
        let fill = section_fill(&tris, DVec3::ZERO, DVec3::Z);
        assert!(fill.is_empty());
    }
}
