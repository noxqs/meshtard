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

use std::collections::HashMap;
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
struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
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
type Cells<K> = HashMap<Cell, Vec<K>, BuildHasherDefault<FxHasher>>;

/// Deepest level: cells of `cell * 2^MAX_LEVEL` hold anything.
const MAX_LEVEL: usize = 62;

/// One kind of item (vertices, edges or faces) in the grid.
#[derive(Debug, Clone)]
struct Layer<K: Key> {
    levels: Vec<Cells<K>>,
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
        self.levels[level].entry(c).or_default().push(k);
        self.loc.insert(k, (level as u8, c, b));
    }

    fn remove(&mut self, k: K) {
        let Some((level, c, _)) = self.loc.remove(k) else {
            return;
        };
        let cells = &mut self.levels[level as usize];
        if let Some(list) = cells.get_mut(&c) {
            if let Some(i) = list.iter().position(|x| *x == k) {
                list.swap_remove(i);
            }
            if list.is_empty() {
                cells.remove(&c);
            }
        }
    }

    /// Items whose box meets `b`, unsorted.
    fn query(&self, cell: f64, b: &Aabb, out: &mut Vec<K>) {
        let keep = |k: &K| self.loc[*k].2.overlaps(b);
        for (level, cells) in self.levels.iter().enumerate() {
            if cells.is_empty() {
                continue;
            }
            let cs = cell * (level as f64).exp2();
            let (lo, hi, count) = cell_range(b, cs);
            if count <= cells.len() as f64 {
                for x in lo[0]..=hi[0] {
                    for y in lo[1]..=hi[1] {
                        for z in lo[2]..=hi[2] {
                            if let Some(list) = cells.get(&[x, y, z]) {
                                out.extend(list.iter().filter(|k| keep(k)));
                            }
                        }
                    }
                }
            } else {
                for (c, list) in cells {
                    if in_range(c, &lo, &hi) {
                        out.extend(list.iter().filter(|k| keep(k)));
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
        for (level, cells) in self.levels.iter().enumerate() {
            if cells.is_empty() {
                continue;
            }
            let cs = cell * (level as f64).exp2();
            let pieces = (len / cs).ceil().max(1.0);
            let budget = cells.len() as f64;
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
                for (c, list) in cells {
                    if in_range(c, &lo, &hi) {
                        out.extend(list.iter().filter(|k| keep(k)));
                    }
                }
            } else {
                visit.sort_unstable();
                visit.dedup();
                for c in &visit {
                    if let Some(list) = cells.get(c) {
                        out.extend(list.iter().filter(|k| keep(k)));
                    }
                }
            }
        }
    }

    /// Items whose box comes within `tol` of the plane `normal . p = d`, unsorted.
    fn query_plane(&self, cell: f64, normal: DVec3, d: f64, tol: f64, out: &mut Vec<K>) {
        for (level, cells) in self.levels.iter().enumerate() {
            let cs = cell * (level as f64).exp2();
            for (c, list) in cells {
                let min = DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64) * cs;
                let grown = Aabb {
                    min,
                    max: min + DVec3::splat(cs),
                }
                .expand(cs / 2.0);
                if grown.meets_plane(normal, d, tol) {
                    out.extend(
                        list.iter()
                            .filter(|k| self.loc[**k].2.meets_plane(normal, d, tol)),
                    );
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
        if std::env::var_os("MT_NO_INDEX").is_some() { return; } // TEMP
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
        let mut g = Grid {
            cell,
            vertices: Layer::default(),
            edges: Layer::default(),
            faces: Layer::default(),
            built_items: self.vertices.len() + self.edges.len() + self.faces.len(),
        };
        for (v, x) in &self.vertices {
            g.insert_vertex(v, x.pos);
        }
        for e in self.edges.keys() {
            g.edges.insert(cell, e, self.edge_bounds(e));
        }
        for f in self.faces.keys() {
            g.faces.insert(cell, f, self.face_bounds(f));
        }
        self.index.grid = Some(Box::new(g));
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

    /// Candidate edges within `tol` of the plane, in key order.
    pub fn edges_near_plane(&self, plane: &crate::geom::Plane, tol: f64) -> Vec<EdgeId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.edges
                    .query_plane(g.cell, plane.normal, plane.d, tol, &mut out);
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

    /// Candidate faces within `tol` of the plane, in key order.
    pub fn faces_near_plane(&self, plane: &crate::geom::Plane, tol: f64) -> Vec<FaceId> {
        match self.grid() {
            Some(g) => {
                let mut out = Vec::new();
                g.faces
                    .query_plane(g.cell, plane.normal, plane.d, tol, &mut out);
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
}
