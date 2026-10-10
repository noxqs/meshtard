//! Section planes: where a cutting plane crosses the model.
//!
//! The viewport clips away everything on the side the plane's normal points to (see
//! [`crate::Viewport::set_section`]); [`section_lines`] gives the cut outline, which
//! SketchUp draws as thick black "section cut" edges. SketchUp's optional section fill
//! (caps) is not done here.

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
}
