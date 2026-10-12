//! Finding the segments and points that may lie near the cursor on screen without
//! projecting every one: a bounding volume hierarchy over their boxes, culled by the
//! narrow pyramid of view rays through a square of pixels around the cursor.
//!
//! The answer is a superset (in index order) of the items whose projection can be
//! within the radius, so callers run their exact pixel test over it and pick the same
//! item a scan of everything would.

use glam::{DVec2, DVec3};
use mt_render::Camera;

/// Items per leaf.
const LEAF: usize = 8;
/// Extra pixels around the radius, so rounding in the pyramid planes never drops an
/// item the exact test keeps.
const SLACK_PX: f64 = 2.0;

#[derive(Debug, Clone)]
struct Node {
    min: DVec3,
    max: DVec3,
    /// Leaf: `order[first..first + count]`; inner (count 0): children at `first`
    /// and `first + 1`.
    first: u32,
    count: u32,
}

/// Bounding volume hierarchy over axis-aligned boxes of segments or points.
#[derive(Debug, Clone, Default)]
pub struct BoxIndex {
    len: usize,
    nodes: Vec<Node>,
    order: Vec<u32>,
}

impl BoxIndex {
    /// An index over segments `[a, b]`.
    pub fn segments(segments: impl ExactSizeIterator<Item = [DVec3; 2]>) -> Self {
        let boxes: Vec<(DVec3, DVec3)> = segments.map(|[a, b]| (a.min(b), a.max(b))).collect();
        Self::build(boxes)
    }

    /// An index over points.
    pub fn points(points: impl ExactSizeIterator<Item = DVec3>) -> Self {
        Self::build(points.map(|p| (p, p)).collect())
    }

    fn build(boxes: Vec<(DVec3, DVec3)>) -> Self {
        let len = boxes.len();
        let mut index = Self {
            len,
            nodes: Vec::new(),
            order: (0..len as u32).collect(),
        };
        if len > 0 {
            index.nodes.push(Node {
                min: DVec3::ZERO,
                max: DVec3::ZERO,
                first: 0,
                count: 0,
            });
            let centers: Vec<DVec3> = boxes.iter().map(|(a, b)| (*a + *b) * 0.5).collect();
            index.split(0, 0, len, &boxes, &centers);
        }
        index
    }

    /// Fills node `node` with `order[first..first + count]`.
    fn split(
        &mut self,
        node: usize,
        first: usize,
        count: usize,
        boxes: &[(DVec3, DVec3)],
        centers: &[DVec3],
    ) {
        let (mut min, mut max) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        let (mut cmin, mut cmax) = (min, max);
        for &i in &self.order[first..first + count] {
            let (a, b) = boxes[i as usize];
            // NaN coordinates leave the box unchanged (`min` / `max` take the number).
            min = min.min(a);
            max = max.max(b);
            let c = centers[i as usize];
            cmin = cmin.min(c);
            cmax = cmax.max(c);
        }
        if count <= LEAF {
            self.nodes[node] = Node {
                min,
                max,
                first: first as u32,
                count: count as u32,
            };
            return;
        }
        let extent = cmax - cmin;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let mid = count / 2;
        self.order[first..first + count].select_nth_unstable_by(mid, |&a, &b| {
            centers[a as usize][axis].total_cmp(&centers[b as usize][axis])
        });
        let children = self.nodes.len();
        let empty = Node {
            min: DVec3::ZERO,
            max: DVec3::ZERO,
            first: 0,
            count: 0,
        };
        self.nodes.push(empty.clone());
        self.nodes.push(empty);
        self.nodes[node] = Node {
            min,
            max,
            first: children as u32,
            count: 0,
        };
        self.split(children, first, mid, boxes, centers);
        self.split(children + 1, first + mid, count - mid, boxes, centers);
    }

    /// Number of items indexed.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Indices (ascending) of the items that may come within `radius_px` pixels of
    /// `cursor` on screen; every item that does is included. `None` when the view
    /// gives no usable pyramid (the caller then tests every item).
    pub fn near(
        &self,
        camera: &Camera,
        viewport: DVec2,
        cursor: DVec2,
        radius_px: f64,
    ) -> Option<Vec<usize>> {
        let pyramid = Pyramid::new(camera, viewport, cursor, radius_px + SLACK_PX)?;
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return Some(out);
        }
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            // Boxes with NaN or infinite corners are never culled.
            let finite = node.min.is_finite() && node.max.is_finite();
            if finite && pyramid.outside(node.min, node.max) {
                continue;
            }
            if node.count > 0 {
                let (f, c) = (node.first as usize, node.count as usize);
                out.extend(self.order[f..f + c].iter().map(|&i| i as usize));
            } else {
                stack.push(node.first as usize);
                stack.push(node.first as usize + 1);
            }
        }
        out.sort_unstable();
        Some(out)
    }
}

/// The four side planes of the view rays through a square of pixels.
struct Pyramid {
    /// A point on the plane and the normal pointing inside.
    planes: [(DVec3, DVec3); 4],
}

impl Pyramid {
    fn new(camera: &Camera, viewport: DVec2, cursor: DVec2, half: f64) -> Option<Self> {
        let corners = [
            DVec2::new(-half, -half),
            DVec2::new(half, -half),
            DVec2::new(half, half),
            DVec2::new(-half, half),
        ]
        .map(|d| camera.ray(cursor + d, viewport));
        if corners
            .iter()
            .any(|r| !(r.origin.is_finite() && r.dir.is_finite()))
        {
            return None;
        }
        // Points along the corner rays; their middle is inside every plane.
        let far = corners
            .iter()
            .map(|r| r.origin.distance(corners[0].origin))
            .fold(1.0f64, f64::max);
        let inside = corners
            .iter()
            .map(|r| r.origin + r.dir * far)
            .fold(DVec3::ZERO, |s, p| s + p)
            / 4.0;
        let mut planes = [(DVec3::ZERO, DVec3::ZERO); 4];
        for k in 0..4 {
            let (r, s) = (&corners[k], &corners[(k + 1) % 4]);
            // The plane through both rays (through the eye in perspective, along
            // the view direction in parallel projection).
            let n = r.dir.cross(s.origin + s.dir - r.origin);
            let len = n.length();
            if len.is_nan() || len <= 1e-12 {
                return None;
            }
            let n = n / len;
            let side = n.dot(inside - r.origin);
            if side.is_nan() || side.abs() <= 1e-12 * far {
                return None;
            }
            planes[k] = (r.origin, if side > 0.0 { n } else { -n });
        }
        Some(Self { planes })
    }

    /// The box lies wholly outside one of the planes.
    fn outside(&self, min: DVec3, max: DVec3) -> bool {
        self.planes.iter().any(|&(o, n)| {
            // The corner furthest along the normal.
            let c = DVec3::new(
                if n.x >= 0.0 { max.x } else { min.x },
                if n.y >= 0.0 { max.y } else { min.y },
                if n.z >= 0.0 { max.z } else { min.z },
            );
            n.dot(c - o) < 0.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index keeps every segment the exact pixel test keeps, for perspective and
    /// parallel views, at many cursor positions.
    #[test]
    fn near_keeps_every_segment_within_the_radius() {
        let mut segs = Vec::new();
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / (1u64 << 53) as f64) * 200.0 - 100.0
        };
        for _ in 0..3000 {
            let a = DVec3::new(rnd(), rnd(), rnd());
            let b = a + DVec3::new(rnd(), rnd(), rnd()) * 0.1;
            segs.push([a, b]);
        }
        let index = BoxIndex::segments(segs.iter().copied());
        let points = BoxIndex::points(segs.iter().map(|s| s[0]));
        let viewport = DVec2::new(800.0, 600.0);
        for perspective in [true, false] {
            let camera = Camera {
                eye: DVec3::new(150.0, -220.0, 130.0),
                target: DVec3::ZERO,
                fov_deg: 35.0,
                perspective,
                two_point: false,
            };
            let m = camera.view_proj(viewport.x / viewport.y);
            for k in 0..64 {
                let cursor = DVec2::new((k % 8) as f64 * 100.0 + 13.0, (k / 8) as f64 * 75.0 + 7.0);
                let radius = 10.0;
                let near = index.near(&camera, viewport, cursor, radius).unwrap();
                let near_points = points.near(&camera, viewport, cursor, radius).unwrap();
                for (i, [a, b]) in segs.iter().enumerate() {
                    let (Some(pa), Some(pb)) = (
                        Camera::project_with(&m, *a, viewport),
                        Camera::project_with(&m, *b, viewport),
                    ) else {
                        continue;
                    };
                    let ab = pb - pa;
                    let t =
                        ((cursor - pa).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                    if cursor.distance(pa + ab * t) <= radius {
                        assert!(near.binary_search(&i).is_ok(), "segment {i} dropped");
                    }
                    if pa.distance(cursor) <= radius {
                        assert!(near_points.binary_search(&i).is_ok(), "point {i} dropped");
                    }
                }
                assert!(near.len() < segs.len() / 4, "culls most segments");
            }
        }
    }
}
