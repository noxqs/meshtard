//! Spatial index over the vertices, edges and faces of an [`Entities`](crate::Entities),
//! so kernel lookups (the vertex at a point, edges near a line, faces containing a point,
//! everything lying in a plane) do not scan the whole model.
//!
//! A hierarchical loose grid: level `l` has cubic cells of size `cell * 2^l`. An item
//! (its bounding box) goes into the level whose cell size is at least its extent, in the
//! one cell holding its box center, so its box lies within that cell grown by half a cell
//! on every side. A query visits, at every occupied level, the cells whose grown box meets
//! the query, or (when that would visit more cells than the level has) every occupied cell
//! of the level, so a query never costs more than a scan of the occupied cells.
//!
//! Queries return candidates: a superset of the matching items, in key order (the order
//! a scan of the slot map visits them), so callers that pick the first match behave as
//! with a scan. Callers apply their exact tests to the candidates.
//!
//! The index lives in `Entities` and is kept up to date by its editing methods
//! (`add_vertex`, `add_raw_edge`, `add_raw_face`, `remove_*`, `split_edge`,
//! `set_vertex_positions`). It is built lazily (`Entities::ensure_index`) and detects
//! additions or removals that bypassed those methods by comparing item counts; it cannot
//! detect a vertex moved by writing `Vertex::pos` directly, so code doing that must call
//! `Entities::set_vertex_positions` or `Entities::invalidate_index`. Cloning `Entities`
//! does not copy the index (undo snapshots stay small); the clone rebuilds it on first use.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

use glam::DVec3;
use slotmap::{Key, SecondaryMap};

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub const EMPTY: Self = Self {
        min: DVec3::splat(f64::INFINITY),
        max: DVec3::splat(f64::NEG_INFINITY),
    };

    pub fn point(p: DVec3) -> Self {
        Self { min: p, max: p }
    }

    pub fn segment(a: DVec3, b: DVec3) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    pub fn of(points: impl IntoIterator<Item = DVec3>) -> Self {
        let mut b = Self::EMPTY;
        for p in points {
            b.grow(p);
        }
        b
    }

    pub fn grow(&mut self, p: DVec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(&self, o: &Self) -> Self {
        Self {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }

    /// Grown by `m` on every side.
    pub fn expand(&self, m: f64) -> Self {
        Self {
            min: self.min - DVec3::splat(m),
            max: self.max + DVec3::splat(m),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    pub fn overlaps(&self, o: &Self) -> bool {
        self.min.cmple(o.max).all() && o.min.cmple(self.max).all()
    }

    pub fn center(&self) -> DVec3 {
        (self.min + self.max) / 2.0
    }

    pub fn extent(&self) -> f64 {
        (self.max - self.min).max_element()
    }

    /// [`Aabb::meets_plane`] with some slack for rounding: true whenever it is for any
    /// box inside this one.
    fn may_meet_plane(&self, normal: DVec3, d: f64, tol: f64) -> bool {
        let c = self.center();
        let h = (self.max - self.min) / 2.0;
        let r = h.dot(normal.abs());
        let along = normal.dot(c);
        (along - d).abs() <= r + tol + 1e-9 * (r + along.abs() + d.abs() + tol)
    }

    /// Whether the box comes within `tol` of the plane `normal . p = d`.
    pub fn meets_plane(&self, normal: DVec3, d: f64, tol: f64) -> bool {
        let c = self.center();
        let h = (self.max - self.min) / 2.0;
        let r = h.dot(normal.abs());
        (normal.dot(c) - d).abs() <= r + tol
    }
}

/// FxHash: a fast hasher for integer cell coordinates.
#[derive(Default)]
pub struct FxHasher(u64);

/// A hash map keyed by integer cell coordinates (or other small integers).
pub type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// A hash set of small integers.
pub type FxSet<K> = std::collections::HashSet<K, BuildHasherDefault<FxHasher>>;

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Whole words at a time (`[i64; 3]` keys hash as their bytes).
        let (words, rest) = bytes.as_chunks::<8>();
        for w in words {
            self.write_u64(u64::from_le_bytes(*w));
        }
        for &b in rest {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_i64(&mut self, x: i64) {
        self.write_u64(x as u64);
    }

    fn write_usize(&mut self, x: usize) {
        self.write_u64(x as u64);
    }
}

type Cell = [i64; 3];

/// The items of one occupied cell, the box of their boxes, and the cell's block (and
/// where it is in the block's list).
///
/// Queries skip a cell whose items' box misses them: none of its items could match
/// (each item's box lies in it), so the answers are the same as testing every item, only
/// faster where cells are much bigger than what they hold (a flat model's plane queries).
#[derive(Debug, Clone)]
struct Bucket<K> {
    cell: Cell,
    items: Vec<K>,
    bounds: Aabb,
    block: u32,
    block_at: u32,
}

/// Cells of a level grouped `2^BLOCK_SHIFT` to a side, so a plane query skips whole
/// groups the plane misses instead of testing every occupied cell (a flat model's cells
/// all lie in one or two slabs, which a slightly tilted plane crosses everywhere).
const BLOCK_SHIFT: u32 = 3;

/// The occupied cells of one block, a box holding all of their items' boxes (grown on
/// insertion, never shrunk: still a bound after removals), and where the block is in
/// each of its level's slabs (so emptying a block does not search a slab that may hold
/// most of the level's blocks).
#[derive(Debug, Clone)]
struct Block {
    at: Cell,
    buckets: Vec<u32>,
    bounds: Aabb,
    slab_at: [u32; 3],
}

fn block_of(c: &Cell) -> Cell {
    c.map(|x| x >> BLOCK_SHIFT)
}

/// Deepest level: cells of `cell * 2^MAX_LEVEL` hold anything.
const MAX_LEVEL: usize = 62;

/// One level of a layer: occupied cells, their blocks, and the blocks of each slab
/// (blocks with the same coordinate along an axis), so a plane query visits only the
/// slabs the plane crosses and, in them, the blocks and cells whose items' boxes it
/// meets. The cells are kept side by side in `buckets` (the map and blocks point into
/// it), so visiting many of them reads one array.
#[derive(Debug, Clone)]
struct Level<K: Key> {
    /// Each occupied cell's index in `buckets`.
    cells: FxMap<Cell, u32>,
    buckets: Vec<Bucket<K>>,
    /// Each occupied block's index in `blocks`.
    block_map: FxMap<Cell, u32>,
    blocks: Vec<Block>,
    slabs: [BTreeMap<i64, Vec<u32>>; 3],
}

impl<K: Key> Default for Level<K> {
    fn default() -> Self {
        Self {
            cells: FxMap::default(),
            buckets: Vec::new(),
            block_map: FxMap::default(),
            blocks: Vec::new(),
            slabs: Default::default(),
        }
    }
}

impl<K: Key> Level<K> {
    fn get(&self, c: &Cell) -> Option<&Bucket<K>> {
        self.cells.get(c).map(|&i| &self.buckets[i as usize])
    }

    fn len(&self) -> usize {
        self.buckets.len()
    }

    fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// The range of cells along `axis` that the occupied blocks span, if any.
    fn cell_extent(&self, axis: usize) -> Option<(i64, i64)> {
        let lo = *self.slabs[axis].first_key_value()?.0;
        let hi = *self.slabs[axis].last_key_value()?.0;
        Some((lo << BLOCK_SHIFT, ((hi + 1) << BLOCK_SHIFT) - 1))
    }

    /// Adds `k` with box `b` to cell `c`.
    fn insert(&mut self, c: Cell, k: K, b: Aabb) {
        let block = match self.cells.entry(c) {
            Entry::Occupied(o) => {
                let bucket = &mut self.buckets[*o.get() as usize];
                bucket.items.push(k);
                bucket.bounds = bucket.bounds.union(&b);
                bucket.block
            }
            Entry::Vacant(v) => {
                let bi = self.buckets.len() as u32;
                v.insert(bi);
                let at = block_of(&c);
                let block = match self.block_map.entry(at) {
                    Entry::Occupied(o) => *o.get(),
                    Entry::Vacant(v) => {
                        let n = self.blocks.len() as u32;
                        v.insert(n);
                        let mut slab_at = [0; 3];
                        for (axis, slabs) in self.slabs.iter_mut().enumerate() {
                            let slab = slabs.entry(at[axis]).or_default();
                            slab_at[axis] = slab.len() as u32;
                            slab.push(n);
                        }
                        self.blocks.push(Block {
                            at,
                            buckets: Vec::new(),
                            bounds: Aabb::EMPTY,
                            slab_at,
                        });
                        n
                    }
                };
                let list = &mut self.blocks[block as usize].buckets;
                self.buckets.push(Bucket {
                    cell: c,
                    items: vec![k],
                    bounds: b,
                    block,
                    block_at: list.len() as u32,
                });
                list.push(bi);
                block
            }
        };
        let block = &mut self.blocks[block as usize];
        block.bounds = block.bounds.union(&b);
    }

    /// Takes out the (emptied) bucket `bi`: its cell leaves the map and its block, and
    /// the last bucket takes its place.
    fn remove_bucket(&mut self, bi: usize) {
        let (c, block, at) = {
            let b = &self.buckets[bi];
            (b.cell, b.block as usize, b.block_at as usize)
        };
        self.cells.remove(&c);
        let list = &mut self.blocks[block].buckets;
        debug_assert_eq!(list[at] as usize, bi);
        list.swap_remove(at);
        if let Some(&moved) = list.get(at) {
            self.buckets[moved as usize].block_at = at as u32;
        }
        if list.is_empty() {
            self.remove_block(block);
        }
        self.buckets.swap_remove(bi);
        if let Some(moved) = self.buckets.get(bi) {
            self.cells.insert(moved.cell, bi as u32);
            self.blocks[moved.block as usize].buckets[moved.block_at as usize] = bi as u32;
        }
    }

    /// Takes out the (emptied) block `k`: it leaves the map and the slabs, and the last
    /// block takes its place.
    fn remove_block(&mut self, k: usize) {
        let at = self.blocks[k].at;
        self.block_map.remove(&at);
        for (axis, slabs) in self.slabs.iter_mut().enumerate() {
            if let Some(slab) = slabs.get_mut(&at[axis]) {
                // The block is in the slab once, at `slab_at` (what a search finds).
                let i = self.blocks[k].slab_at[axis] as usize;
                debug_assert_eq!(slab[i] as usize, k);
                slab.swap_remove(i);
                if let Some(&moved) = slab.get(i) {
                    self.blocks[moved as usize].slab_at[axis] = i as u32;
                }
                if slab.is_empty() {
                    slabs.remove(&at[axis]);
                }
            }
        }
        self.blocks.swap_remove(k);
        if let Some(moved) = self.blocks.get(k) {
            let m = moved.at;
            self.block_map.insert(m, k as u32);
            for (axis, slabs) in self.slabs.iter_mut().enumerate() {
                if let Some(slab) = slabs.get_mut(&m[axis]) {
                    slab[moved.slab_at[axis] as usize] = k as u32;
                }
            }
            for &b in &moved.buckets {
                self.buckets[b as usize].block = k as u32;
            }
        }
    }
}

/// One kind of item (vertices, edges or faces) in the grid.
#[derive(Debug, Clone)]
struct Layer<K: Key> {
    levels: Vec<Level<K>>,
    /// Where each item is stored and its box.
    loc: SecondaryMap<K, (u8, Cell, Aabb)>,
}

impl<K: Key> Default for Layer<K> {
    fn default() -> Self {
        Self {
            levels: Vec::new(),
            loc: SecondaryMap::new(),
        }
    }
}

fn cell_of(p: DVec3, cs: f64) -> Cell {
    let q = (p / cs).floor();
    // `as` saturates (and maps NaN to 0): degenerate coordinates still land somewhere.
    [q.x as i64, q.y as i64, q.z as i64]
}

/// The range of cells of size `cs` whose grown boxes meet `b`.
fn cell_range(b: &Aabb, cs: f64) -> (Cell, Cell, f64) {
    let g = b.expand(cs / 2.0);
    let lo = cell_of(g.min, cs);
    let hi = cell_of(g.max, cs);
    let count = (0..3)
        .map(|i| (hi[i] as f64 - lo[i] as f64 + 1.0).max(0.0))
        .product::<f64>();
    (lo, hi, count)
}

fn in_range(c: &Cell, lo: &Cell, hi: &Cell) -> bool {
    (0..3).all(|i| lo[i] <= c[i] && c[i] <= hi[i])
}

impl<K: Key + Ord> Layer<K> {
    fn with_capacity(items: usize) -> Self {
        Self {
            levels: Vec::new(),
            loc: SecondaryMap::with_capacity(items),
        }
    }

    fn len(&self) -> usize {
        self.loc.len()
    }

    fn level_for(cell: f64, b: &Aabb) -> usize {
        let e = b.extent();
        if e.is_nan() || e <= cell {
            return 0;
        }
        ((e / cell).log2().ceil().max(0.0) as usize).min(MAX_LEVEL)
    }

    fn insert(&mut self, cell: f64, k: K, b: Aabb) {
        self.remove(k);
        let level = Self::level_for(cell, &b);
        let cs = cell * (level as f64).exp2();
        let c = cell_of(b.center(), cs);
        if self.levels.len() <= level {
            self.levels.resize_with(level + 1, Default::default);
        }
        self.levels[level].insert(c, k, b);
        self.loc.insert(k, (level as u8, c, b));
    }

    fn remove(&mut self, k: K) {
        let Some((level, c, _)) = self.loc.remove(k) else {
            return;
        };
        let lv = &mut self.levels[level as usize];
        let Some(&bi) = lv.cells.get(&c) else {
            return;
        };
        let bucket = &mut lv.buckets[bi as usize];
        let list = &mut bucket.items;
        if let Some(i) = list.iter().position(|x| *x == k) {
            list.swap_remove(i);
        }
        if list.is_empty() {
            lv.remove_bucket(bi as usize);
        } else {
            bucket.bounds = list
                .iter()
                .fold(Aabb::EMPTY, |b, x| b.union(&self.loc[*x].2));
        }
    }

    /// Items whose box meets `b`, unsorted.
    fn query(&self, cell: f64, b: &Aabb, out: &mut Vec<K>) {
        let keep = |k: &K| self.loc[*k].2.overlaps(b);
        for (level, lv) in self.levels.iter().enumerate() {
            if lv.is_empty() {
                continue;
            }
            let cs = cell * (level as f64).exp2();
            let (lo, hi, count) = cell_range(b, cs);
            if count <= lv.len() as f64 {
                for x in lo[0]..=hi[0] {
                    for y in lo[1]..=hi[1] {
                        for z in lo[2]..=hi[2] {
                            if let Some(list) = lv.get(&[x, y, z])
                                && list.bounds.overlaps(b)
                            {
                                out.extend(list.items.iter().filter(|k| keep(k)));
                            }
                        }
                    }
                }
            } else {
                for list in &lv.buckets {
                    if in_range(&list.cell, &lo, &hi) && list.bounds.overlaps(b) {
                        out.extend(list.items.iter().filter(|k| keep(k)));
                    }
                }
            }
        }
    }

    /// Items whose box comes within `margin` of the segment, unsorted. The segment is cut
    /// into pieces about a cell long so a long diagonal segment visits few cells.
    fn query_segment(&self, cell: f64, a: DVec3, b: DVec3, margin: f64, out: &mut Vec<K>) {
        let whole = Aabb::segment(a, b).expand(margin);
        let len = a.distance(b);
        let mut visit: Vec<Cell> = Vec::new();
        for (level, lv) in self.levels.iter().enumerate() {
            if lv.is_empty() {
                continue;
            }
            let cs = cell * (level as f64).exp2();
            let pieces = (len / cs).ceil().max(1.0);
            let budget = lv.len() as f64;
            visit.clear();
            let mut over = pieces * 8.0 > budget;
            if !over {
                let n = pieces as usize;
                for i in 0..n {
                    let p = a.lerp(b, i as f64 / n as f64);
                    let q = a.lerp(b, (i + 1) as f64 / n as f64);
                    let (lo, hi, count) = cell_range(&Aabb::segment(p, q).expand(margin), cs);
                    if visit.len() as f64 + count > budget {
                        over = true;
                        break;
                    }
                    for x in lo[0]..=hi[0] {
                        for y in lo[1]..=hi[1] {
                            for z in lo[2]..=hi[2] {
                                visit.push([x, y, z]);
                            }
                        }
                    }
                }
            }
            let keep = |k: &K| self.loc[*k].2.overlaps(&whole);
            if over {
                let (lo, hi, _) = cell_range(&whole, cs);
                for list in &lv.buckets {
                    if in_range(&list.cell, &lo, &hi) && list.bounds.overlaps(&whole) {
                        out.extend(list.items.iter().filter(|k| keep(k)));
                    }
                }
            } else {
                visit.sort_unstable();
                visit.dedup();
                for c in &visit {
                    if let Some(list) = lv.get(c)
                        && list.bounds.overlaps(&whole)
                    {
                        out.extend(list.items.iter().filter(|k| keep(k)));
                    }
                }
            }
        }
    }

    /// Items whose box comes within `tol` of the plane `normal . p = d`, unsorted. Only
    /// the slabs (along the normal's main axis) that the plane crosses within the level's
    /// occupied range are visited, and in them only the blocks and cells whose items'
    /// boxes the plane may meet.
    ///
    /// `rel` widens `tol` by that much per inch of distance from the origin, for planes
    /// that agree in direction only within an angle.
    fn query_plane(&self, cell: f64, normal: DVec3, d: f64, tol: f64, rel: f64, out: &mut Vec<K>) {
        let n = normal.to_array();
        let axis = (0..3)
            .max_by(|&i, &j| n[i].abs().total_cmp(&n[j].abs()))
            .unwrap_or(2);
        if n[axis] == 0.0 || !n[axis].is_finite() {
            return;
        }
        for (level, lv) in self.levels.iter().enumerate() {
            if lv.is_empty() {
                continue;
            }
            let cs = cell * (level as f64).exp2();
            let reach = (0..3)
                .map(|i| {
                    let (lo, hi) = lv.cell_extent(i).unwrap_or((0, 0));
                    ((lo as f64).abs().max((hi + 1) as f64).abs() + 1.0) * cs
                })
                .map(|x| x * x)
                .sum::<f64>()
                .sqrt();
            let tol = tol + rel * reach;
            // Extremes of the other two terms of the plane equation over the occupied
            // (grown) range, so the range of the main coordinate on the plane.
            let (mut lo, mut hi) = (d - tol, d + tol);
            for other in (0..3).filter(|&i| i != axis) {
                let Some((first, last)) = lv.cell_extent(other) else {
                    continue;
                };
                let a = first as f64 * cs - cs / 2.0;
                let b = (last + 1) as f64 * cs + cs / 2.0;
                let (t0, t1) = (n[other] * a, n[other] * b);
                lo -= t0.max(t1);
                hi -= t0.min(t1);
            }
            let (x0, x1) = if n[axis] > 0.0 {
                (lo / n[axis], hi / n[axis])
            } else {
                (hi / n[axis], lo / n[axis])
            };
            let first = ((x0 - 1.5 * cs) / cs).floor() as i64 >> BLOCK_SHIFT;
            let last = ((x1 + 0.5 * cs) / cs).ceil() as i64 >> BLOCK_SHIFT;
            for (_, slab) in lv.slabs[axis].range(first..=last) {
                for &k in slab {
                    // The boxes hold their items' boxes, which lie in the cells' grown
                    // boxes: no need to test those too.
                    let block = &lv.blocks[k as usize];
                    if !block.bounds.may_meet_plane(normal, d, tol) {
                        continue;
                    }
                    for &bi in &block.buckets {
                        let bucket = &lv.buckets[bi as usize];
                        if bucket.bounds.may_meet_plane(normal, d, tol) {
                            out.extend(
                                (bucket.items.iter())
                                    .filter(|k| self.loc[**k].2.meets_plane(normal, d, tol)),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Whether `k` is stored with box `b`.
    fn holds(&self, k: K, b: &Aabb) -> bool {
        self.loc.get(k).is_some_and(|(_, _, x)| x == b)
    }
}

/// The grid proper.
#[derive(Debug, Clone)]
pub(crate) struct Grid {
    /// Cell size of level 0, in inches.
    cell: f64,
    vertices: Layer<crate::VertexId>,
    edges: Layer<crate::EdgeId>,
    faces: Layer<crate::FaceId>,
    /// Items when built; the grid is rebuilt (with a new cell size) once the model has
    /// grown well past this.
    built_items: usize,
}

/// The spatial index of an `Entities`: empty until built. Not part of the model: clones
/// start without it, it is skipped from `Debug` details and always compares equal.
#[derive(Default)]
pub struct SpatialIndex {
    pub(crate) grid: Option<Box<Grid>>,
}

impl Clone for SpatialIndex {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for SpatialIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for SpatialIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.grid {
            Some(g) => write!(
                f,
                "SpatialIndex {{ cell: {}, vertices: {}, edges: {}, faces: {} }}",
                g.cell,
                g.vertices.len(),
                g.edges.len(),
                g.faces.len()
            ),
            None => f.write_str("SpatialIndex { not built }"),
        }
    }
}

use crate::entities::{EdgeId, Entities, FaceId, VertexId};

impl Grid {
    pub(crate) fn insert_vertex(&mut self, v: VertexId, p: DVec3) {
        self.vertices.insert(self.cell, v, Aabb::point(p));
    }

    pub(crate) fn insert_edge(&mut self, e: EdgeId, a: DVec3, b: DVec3) {
        self.edges.insert(self.cell, e, Aabb::segment(a, b));
    }

    pub(crate) fn insert_face(&mut self, f: FaceId, b: Aabb) {
        self.faces.insert(self.cell, f, b);
    }

    pub(crate) fn remove_vertex(&mut self, v: VertexId) {
        self.vertices.remove(v);
    }

    pub(crate) fn remove_edge(&mut self, e: EdgeId) {
        self.edges.remove(e);
    }

    pub(crate) fn remove_face(&mut self, f: FaceId) {
        self.faces.remove(f);
    }
}

/// Angle (radians, generously) by which the normals of planes that `Plane::coincides`
/// treats as the same can differ.
const COINCIDE_ANGLE: f64 = 1e-4;

/// Items from which `Entities::build_index` builds its three layers on threads.
const PARALLEL_BUILD: usize = 50_000;

/// Sorts candidates into key (slot map) order.
fn sorted<K: Ord>(mut v: Vec<K>) -> Vec<K> {
    v.sort_unstable();
    v
}

impl Entities {
    /// Bounding box of a face (all its loops).
    pub fn face_bounds(&self, f: FaceId) -> Aabb {
        let mut b = Aabb::EMPTY;
        for l in self.faces[f].loops() {
            for c in &l.coedges {
                if let Some(e) = self.edges.get(c.edge) {
                    for v in [e.start, e.end] {
                        if let Some(x) = self.vertices.get(v) {
                            b.grow(x.pos);
                        }
                    }
                }
            }
        }
        b
    }

    pub fn edge_bounds(&self, e: EdgeId) -> Aabb {
        let edge = &self.edges[e];
        Aabb::segment(self.pos(edge.start), self.pos(edge.end))
    }

    /// The index, if built and in step with the entities.
    pub(crate) fn grid(&self) -> Option<&Grid> {
        self.index.grid.as_deref().filter(|g| {
            g.vertices.len() == self.vertices.len()
                && g.edges.len() == self.edges.len()
                && g.faces.len() == self.faces.len()
        })
    }

    /// Builds the spatial index if it is missing, out of step (items added or removed
    /// without the editing methods) or sized for a much smaller model. Kernel operations
    /// call this; queries on an `Entities` without an index fall back to scans.
    pub fn ensure_index(&mut self) {
        let items = self.vertices.len() + self.edges.len() + self.faces.len();
        let fresh = self
            .grid()
            .is_some_and(|g| items <= 2 * g.built_items + 1024);
        if !fresh {
            self.build_index();
        }
    }

    /// Drops the spatial index (after editing positions directly); the next kernel
    /// operation rebuilds it.
    pub fn invalidate_index(&mut self) {
        self.index.grid = None;
    }

    /// (Re)builds the spatial index. The level-0 cell size is the median edge length,
    /// so a typical vertex query looks at a handful of vertices.
    pub fn build_index(&mut self) {
        let mut lens: Vec<f64> = self
            .edges
            .values()
            .map(|e| self.pos(e.start).distance(self.pos(e.end)))
            .filter(|l| l.is_finite() && *l > 0.0)
            .collect();
        let cell = if lens.is_empty() {
            12.0
        } else {
            let mid = lens.len() / 2;
            *lens.select_nth_unstable_by(mid, f64::total_cmp).1
        }
        .max(16.0 * crate::TOL);
        let built_items = self.vertices.len() + self.edges.len() + self.faces.len();
        // The three layers are independent: a big model builds them side by side.
        let vertices = || {
            let mut l = Layer::with_capacity(self.vertices.len());
            for (v, x) in &self.vertices {
                l.insert(cell, v, Aabb::point(x.pos));
            }
            l
        };
        let edges = || {
            let mut l = Layer::with_capacity(self.edges.len());
            for e in self.edges.keys() {
                l.insert(cell, e, self.edge_bounds(e));
            }
            l
        };
        let faces = || {
            let mut l = Layer::with_capacity(self.faces.len());
            for f in self.faces.keys() {
                l.insert(cell, f, self.face_bounds(f));
            }
            l
        };
        let (vertices, edges, faces) = if built_items >= PARALLEL_BUILD {
            std::thread::scope(|s| {
                let e = s.spawn(edges);
                let f = s.spawn(faces);
                let v = vertices();
                (v, e.join().unwrap(), f.join().unwrap())
            })
        } else {
            (vertices(), edges(), faces())
        };
        self.index.grid = Some(Box::new(Grid {
            cell,
            vertices,
            edges,
            faces,
            built_items,
        }));
    }

    /// Updates the index entry of a face whose loops changed.
    pub(crate) fn refresh_face_bounds(&mut self, f: FaceId) {
        if self.index.grid.is_some() && self.faces.contains_key(f) {
            let b = self.face_bounds(f);
            if let Some(g) = &mut self.index.grid {
                g.insert_face(f, b);
            }
        }
    }

    /// Moves vertices, keeping the index (vertices, their edges and faces) up to date.
    /// Faces using a moved vertex are not otherwise changed.
    pub fn set_vertex_positions(&mut self, moves: &[(VertexId, DVec3)]) {
        for &(v, p) in moves {
            self.vertices[v].pos = p;
        }
        if self.index.grid.is_none() {
            return;
        }
        let mut edges: Vec<EdgeId> = moves
            .iter()
            .flat_map(|(v, _)| self.vertices[*v].edges.iter().copied())
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mut faces: Vec<FaceId> = edges
            .iter()
            .flat_map(|e| self.edges[*e].faces.iter().copied())
            .collect();
        faces.sort_unstable();
        faces.dedup();
        let eb: Vec<(EdgeId, Aabb)> = edges.iter().map(|&e| (e, self.edge_bounds(e))).collect();
        let fb: Vec<(FaceId, Aabb)> = faces.iter().map(|&f| (f, self.face_bounds(f))).collect();
        let g = self.index.grid.as_mut().expect("checked");
        for &(v, p) in moves {
            g.insert_vertex(v, p);
        }
        for (e, b) in eb {
            g.edges.insert(g.cell, e, b);
        }
        for (f, b) in fb {
            g.faces.insert(g.cell, f, b);
        }
    }

    /// Candidate vertices in the box `b`, in key order.
    pub fn vertices_in(&self, b: &Aabb) -> Vec<VertexId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.vertices.query(g.cell, b, &mut out);
                sorted(out)
            }
            None => self.vertices.keys().collect(),
        }
    }

    /// Candidate vertices within `r` of `p`, in key order.
    pub fn vertices_near(&self, p: DVec3, r: f64) -> Vec<VertexId> {
        self.vertices_in(&Aabb::point(p).expand(r))
    }

    /// Candidate vertices within `r` of segment `a-b`, in key order.
    pub fn vertices_near_segment(&self, a: DVec3, b: DVec3, r: f64) -> Vec<VertexId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.vertices.query_segment(g.cell, a, b, r, &mut out);
                sorted(out)
            }
            None => self.vertices.keys().collect(),
        }
    }

    /// Candidate edges meeting the box, in key order.
    pub fn edges_in(&self, b: &Aabb) -> Vec<EdgeId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.edges.query(g.cell, b, &mut out);
                sorted(out)
            }
            None => self.edges.keys().collect(),
        }
    }

    /// Candidate edges within `r` of `p`, in key order.
    pub fn edges_near(&self, p: DVec3, r: f64) -> Vec<EdgeId> {
        self.edges_in(&Aabb::point(p).expand(r))
    }

    /// Candidate edges within `r` of segment `a-b`, in key order.
    pub fn edges_near_segment(&self, a: DVec3, b: DVec3, r: f64) -> Vec<EdgeId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.edges.query_segment(g.cell, a, b, r, &mut out);
                sorted(out)
            }
            None => self.edges.keys().collect(),
        }
    }

    /// Candidate edges within `tol` of the plane, in key order (so what is built from
    /// them does not depend on the index's layout, which depends on its history).
    pub fn edges_near_plane(&self, plane: &crate::geom::Plane, tol: f64) -> Vec<EdgeId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.edges
                    .query_plane(g.cell, plane.normal, plane.d, tol, 0.0, &mut out);
                sorted(out)
            }
            None => self.edges.keys().collect(),
        }
    }

    /// Candidate faces meeting the box, in key order.
    pub fn faces_in(&self, b: &Aabb) -> Vec<FaceId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.faces.query(g.cell, b, &mut out);
                sorted(out)
            }
            None => self.faces.keys().collect(),
        }
    }

    /// Candidate faces whose region may contain `p` (within `r`), in key order.
    pub fn faces_near(&self, p: DVec3, r: f64) -> Vec<FaceId> {
        self.faces_in(&Aabb::point(p).expand(r))
    }

    /// Candidate faces within `tol` of the plane, or whose own plane may coincide with it
    /// (`Plane::coincides` lets directions differ slightly, so far from the origin a
    /// coinciding face can be further away), in key order.
    pub fn faces_near_plane(&self, plane: &crate::geom::Plane, tol: f64) -> Vec<FaceId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.faces
                    .query_plane(g.cell, plane.normal, plane.d, tol, COINCIDE_ANGLE, &mut out);
                sorted(out)
            }
            None => self.faces.keys().collect(),
        }
    }

    /// Index problems (for `validate`): items missing or stored with a stale box.
    pub(crate) fn index_errors(&self) -> Vec<String> {
        let Some(g) = self.index.grid.as_deref() else {
            return Vec::new();
        };
        let mut errs = Vec::new();
        if g.vertices.len() != self.vertices.len()
            || g.edges.len() != self.edges.len()
            || g.faces.len() != self.faces.len()
        {
            // Out of step: never used until rebuilt; only stale boxes are errors.
            return errs;
        }
        for (v, x) in &self.vertices {
            if !g.vertices.holds(v, &Aabb::point(x.pos)) {
                errs.push(format!("spatial index: vertex {v:?} missing or stale"));
            }
        }
        for e in self.edges.keys() {
            if !g.edges.holds(e, &self.edge_bounds(e)) {
                errs.push(format!("spatial index: edge {e:?} missing or stale"));
            }
        }
        for f in self.faces.keys() {
            if !g.faces.holds(f, &self.face_bounds(f)) {
                errs.push(format!("spatial index: face {f:?} missing or stale"));
            }
        }
        errs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_match_scans() {
        let mut s = 3u64;
        let mut rnd = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64 * 200.0 - 100.0
        };
        let mut e = Entities::new();
        let mut vs = Vec::new();
        for _ in 0..400 {
            vs.push(e.add_vertex(DVec3::new(rnd(), rnd(), rnd() / 10.0)));
        }
        for i in 0..399 {
            e.add_raw_edge(vs[i], vs[i + 1], Default::default());
        }
        e.build_index();
        for _ in 0..200 {
            let a = DVec3::new(rnd(), rnd(), rnd() / 10.0);
            let b = DVec3::new(rnd(), rnd(), rnd() / 10.0) / 4.0;
            let r = rnd().abs() / 10.0;
            let near = e.vertices_near(a, r);
            for (v, x) in &e.vertices {
                if x.pos.distance(a) <= r {
                    assert!(near.contains(&v));
                }
            }
            let near = e.edges_near_segment(a, b, r);
            for (id, x) in &e.edges {
                let (p, q) = (e.pos(x.start), e.pos(x.end));
                let d = crate::geom::segment_segment(a, b, p, q).map_or(0.0, |x| x.0);
                if d <= r {
                    assert!(near.contains(&id), "edge near segment missed");
                }
            }
            let near = e.vertices_near_segment(a, b, r);
            for (v, x) in &e.vertices {
                if crate::geom::point_segment(x.pos, a, b).0 <= r {
                    assert!(near.contains(&v), "vertex near segment missed");
                }
            }
        }
        // Moving and removing keep the index in step.
        let moves: Vec<_> = vs[..50]
            .iter()
            .map(|&v| (v, e.pos(v) + DVec3::splat(37.0)))
            .collect();
        e.set_vertex_positions(&moves);
        let edge = e.vertices[vs[60]].edges[0];
        e.remove_edge(edge);
        assert!(e.index_errors().is_empty());
        assert!(e.grid().is_some());
        let plane = crate::geom::Plane::new(DVec3::Z, DVec3::ZERO);
        let on = e.edges_near_plane(&plane, 1e-3);
        for (id, x) in &e.edges {
            if (e.pos(x.start).z > 0.0) != (e.pos(x.end).z > 0.0) {
                assert!(on.contains(&id));
            }
        }
    }

    impl<K: Key> Level<K> {
        /// Panics unless the map, the cells, the blocks and the slabs agree.
        fn check(&self) {
            assert_eq!(self.cells.len(), self.buckets.len());
            assert_eq!(self.block_map.len(), self.blocks.len());
            for (i, b) in self.buckets.iter().enumerate() {
                assert!(!b.items.is_empty());
                assert_eq!(self.cells[&b.cell] as usize, i);
                let block = &self.blocks[b.block as usize];
                assert_eq!(block.at, block_of(&b.cell));
                assert_eq!(block.buckets[b.block_at as usize] as usize, i);
                assert!(block.bounds.union(&b.bounds) == block.bounds);
            }
            for (k, block) in self.blocks.iter().enumerate() {
                assert!(!block.buckets.is_empty());
                assert_eq!(self.block_map[&block.at] as usize, k);
                for axis in 0..3 {
                    assert_eq!(
                        self.slabs[axis][&block.at[axis]][block.slab_at[axis] as usize] as usize,
                        k
                    );
                }
            }
            let listed: usize = (self.slabs.iter())
                .map(|s| s.values().map(Vec::len).sum::<usize>())
                .sum();
            assert_eq!(listed, 3 * self.blocks.len());
        }
    }

    #[test]
    fn plane_queries_match_scans_after_edits() {
        let mut s = 7u64;
        let mut rnd = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        // A wavy, nearly flat sheet of points and small boxes, as a flat model's.
        let mut sm: slotmap::SlotMap<VertexId, ()> = slotmap::SlotMap::with_key();
        let mut layer: Layer<VertexId> = Layer::default();
        let mut keys = Vec::new();
        let cell = 1.0;
        for _ in 0..3000 {
            let k = sm.insert(());
            let p = DVec3::new(rnd() * 60.0, rnd() * 60.0, rnd() * 0.5);
            layer.insert(cell, k, Aabb::point(p).expand(rnd().abs() * 3.0));
            keys.push(k);
        }
        for round in 0..6 {
            // Remove and move some (emptying cells and blocks), then query.
            for (i, &k) in keys.iter().enumerate() {
                if (i + round) % 3 == 0 {
                    layer.remove(k);
                } else if (i + round) % 3 == 1 {
                    let p = DVec3::new(rnd() * 60.0, rnd() * 60.0, rnd() * 0.5);
                    layer.insert(cell, k, Aabb::point(p).expand(rnd().abs()));
                }
            }
            for lv in &layer.levels {
                lv.check();
            }
            for _ in 0..40 {
                let normal = DVec3::new(rnd() * 0.1, rnd() * 0.1, 1.0).normalize();
                let d = rnd() * 0.5;
                let tol = rnd().abs() * 0.01;
                let mut out = Vec::new();
                layer.query_plane(cell, normal, d, tol, 0.0, &mut out);
                for (k, (_, _, b)) in &layer.loc {
                    if b.meets_plane(normal, d, tol) {
                        assert!(out.contains(&k), "plane query missed an item");
                    }
                }
            }
            for &k in &keys {
                if !layer.loc.contains_key(k) {
                    let p = DVec3::new(rnd() * 60.0, rnd() * 60.0, rnd() * 0.5);
                    layer.insert(cell, k, Aabb::point(p));
                }
            }
        }
    }
}
