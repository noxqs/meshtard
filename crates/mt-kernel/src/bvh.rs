//! Bounding volume hierarchy over triangles, for picking and (later) inference.
//!
//! Built on the CPU with median splits. The build is the part a CUDA version would
//! replace for large models (PLAN.md); the query API stays the same.

use glam::DVec3;

#[derive(Debug, Clone, Copy)]
struct Aabb {
    min: DVec3,
    max: DVec3,
}

impl Aabb {
    const EMPTY: Self = Self {
        min: DVec3::splat(f64::INFINITY),
        max: DVec3::splat(f64::NEG_INFINITY),
    };

    fn grow(&mut self, p: DVec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    /// Entry distance of the ray, if it hits before `t_max`.
    fn hit(&self, origin: DVec3, inv_dir: DVec3, t_max: f64) -> Option<f64> {
        let t0 = (self.min - origin) * inv_dir;
        let t1 = (self.max - origin) * inv_dir;
        let near = t0.min(t1).max_element().max(0.0);
        let far = t0.max(t1).min_element().min(t_max);
        (near <= far).then_some(near)
    }
}

#[derive(Debug, Clone)]
enum Node {
    Leaf {
        bounds: Aabb,
        first: usize,
        count: usize,
    },
    Inner {
        bounds: Aabb,
        left: usize,
        right: usize,
    },
}

impl Node {
    fn bounds(&self) -> Aabb {
        match self {
            Node::Leaf { bounds, .. } | Node::Inner { bounds, .. } => *bounds,
        }
    }
}

/// A ray hit: distance along the (unit) ray direction and triangle index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub t: f64,
    pub triangle: usize,
}

/// Nearest-hit ray queries over a triangle set: [`Bvh`] here, or a GPU-built index
/// (`mt-cuda`'s `Lbvh`) for large models. Picking and inference take this.
pub trait RayIndex: Send + Sync {
    /// Nearest triangle hit by the ray.
    fn ray(&self, origin: DVec3, dir: DVec3) -> Option<Hit>;
    fn triangle(&self, i: usize) -> [DVec3; 3];
}

impl RayIndex for Bvh {
    fn ray(&self, origin: DVec3, dir: DVec3) -> Option<Hit> {
        Bvh::ray(self, origin, dir)
    }

    fn triangle(&self, i: usize) -> [DVec3; 3] {
        Bvh::triangle(self, i)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Bvh {
    triangles: Vec<[DVec3; 3]>,
    /// Triangle indices, ordered so each leaf owns a contiguous range.
    order: Vec<usize>,
    nodes: Vec<Node>,
}

const LEAF_SIZE: usize = 4;

impl Bvh {
    pub fn build(triangles: Vec<[DVec3; 3]>) -> Self {
        let mut bvh = Self {
            order: (0..triangles.len()).collect(),
            triangles,
            nodes: Vec::new(),
        };
        if !bvh.triangles.is_empty() {
            let n = bvh.triangles.len();
            bvh.build_node(0, n);
        }
        bvh
    }

    pub fn len(&self) -> usize {
        self.triangles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    pub fn triangle(&self, i: usize) -> [DVec3; 3] {
        self.triangles[i]
    }

    fn bounds_of(&self, range: &[usize]) -> Aabb {
        let mut b = Aabb::EMPTY;
        for &i in range {
            for p in self.triangles[i] {
                b.grow(p);
            }
        }
        b
    }

    /// Builds the node for `order[first..first + count]` and returns its index.
    fn build_node(&mut self, first: usize, count: usize) -> usize {
        let bounds = self.bounds_of(&self.order[first..first + count]);
        let index = self.nodes.len();
        if count <= LEAF_SIZE {
            self.nodes.push(Node::Leaf {
                bounds,
                first,
                count,
            });
            return index;
        }
        // Split at the median centroid along the widest axis.
        let extent = bounds.max - bounds.min;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let tris = &self.triangles;
        let centroid = |i: usize| (tris[i][0] + tris[i][1] + tris[i][2])[axis];
        let mid = count / 2;
        self.order[first..first + count]
            .select_nth_unstable_by(mid, |&a, &b| centroid(a).total_cmp(&centroid(b)));
        self.nodes.push(Node::Leaf {
            bounds,
            first,
            count,
        });
        let left = self.build_node(first, mid);
        let right = self.build_node(first + mid, count - mid);
        self.nodes[index] = Node::Inner {
            bounds,
            left,
            right,
        };
        index
    }

    /// Nearest triangle hit by the ray (`dir` need not be unit; `t` is in units of `dir`).
    pub fn ray(&self, origin: DVec3, dir: DVec3) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = DVec3::ONE / dir;
        let mut best: Option<Hit> = None;
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let t_max = best.map_or(f64::INFINITY, |h| h.t);
            if self.nodes[n].bounds().hit(origin, inv, t_max).is_none() {
                continue;
            }
            match self.nodes[n] {
                Node::Leaf { first, count, .. } => {
                    for &i in &self.order[first..first + count] {
                        if let Some(t) = ray_triangle(origin, dir, self.triangles[i])
                            && t < t_max.min(best.map_or(f64::INFINITY, |h| h.t))
                        {
                            best = Some(Hit { t, triangle: i });
                        }
                    }
                }
                Node::Inner { left, right, .. } => {
                    stack.push(left);
                    stack.push(right);
                }
            }
        }
        best
    }

    /// Total bounds, or `None` when empty.
    pub fn bounds(&self) -> Option<(DVec3, DVec3)> {
        self.nodes.first().map(|n| {
            let b = n.bounds();
            (b.min, b.max)
        })
    }
}

/// Möller-Trumbore; two-sided. Distance in units of `dir`.
pub fn ray_triangle(origin: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-14 * e1.length() * e2.length() * dir.length() {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_at(z: f64) -> [DVec3; 3] {
        [
            DVec3::new(-1.0, -1.0, z),
            DVec3::new(1.0, -1.0, z),
            DVec3::new(0.0, 1.0, z),
        ]
    }

    #[test]
    fn nearest_hit_wins() {
        let bvh = Bvh::build((0..50).map(|i| tri_at(f64::from(i))).collect());
        let hit = bvh.ray(DVec3::new(0.0, 0.0, 100.0), DVec3::NEG_Z).unwrap();
        assert_eq!(hit.triangle, 49);
        assert!((hit.t - 51.0).abs() < 1e-12);
        assert!(bvh.ray(DVec3::new(5.0, 0.0, 100.0), DVec3::NEG_Z).is_none());
    }

    #[test]
    fn matches_brute_force() {
        // Pseudo-random triangles, compared against testing every triangle.
        let mut s = 1u64;
        let mut rnd = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64 * 20.0 - 10.0
        };
        let tris: Vec<[DVec3; 3]> = (0..500)
            .map(|_| {
                let c = DVec3::new(rnd(), rnd(), rnd());
                [
                    c,
                    c + DVec3::new(rnd(), rnd(), rnd()) / 5.0,
                    c + DVec3::new(rnd(), rnd(), rnd()) / 5.0,
                ]
            })
            .collect();
        let bvh = Bvh::build(tris.clone());
        for _ in 0..200 {
            let o = DVec3::new(rnd(), rnd(), rnd()) * 2.0;
            let d = DVec3::new(rnd(), rnd(), rnd());
            let brute = tris
                .iter()
                .enumerate()
                .filter_map(|(i, t)| ray_triangle(o, d, *t).map(|t| (t, i)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let got = bvh.ray(o, d).map(|h| (h.t, h.triangle));
            assert_eq!(got, brute);
        }
    }
}
