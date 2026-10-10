//! Linear BVH (Karras 2012, "Maximizing Parallelism in the Construction of BVHs,
//! Octrees, and k-d Trees").
//!
//! Triangles are sorted by the Morton code of their centroid; the hierarchy then follows
//! from the sorted codes, each internal node independently, which is what makes the build
//! parallel. This module is the CPU version; `gpu` runs the same steps as CUDA kernels and
//! produces the same nodes, bit for bit.
//!
//! Compared with [`mt_kernel::bvh::Bvh`] (median splits, up to 4 triangles per leaf) the
//! tree is somewhat worse for queries but much cheaper to build. Both return the nearest
//! hit; when several triangles are hit at exactly the same distance, this one returns the
//! lowest triangle index.

use glam::DVec3;
use mt_kernel::bvh::{Hit, ray_triangle};

use crate::Ray;

/// Marks a child reference as a leaf; the low bits are then the triangle index.
pub const LEAF: u32 = 1 << 31;

/// Morton codes use 21 bits per axis.
const MORTON_CELLS: f64 = 2_097_152.0;
const MORTON_MAX: u32 = (1 << 21) - 1;

/// An internal node. Children are internal node indices, or `LEAF | triangle`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub left: u32,
    pub right: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lbvh {
    triangles: Vec<[DVec3; 3]>,
    /// `n - 1` internal nodes for `n >= 2` triangles, root first.
    nodes: Vec<Node>,
    /// `0`, or `LEAF | 0` for a single triangle.
    root: u32,
}

// The min/max below are written as in `glam` (`a < b ? a : b`), and the GPU kernels the
// same way, so that both give identical results.
pub(crate) fn gmin(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

pub(crate) fn gmax(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

/// Sum of the corners (three times the centroid; the scale does not matter for sorting).
pub(crate) fn centroid([a, b, c]: &[DVec3; 3]) -> DVec3 {
    a + b + c
}

/// Lower corner and per-axis scale mapping centroids to Morton cells.
pub(crate) fn morton_params(min: DVec3, max: DVec3) -> ([f64; 3], [f64; 3]) {
    let scale = |lo: f64, hi: f64| {
        let extent = hi - lo;
        if extent > 0.0 {
            MORTON_CELLS / extent
        } else {
            0.0
        }
    };
    (
        min.to_array(),
        [
            scale(min.x, max.x),
            scale(min.y, max.y),
            scale(min.z, max.z),
        ],
    )
}

fn quantize(x: f64, lo: f64, scale: f64) -> u32 {
    let t = (x - lo) * scale;
    if t.is_nan() || t <= 0.0 {
        0
    } else if t >= f64::from(MORTON_MAX) {
        MORTON_MAX
    } else {
        t as u32
    }
}

/// Spreads the low 21 bits of `v` to every third bit.
fn spread(v: u32) -> u64 {
    let mut x = u64::from(v) & 0x1f_ffff;
    x = (x | x << 32) & 0x001f_0000_0000_ffff;
    x = (x | x << 16) & 0x001f_0000_ff00_00ff;
    x = (x | x << 8) & 0x100f_00f0_0f00_f00f;
    x = (x | x << 4) & 0x10c3_0c30_c30c_30c3;
    x = (x | x << 2) & 0x1249_2492_4924_9249;
    x
}

pub(crate) fn morton(c: DVec3, lo: &[f64; 3], scale: &[f64; 3]) -> u64 {
    spread(quantize(c.x, lo[0], scale[0])) << 2
        | spread(quantize(c.y, lo[1], scale[1])) << 1
        | spread(quantize(c.z, lo[2], scale[2]))
}

/// Length of the common prefix of the sorted keys at `i` and `j`, with the position as
/// tie-breaker for equal keys; -1 outside the array.
fn delta(keys: &[u64], i: i64, j: i64) -> i64 {
    if j < 0 || j >= keys.len() as i64 {
        return -1;
    }
    let (a, b) = (keys[i as usize], keys[j as usize]);
    if a == b {
        64 + i64::from((i as u32 ^ j as u32).leading_zeros())
    } else {
        i64::from((a ^ b).leading_zeros())
    }
}

/// Children of internal node `i`, from keys sorted by `(key, triangle)`.
fn children(keys: &[u64], order: &[u32], i: usize) -> (u32, u32) {
    let i = i as i64;
    let d = (delta(keys, i, i + 1) - delta(keys, i, i - 1)).signum();
    let d_min = delta(keys, i, i - d);
    let mut l_max = 2;
    while delta(keys, i, i + l_max * d) > d_min {
        l_max *= 2;
    }
    let mut l = 0;
    let mut t = l_max / 2;
    while t >= 1 {
        if delta(keys, i, i + (l + t) * d) > d_min {
            l += t;
        }
        t /= 2;
    }
    let j = i + l * d;
    let d_node = delta(keys, i, j);
    let mut s = 0;
    let mut t = l;
    loop {
        t = (t + 1) / 2;
        if delta(keys, i, i + (s + t) * d) > d_node {
            s += t;
        }
        if t <= 1 {
            break;
        }
    }
    let gamma = i + s * d + d.min(0);
    let child = |pos: i64, is_leaf: bool| {
        if is_leaf {
            LEAF | order[pos as usize]
        } else {
            pos as u32
        }
    };
    (
        child(gamma, i.min(j) == gamma),
        child(gamma + 1, i.max(j) == gamma + 1),
    )
}

pub(crate) fn triangle_bounds([a, b, c]: &[DVec3; 3]) -> ([f64; 3], [f64; 3]) {
    let mut lo = a.to_array();
    let mut hi = lo;
    for p in [b, c] {
        for k in 0..3 {
            lo[k] = gmin(lo[k], p[k]);
            hi[k] = gmax(hi[k], p[k]);
        }
    }
    (lo, hi)
}

impl Lbvh {
    pub fn build(triangles: Vec<[DVec3; 3]>) -> Self {
        let n = triangles.len();
        assert!(n < LEAF as usize, "too many triangles for a BVH");
        if n < 2 {
            return Self {
                triangles,
                nodes: Vec::new(),
                root: LEAF,
            };
        }
        let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for t in &triangles {
            let c = centroid(t);
            lo = lo.min(c);
            hi = hi.max(c);
        }
        let (lo, scale) = morton_params(lo, hi);
        let mut sorted: Vec<(u64, u32)> = triangles
            .iter()
            .enumerate()
            .map(|(i, t)| (morton(centroid(t), &lo, &scale), i as u32))
            .collect();
        sorted.sort_unstable();
        let (keys, order): (Vec<u64>, Vec<u32>) = sorted.into_iter().unzip();

        let mut nodes: Vec<Node> = (0..n - 1)
            .map(|i| {
                let (left, right) = children(&keys, &order, i);
                Node {
                    min: [0.0; 3],
                    max: [0.0; 3],
                    left,
                    right,
                }
            })
            .collect();
        fill_bounds(&triangles, &mut nodes, 0);
        Self {
            triangles,
            nodes,
            root: 0,
        }
    }

    #[cfg_attr(not(feature = "cuda"), allow(dead_code))]
    pub(crate) fn from_parts(triangles: Vec<[DVec3; 3]>, nodes: Vec<Node>, root: u32) -> Self {
        Self {
            triangles,
            nodes,
            root,
        }
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

    pub fn triangles(&self) -> &[[DVec3; 3]] {
        &self.triangles
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn root(&self) -> u32 {
        self.root
    }

    /// Total bounds, or `None` when empty.
    pub fn bounds(&self) -> Option<(DVec3, DVec3)> {
        let (lo, hi) = match (self.nodes.first(), self.triangles.first()) {
            (Some(n), _) => (n.min, n.max),
            (None, Some(t)) => triangle_bounds(t),
            (None, None) => return None,
        };
        Some((DVec3::from(lo), DVec3::from(hi)))
    }

    /// Nearest triangle hit by the ray (`dir` need not be unit; `t` is in units of `dir`).
    /// Of triangles hit at exactly the same distance, the lowest index wins.
    pub fn ray(&self, origin: DVec3, dir: DVec3) -> Option<Hit> {
        if self.triangles.is_empty() {
            return None;
        }
        let o = origin.to_array();
        let inv = (DVec3::ONE / dir).to_array();
        let (mut best_t, mut best_i) = (f64::INFINITY, u32::MAX);
        let mut stack = vec![self.root];
        while let Some(n) = stack.pop() {
            if n & LEAF != 0 {
                let i = n & !LEAF;
                if let Some(t) = ray_triangle(origin, dir, self.triangles[i as usize])
                    && (t < best_t || (t == best_t && i < best_i))
                {
                    (best_t, best_i) = (t, i);
                }
                continue;
            }
            let node = &self.nodes[n as usize];
            if slab(node, &o, &inv, best_t) {
                stack.push(node.right);
                stack.push(node.left);
            }
        }
        (best_i != u32::MAX).then_some(Hit {
            t: best_t,
            triangle: best_i as usize,
        })
    }

    /// [`Lbvh::ray`] for each ray.
    pub fn rays(&self, rays: &[Ray]) -> Vec<Option<Hit>> {
        rays.iter().map(|&(o, d)| self.ray(o, d)).collect()
    }
}

/// Whether the ray enters the node's box before `t_max`.
fn slab(n: &Node, o: &[f64; 3], inv: &[f64; 3], t_max: f64) -> bool {
    let mut near = [0.0; 3];
    let mut far = [0.0; 3];
    for k in 0..3 {
        let t0 = (n.min[k] - o[k]) * inv[k];
        let t1 = (n.max[k] - o[k]) * inv[k];
        near[k] = gmin(t0, t1);
        far[k] = gmax(t0, t1);
    }
    let near = gmax(near[0], gmax(near[1], near[2])).max(0.0);
    let far = gmin(far[0], gmin(far[1], far[2])).min(t_max);
    near <= far
}

fn child_bounds(triangles: &[[DVec3; 3]], nodes: &mut [Node], c: u32) -> ([f64; 3], [f64; 3]) {
    if c & LEAF != 0 {
        triangle_bounds(&triangles[(c & !LEAF) as usize])
    } else {
        fill_bounds(triangles, nodes, c as usize);
        (nodes[c as usize].min, nodes[c as usize].max)
    }
}

fn fill_bounds(triangles: &[[DVec3; 3]], nodes: &mut [Node], i: usize) {
    let (l_lo, l_hi) = child_bounds(triangles, nodes, nodes[i].left);
    let (r_lo, r_hi) = child_bounds(triangles, nodes, nodes[i].right);
    for k in 0..3 {
        nodes[i].min[k] = gmin(l_lo[k], r_lo[k]);
        nodes[i].max[k] = gmax(l_hi[k], r_hi[k]);
    }
}

impl mt_kernel::bvh::RayIndex for Lbvh {
    fn ray(&self, origin: DVec3, dir: DVec3) -> Option<Hit> {
        Lbvh::ray(self, origin, dir)
    }

    fn triangle(&self, i: usize) -> [DVec3; 3] {
        Lbvh::triangle(self, i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_triangles(n: usize, seed: u64) -> Vec<[DVec3; 3]> {
        let mut s = seed;
        let mut rnd = move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64 * 20.0 - 10.0
        };
        (0..n)
            .map(|_| {
                let c = DVec3::new(rnd(), rnd(), rnd());
                [
                    c,
                    c + DVec3::new(rnd(), rnd(), rnd()) / 5.0,
                    c + DVec3::new(rnd(), rnd(), rnd()) / 5.0,
                ]
            })
            .collect()
    }

    fn brute(tris: &[[DVec3; 3]], o: DVec3, d: DVec3) -> Option<Hit> {
        tris.iter()
            .enumerate()
            .filter_map(|(i, t)| ray_triangle(o, d, *t).map(|t| (t, i)))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(t, triangle)| Hit { t, triangle })
    }

    #[test]
    fn spread_puts_bits_three_apart() {
        assert_eq!(spread(0b111), 0b1001001);
        assert_eq!(spread(MORTON_MAX), 0x1249_2492_4924_9249);
    }

    #[test]
    fn every_triangle_is_a_leaf_once() {
        for n in [2, 3, 7, 100, 1000] {
            let bvh = Lbvh::build(random_triangles(n, n as u64));
            let mut seen = vec![0; n];
            let mut stack = vec![bvh.root];
            while let Some(c) = stack.pop() {
                if c & LEAF != 0 {
                    seen[(c & !LEAF) as usize] += 1;
                } else {
                    let node = bvh.nodes[c as usize];
                    stack.extend([node.left, node.right]);
                }
            }
            assert!(seen.iter().all(|&s| s == 1), "n = {n}");
        }
    }

    #[test]
    fn duplicate_centroids_still_build() {
        let t = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let bvh = Lbvh::build(vec![t; 50]);
        assert_eq!(bvh.nodes.len(), 49);
        // All hit at the same distance: the lowest index wins.
        let hit = bvh.ray(DVec3::new(0.2, 0.2, 1.0), DVec3::NEG_Z).unwrap();
        assert_eq!(hit.triangle, 0);
    }

    #[test]
    fn matches_brute_force() {
        let tris = random_triangles(2000, 7);
        let bvh = Lbvh::build(tris.clone());
        let mut s = 99u64;
        let mut rnd = move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64 * 20.0 - 10.0
        };
        let mut hits = 0;
        for _ in 0..500 {
            let o = DVec3::new(rnd(), rnd(), rnd()) * 2.0;
            let d = DVec3::new(rnd(), rnd(), rnd()) - o / 2.0;
            let got = bvh.ray(o, d);
            assert_eq!(got, brute(&tris, o, d));
            hits += usize::from(got.is_some());
        }
        assert!(hits > 50, "{hits}");
    }

    #[test]
    fn tiny_inputs() {
        assert!(Lbvh::build(Vec::new()).ray(DVec3::ZERO, DVec3::X).is_none());
        let one = Lbvh::build(vec![[DVec3::ZERO, DVec3::X, DVec3::Y]]);
        let hit = one.ray(DVec3::new(0.1, 0.1, 1.0), DVec3::NEG_Z).unwrap();
        assert_eq!((hit.t, hit.triangle), (1.0, 0));
        assert_eq!(one.bounds(), Some((DVec3::ZERO, DVec3::new(1.0, 1.0, 0.0))));
    }
}
