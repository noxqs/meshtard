//! Healing of imported polygon meshes, so they become valid kernel geometry
//! ([`crate::validate`] with [`crate::Checks::ALL`]) the way SketchUp's importers leave
//! them: points closer than the tolerance are welded, polygon sides are split where a
//! vertex lies on them or where they cross another side (in 3D, within the tolerance),
//! and coplanar polygons that overlap are replaced by the cells of their arrangement.
//!
//! - Welding: each point joins the nearest earlier point within [`TOL`], else stays.
//!   Points kept are always more than `TOL` apart.
//! - Splitting ([`split_sides`]): repeated until nothing changes (at most
//!   [`MAX_ROUNDS`] rounds): a vertex within `TOL` of a side splits it; two sides
//!   closer than `TOL` away from their ends get a new (welded) vertex at the crossing
//!   that splits both. A polygon that then visits a vertex twice is cut there into its
//!   simple parts: a part of two vertices (a spike: a sliver whose apex lay on its
//!   base) or turning the other way (folded over) is dropped, so a side the rerouting
//!   took out does not come back as a diagonal. A part that is no longer planar is
//!   ear-clipped into triangles; triangles thinner than [`REACH`] are dropped (their
//!   neighbors were split along them, so no crack opens). Splitting stops when a round
//!   finds what an earlier round did (going round in circles) or the splits grow.
//! - Snapping: when splitting does not settle (a tangle of vertices and sides within
//!   the tolerance of each other, as in the bundles of nearly collinear long sides of
//!   some meshes, where rerouting a side through one vertex brings it back off
//!   another), each vertex still to be inserted into a side is merged into the nearer
//!   end of that side when within [`MERGE`] (iterated snap rounding onto the mesh's
//!   own vertices), and splitting runs again, at most [`MAX_MERGES`] times. Merging
//!   only removes vertices, so this ends.
//! - Overlaps ([`resolve_overlaps`]): faces in one plane whose bounds touch are grouped,
//!   and so are faces facing each other with every corner within the tolerance of the
//!   other's plane (the two sides of a fold or of a solid thinner than the tolerance,
//!   which can be slivers whose planes differ a lot); when any point of the group's
//!   arrangement ([`crate::arrangement`]) is covered by two faces, the group's faces
//!   are replaced by one face per covered cell, taking the material, texture position
//!   and side of the earliest face covering it (as a face drawn on another one in
//!   SketchUp takes that face's material). Corners of the group farther than `TOL / 2`
//!   from its plane are first moved onto it (at most [`MERGE`], when no other face at
//!   them stops being planar and no vertex comes within the tolerance of another);
//!   each cell's loop starts at its corner nearest the middle of its corners' heights,
//!   since a face's plane goes through its first corner. Faces of different planes
//!   that pass through each other are left alone (SketchUp does not intersect them on
//!   import either).

use std::collections::{BTreeMap, BTreeSet};

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{CoEdge, EdgeId, Entities, FaceId, Loop};
use crate::geom::{Plane, point_in_polygon, point_segment, segment_segment, signed_area2};
use crate::spatial::{FxMap, FxSet};
use crate::validate::{Grid, overlaps};

/// Splitting rounds before giving up (each round usually settles everything; a new
/// vertex can bring another within reach of a side, hence a few more).
pub const MAX_ROUNDS: usize = 8;

/// How close a vertex or another side must come to a side to split it: more than the
/// `TOL / 2` that [`crate::validate`] reports, so sides moved by a split stay clear,
/// less than `TOL`, so polygons bent by a split stay within their plane.
pub const REACH: f64 = TOL * 0.75;

/// Times a tangle that splitting does not settle is snapped together (each merges
/// vertices, so this ends) before giving up.
pub const MAX_MERGES: usize = 4;

/// How far a vertex of a tangle may move when snapped onto another.
pub const MERGE: f64 = TOL * 2.0;

/// How far from facing exactly away from each other (1 - cos of the angle) the two
/// sides of a fold may be to be resolved as coplanar faces.
const FOLD: f64 = 1e-4;

/// Points welded within [`TOL`]: indices of the points kept.
#[derive(Debug, Default)]
pub struct Welder {
    pub points: Vec<DVec3>,
    /// Kept points by cell of size `2 TOL`.
    cells: FxMap<[i64; 3], Vec<usize>>,
}

impl Welder {
    const CELL: f64 = 2.0 * TOL;

    /// The nearest kept point within `TOL` of `p` (the earliest on a tie), else `p`
    /// as a new point.
    pub fn weld(&mut self, p: DVec3) -> usize {
        let k = (p / Self::CELL).floor();
        let key = [k.x as i64, k.y as i64, k.z as i64];
        // Cells are twice the reach: along each axis only the neighbor on the side
        // `p` is nearer to can hold a point within reach.
        let f = p / Self::CELL - k;
        let step = |x: f64| if x < 0.5 { -1 } else { 1 };
        let s = [step(f.x), step(f.y), step(f.z)];
        let mut best: Option<(f64, usize)> = None;
        for c in 0..8 {
            let cell = [
                key[0] + if c & 1 != 0 { s[0] } else { 0 },
                key[1] + if c & 2 != 0 { s[1] } else { 0 },
                key[2] + if c & 4 != 0 { s[2] } else { 0 },
            ];
            let Some(list) = self.cells.get(&cell) else {
                continue;
            };
            for &i in list {
                let d = self.points[i].distance(p);
                if d <= TOL && best.is_none_or(|(bd, bi)| (d, i) < (bd, bi)) {
                    best = Some((d, i));
                }
            }
        }
        if let Some((_, i)) = best {
            return i;
        }
        self.points.push(p);
        let i = self.points.len() - 1;
        self.cells.entry(key).or_default().push(i);
        i
    }

    /// Stops welding onto point `i` (merged into another one).
    fn retire(&mut self, i: usize) {
        let k = (self.points[i] / Self::CELL).floor();
        if let Some(list) = self.cells.get_mut(&[k.x as i64, k.y as i64, k.z as i64]) {
            list.retain(|&x| x != i);
        }
    }
}

/// A polygon being healed: welded point indices (counter-clockwise around `normal`) and
/// the index of the mesh polygon it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub vs: Vec<usize>,
    pub normal: DVec3,
    pub src: usize,
}

/// Whether a polygon through `pts` passes the face checks of [`crate::validate`] with
/// normal `normal`: every point in the plane through the first one, and some area. A
/// triangle must also be wider than [`REACH`] everywhere: a thinner one is a sliver
/// whose corner lies on its opposite side (that side gets split instead).
fn face_like(pts: &[DVec3], normal: DVec3) -> bool {
    let plane = Plane::new(normal, pts[0]);
    let flat: Vec<DVec2> = pts.iter().map(|&p| plane.project(p)).collect();
    let area2 = signed_area2(&flat);
    if !pts.iter().all(|&p| plane.contains(p)) || area2 < TOL * TOL {
        return false;
    }
    if let [a, b, c] = pts {
        let longest = a.distance(*b).max(b.distance(*c)).max(c.distance(*a));
        return area2 / longest > REACH;
    }
    true
}

/// The pieces a polygon over welded points `vs` gives: itself when it is a planar
/// polygon through distinct points, else its triangles (ear clipping in its best-fit
/// plane); polygons and triangles without area give none.
pub fn pieces(points: &[DVec3], vs: &[usize], src: usize) -> Vec<Piece> {
    let mut vs: Vec<usize> = vs.to_vec();
    vs.dedup();
    while vs.len() > 1 && vs.first() == vs.last() {
        vs.pop();
    }
    let distinct: BTreeSet<usize> = vs.iter().copied().collect();
    if distinct.len() < 3 {
        return Vec::new();
    }
    let pts: Vec<DVec3> = vs.iter().map(|&v| points[v]).collect();
    let Some(plane) = Plane::from_polygon(&pts) else {
        return Vec::new();
    };
    if distinct.len() == vs.len() && face_like(&pts, plane.normal) {
        return vec![Piece {
            vs,
            normal: plane.normal,
            src,
        }];
    }
    crate::mesh_import::polygon_triangles(&pts, &plane)
        .into_iter()
        .filter_map(|t| {
            let tri = t.map(|k| vs[k]);
            if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
                return None;
            }
            let p = tri.map(|v| points[v]);
            let normal = Plane::from_polygon(&p)?.normal;
            face_like(&p, normal).then(|| Piece {
                vs: tri.to_vec(),
                normal,
                src,
            })
        })
        .collect()
}

/// Splits the sides of `pieces` at vertices lying on them and where they cross (see the
/// module notes). Returns the healed pieces and the number of rounds that split
/// something.
pub fn split_sides(welder: &mut Welder, pieces: Vec<Piece>) -> (Vec<Piece>, usize) {
    let mut checked = None;
    let (mut pieces, mut rounds, mut settled) = split_rounds(welder, pieces, &mut checked);
    for _ in 0..MAX_MERGES {
        if settled {
            break;
        }
        // Sides that keep splitting each other (a tangle of vertices and sides within
        // the tolerance of each other, where rerouting a side through one vertex brings
        // it back off another): snap the tangle's vertices together.
        let inserts = find_splits(welder, &pieces, &mut checked);
        let mut parent: Vec<usize> = (0..welder.points.len()).collect();
        let mut merged = false;
        for (&(a, b), list) in &inserts {
            for &v in list {
                let (pv, pa, pb) = (welder.points[v], welder.points[a], welder.points[b]);
                let near = if pv.distance(pa) <= pv.distance(pb) {
                    a
                } else {
                    b
                };
                if pv.distance(welder.points[near]) > MERGE {
                    continue;
                }
                let (rv, rn) = (find(&mut parent, v), find(&mut parent, near));
                if rv != rn {
                    parent[rv.max(rn)] = rv.min(rn);
                    merged = true;
                }
            }
        }
        if !merged {
            break;
        }
        for i in 0..parent.len() {
            if find(&mut parent, i) != i {
                welder.retire(i);
            }
        }
        let points = &welder.points;
        pieces = pieces
            .into_iter()
            .flat_map(|p| {
                let vs: Vec<usize> = p.vs.iter().map(|&v| find(&mut parent, v)).collect();
                reroute(points, p, vs)
            })
            .collect();
        let (p, r, s) = split_rounds(welder, pieces, &mut checked);
        pieces = p;
        rounds += r;
        settled = s;
    }
    (pieces, rounds)
}

/// Splitting rounds (see [`split_sides`]): the pieces, the number of rounds that split
/// something and whether splitting settled.
fn split_rounds(
    welder: &mut Welder,
    mut pieces: Vec<Piece>,
    checked: &mut Option<Checked>,
) -> (Vec<Piece>, usize, bool) {
    let mut rounds = 0;
    let mut last = usize::MAX;
    let mut seen: Vec<BTreeMap<(usize, usize), Vec<usize>>> = Vec::new();
    for _ in 0..MAX_ROUNDS {
        let inserts = find_splits(welder, &pieces, checked);
        if inserts.is_empty() {
            return (pieces, rounds, true);
        }
        let count: usize = inserts.values().map(Vec::len).sum();
        // Running away (sides along each other within the tolerance keep splitting
        // each other), or going round in circles: stop.
        if (rounds >= 2 && count > last) || seen.contains(&inserts) {
            break;
        }
        last = count;
        rounds += 1;
        let points = &welder.points;
        pieces = pieces
            .into_iter()
            .flat_map(|p| {
                let n = p.vs.len();
                let mut vs = Vec::with_capacity(n + 2);
                for k in 0..n {
                    let (u, v) = (p.vs[k], p.vs[(k + 1) % n]);
                    vs.push(u);
                    if let Some(list) = inserts.get(&(u.min(v), u.max(v))) {
                        if u < v {
                            vs.extend(list.iter().copied());
                        } else {
                            vs.extend(list.iter().rev().copied());
                        }
                    }
                }
                reroute(points, p, vs)
            })
            .collect();
        seen.push(inserts);
    }
    (pieces, rounds, false)
}

/// The pieces piece `p` gives when routed through vertices `vs` instead.
fn reroute(points: &[DVec3], p: Piece, mut vs: Vec<usize>) -> Vec<Piece> {
    if vs == p.vs {
        return vec![p];
    }
    vs.dedup();
    while vs.len() > 1 && vs.first() == vs.last() {
        vs.pop();
    }
    let distinct: BTreeSet<usize> = vs.iter().copied().collect();
    if distinct.len() < 3 {
        return Vec::new();
    }
    let pts: Vec<DVec3> = vs.iter().map(|&v| points[v]).collect();
    if distinct.len() == vs.len() && face_like(&pts, p.normal) {
        // Still flat in its own plane: keep it whole.
        return vec![Piece { vs, ..p }];
    }
    // A polygon routed through one vertex twice pinches there: its parts are separate
    // polygons; a part between two visits of a vertex one vertex apart is a spike (a
    // sliver collapsed onto a side) and goes, and so does a part turning the other way
    // (folded over). Splitting the parts apart rather than triangulating the whole
    // keeps sides that the rerouting removed from coming back as diagonals.
    let mut out = Vec::new();
    for cycle in simple_cycles(&vs) {
        if newell(points, &cycle).dot(p.normal) <= 0.0 {
            continue;
        }
        out.extend(pieces(points, &cycle, p.src));
    }
    // Keep the side the polygon faced.
    for q in &mut out {
        if q.normal.dot(p.normal) < 0.0 {
            q.vs.reverse();
            q.normal = -q.normal;
        }
    }
    out
}

/// The simple cycles of a closed vertex sequence that visits vertices more than once
/// (cut where a vertex repeats); parts with fewer than 3 vertices are left out.
fn simple_cycles(vs: &[usize]) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for &v in vs.iter().chain(vs.first()) {
        if let Some(i) = stack.iter().position(|&x| x == v) {
            let cycle: Vec<usize> = stack.drain(i..).collect();
            if cycle.len() >= 3 {
                out.push(cycle);
            }
        }
        stack.push(v);
    }
    out
}

/// Newell's normal of a closed polygon (length twice its area).
fn newell(points: &[DVec3], vs: &[usize]) -> DVec3 {
    let n = vs.len();
    (0..n)
        .map(|k| points[vs[k]].cross(points[vs[(k + 1) % n]]))
        .sum()
}

/// Query results of a [`Grid`] without sorting them.
#[derive(Debug, Default)]
struct Marks {
    seen: Vec<u32>,
    mark: u32,
    out: Vec<usize>,
}

impl Marks {
    fn query(&mut self, grid: &Grid, a: DVec3, b: DVec3) -> &[usize] {
        self.mark += 1;
        grid.query_marked(a, b, &mut self.seen, self.mark, &mut self.out);
        &self.out
    }
}

/// Sides and vertices a splitting round found nothing to do with: later rounds only
/// look at sides and vertices that are new.
#[derive(Debug, Default)]
struct Checked {
    sides: FxSet<(usize, usize)>,
    vertices: FxSet<usize>,
}

/// Vertices to insert into each side `(low, high)` (welded point indices), in order from
/// `low` to `high`. Empty when no side needs splitting. Pairs of sides and vertices both
/// in `checked` are skipped; `checked` is then updated to everything in `pieces`.
fn find_splits(
    welder: &mut Welder,
    pieces: &[Piece],
    checked: &mut Option<Checked>,
) -> BTreeMap<(usize, usize), Vec<usize>> {
    let mut sides: Vec<(usize, usize)> = Vec::new();
    let mut known: FxSet<(usize, usize)> = FxSet::default();
    let mut used: BTreeSet<usize> = BTreeSet::new();
    for p in pieces {
        let n = p.vs.len();
        for k in 0..n {
            let (u, v) = (p.vs[k], p.vs[(k + 1) % n]);
            let side = (u.min(v), u.max(v));
            if known.insert(side) {
                sides.push(side);
            }
            used.insert(u);
        }
    }
    let used: Vec<usize> = used.into_iter().collect();
    let dirty: Vec<bool> = sides
        .iter()
        .map(|s| checked.as_ref().is_none_or(|c| !c.sides.contains(s)))
        .collect();
    let pad = DVec3::splat(TOL);
    let pos = |v: usize, w: &Welder| w.points[v];
    let boxes: Vec<(DVec3, DVec3)> = sides
        .iter()
        .map(|&(a, b)| {
            let (p, q) = (pos(a, welder), pos(b, welder));
            (p.min(q) - pad, p.max(q) + pad)
        })
        .collect();
    let side_grid = Grid::new(&boxes);
    // Points go in cells the size of the sides that query them.
    let vertex_grid = Grid::with_cell(
        &used
            .iter()
            .map(|&v| (pos(v, welder), pos(v, welder)))
            .collect::<Vec<_>>(),
        Grid::median_size(&boxes),
    );
    let mut found: Vec<Vec<usize>> = vec![Vec::new(); sides.len()];
    let (mut vertex_marks, mut side_marks) = (Marks::default(), Marks::default());
    for (i, &(a, b)) in sides.iter().enumerate() {
        if !dirty[i] {
            continue;
        }
        let (p0, p1) = (pos(a, welder), pos(b, welder));
        for &k in vertex_marks.query(&vertex_grid, boxes[i].0, boxes[i].1) {
            let v = used[k];
            if v == a || v == b {
                continue;
            }
            let (d, _) = point_segment(pos(v, welder), p0, p1);
            if d <= REACH {
                found[i].push(v);
            }
        }
        for &j in side_marks.query(&side_grid, boxes[i].0, boxes[i].1) {
            let (c, e) = sides[j];
            if (dirty[j] && j <= i)
                || !overlaps(&boxes[i], &boxes[j])
                || c == a
                || c == b
                || e == a
                || e == b
            {
                continue;
            }
            let (q0, q1) = (pos(c, welder), pos(e, welder));
            let Some((d, s, t)) = segment_segment(p0, p1, q0, q1) else {
                continue;
            };
            if d > REACH {
                continue;
            }
            let (ci, cj) = (p0 + (p1 - p0) * s, q0 + (q1 - q0) * t);
            let x = welder.weld((ci + cj) / 2.0);
            if x != a && x != b {
                found[i].push(x);
            }
            if x != c && x != e {
                found[j].push(x);
            }
        }
    }
    // New vertices on sides checked before.
    if let Some(c) = checked.as_ref() {
        for &v in used.iter().filter(|v| !c.vertices.contains(v)) {
            let p = pos(v, welder);
            for j in side_grid.query(p, p) {
                let (a, b) = sides[j];
                if dirty[j] || v == a || v == b || !overlaps(&boxes[j], &(p, p)) {
                    continue;
                }
                let (d, _) = point_segment(p, pos(a, welder), pos(b, welder));
                if d <= REACH {
                    found[j].push(v);
                }
            }
        }
    }
    // Sides found to need splitting stay unchecked: rerouting can bring one back.
    for (side, list) in sides.iter().zip(&found) {
        if !list.is_empty() {
            known.remove(side);
        }
    }
    *checked = Some(Checked {
        sides: known,
        vertices: used.iter().copied().collect(),
    });
    let mut out = BTreeMap::new();
    for (i, mut list) in found.into_iter().enumerate() {
        let (a, b) = sides[i];
        let (p0, p1) = (welder.points[a], welder.points[b]);
        let dir = p1 - p0;
        let len2 = dir.length_squared();
        if list.is_empty() || len2 == 0.0 {
            continue;
        }
        let param = |v: usize| (welder.points[v] - p0).dot(dir) / len2;
        list.sort_unstable();
        list.dedup();
        list.retain(|&v| {
            let t = param(v);
            t > 0.0 && t < 1.0
        });
        list.sort_by(|&x, &y| param(x).total_cmp(&param(y)).then(x.cmp(&y)));
        if !list.is_empty() {
            out.insert((a, b), list);
        }
    }
    out
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut k = i;
    while parent[k] != r {
        let next = parent[k];
        parent[k] = r;
        k = next;
    }
    r
}

/// Replaces coplanar faces that overlap by the cells of their arrangement (see the
/// module notes). Returns the number of groups of faces replaced.
///
/// Faces made from cells take their group's plane, which can make them coplanar with a
/// face that was not (planes coincide within 1e-9 of a turn), so this repeats a few
/// times while groups change.
pub fn resolve_overlaps(ents: &mut Entities) -> usize {
    let n = resolve_overlaps_touching(ents, None);
    drop_repeated_faces(ents);
    n
}

/// [`resolve_overlaps`] without dropping repeated faces, looking at first only at
/// groups with a face of `touching` (all of them when `None`).
pub fn resolve_overlaps_touching(
    ents: &mut Entities,
    touching: Option<&BTreeSet<FaceId>>,
) -> usize {
    let mut total = 0;
    for k in 0..3 {
        let n = resolve_overlaps_once(ents, if k == 0 { touching } else { None });
        total += n;
        if n == 0 {
            break;
        }
    }
    total
}

/// Removes faces bounded by the same edges as an earlier face (the two sides of a
/// sliver whose normals disagree too much to count as one plane).
pub fn drop_repeated_faces(ents: &mut Entities) {
    let mut seen: BTreeSet<Vec<EdgeId>> = BTreeSet::new();
    let faces: Vec<FaceId> = ents.faces.keys().collect();
    for f in faces {
        let mut edges: Vec<EdgeId> = ents.faces[f]
            .loops()
            .flat_map(|l| l.coedges.iter().map(|c| c.edge))
            .collect();
        edges.sort();
        edges.dedup();
        if !seen.insert(edges) {
            ents.remove_face(f);
        }
    }
}

fn resolve_overlaps_once(ents: &mut Entities, touching: Option<&BTreeSet<FaceId>>) -> usize {
    let faces: Vec<FaceId> = ents.faces.keys().collect();
    let wanted = |i: usize| touching.is_none_or(|t| t.contains(&faces[i]));
    let planes: Vec<Plane> = faces.iter().map(|&f| ents.face_plane(f)).collect();
    let pad = DVec3::splat(TOL);
    let boxes: Vec<(DVec3, DVec3)> = faces
        .iter()
        .map(|&f| {
            let pts = ents.loop_points(&ents.faces[f].outer);
            let lo = pts.iter().fold(DVec3::INFINITY, |m, p| m.min(*p));
            let hi = pts.iter().fold(DVec3::NEG_INFINITY, |m, p| m.max(*p));
            (lo - pad, hi + pad)
        })
        .collect();
    let grid = Grid::new(&boxes);
    let within = |f: FaceId, plane: &Plane| {
        ents.faces[f].loops().all(|l| {
            l.coedges
                .iter()
                .all(|&c| plane.contains(ents.pos(ents.coedge_ends(c).0)))
        })
    };
    // Faces facing each other with every corner within the tolerance of the other's
    // plane: the two sides of a fold or of a solid thinner than the tolerance, whose
    // planes can differ a lot when they are slivers. Exports give them the same
    // triangles; resolve them as coplanar faces.
    let folded = |i: usize, j: usize| {
        ents.faces[faces[i]].normal.dot(ents.faces[faces[j]].normal) < FOLD - 1.0
            && within(faces[j], &planes[i])
            && within(faces[i], &planes[j])
    };
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    let mut marks = Marks::default();
    // Pairs with a wanted face, each once.
    for i in (0..faces.len()).filter(|&i| wanted(i)) {
        for &j in marks.query(&grid, boxes[i].0, boxes[i].1) {
            if j != i
                && (j > i || !wanted(j))
                && overlaps(&boxes[i], &boxes[j])
                && (planes[i].coincides(&planes[j]) || folded(i, j))
            {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..faces.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut replaced = 0;
    for group in groups.values().filter(|g| g.len() > 1) {
        let fs: Vec<FaceId> = group.iter().map(|&i| faces[i]).collect();
        let bx: Vec<(DVec3, DVec3)> = group.iter().map(|&i| boxes[i]).collect();
        if resolve_group(ents, &fs, &bx, planes[group[0]]) {
            replaced += 1;
        }
    }
    replaced
}

/// Whether `q` is inside a face region (outer polygon and holes in plane coordinates).
fn region_contains(q: DVec2, outer: &[DVec2], holes: &[Vec<DVec2>]) -> bool {
    point_in_polygon(q, outer) && !holes.iter().any(|h| point_in_polygon(q, h))
}

/// One group of coplanar faces whose bounds touch: replaced by their arrangement's
/// cells when two of them overlap. Returns whether it was replaced.
fn resolve_group(
    ents: &mut Entities,
    faces: &[FaceId],
    boxes: &[(DVec3, DVec3)],
    plane: Plane,
) -> bool {
    let mut edges: BTreeSet<EdgeId> = BTreeSet::new();
    for &f in faces {
        for l in ents.faces[f].loops() {
            edges.extend(l.coedges.iter().map(|c| c.edge));
        }
    }
    let list: Vec<EdgeId> = edges.iter().copied().collect();
    if !may_overlap(ents, faces, boxes, &plane)
        && edges_crossing_in_plane(ents, &list, &plane).is_empty()
    {
        return false;
    }
    if !snap_into_plane(ents, faces, &edges, &plane) {
        return false; // Drifts out of the plane: leave it.
    }
    let mut edges = list;
    // Faces apart by nearly the tolerance (a sliver solid's two sides) can have sides
    // that cross seen in the plane but were too far apart to be split: split them now.
    let mut settled = false;
    for _ in 0..4 {
        let crossings = edges_crossing_in_plane(ents, &edges, &plane);
        if crossings.is_empty() {
            settled = true;
            break;
        }
        split_at_crossings(ents, &edges, crossings);
        let mut set: BTreeSet<EdgeId> = BTreeSet::new();
        for &f in faces {
            for l in ents.faces[f].loops() {
                set.extend(l.coedges.iter().map(|c| c.edge));
            }
        }
        edges = set.into_iter().collect();
    }
    if !settled {
        return false;
    }
    let cells = crate::arrangement::cells(ents, &plane, &edges);
    let regions: Vec<(Vec<DVec2>, Vec<Vec<DVec2>>)> = faces
        .iter()
        .map(|&f| ents.face_polygons(f, &plane))
        .collect();
    let grid = Grid::new(boxes);
    let covers: Vec<Vec<usize>> = cells
        .iter()
        .map(|cell| {
            let q = plane.project(cell.sample);
            grid.query(cell.sample, cell.sample)
                .into_iter()
                .filter(|&k| {
                    overlaps(&boxes[k], &(cell.sample, cell.sample))
                        && region_contains(q, &regions[k].0, &regions[k].1)
                })
                .collect()
        })
        .collect();
    if covers.iter().all(|c| c.len() <= 1) {
        return false;
    }
    let kept: Vec<(DVec3, crate::entities::FaceAttrs)> = faces
        .iter()
        .map(|&f| (ents.faces[f].normal, ents.faces[f].attrs.clone()))
        .collect();
    for &f in faces {
        ents.remove_face(f);
    }
    let flip = |l: &[CoEdge]| -> Vec<CoEdge> {
        l.iter()
            .rev()
            .map(|c| CoEdge {
                edge: c.edge,
                reversed: !c.reversed,
            })
            .collect()
    };
    for (cell, cover) in cells.into_iter().zip(covers) {
        let Some(&k) = cover.first() else { continue };
        let (normal, attrs) = &kept[k];
        let (outer, holes, normal) = if normal.dot(plane.normal) < 0.0 {
            (
                flip(&cell.outer),
                cell.holes.iter().map(|h| flip(h)).collect(),
                -plane.normal,
            )
        } else {
            (cell.outer, cell.holes, plane.normal)
        };
        let outer = steady_start(ents, outer, &holes, normal);
        ents.add_raw_face(
            Loop { coedges: outer },
            holes.into_iter().map(|coedges| Loop { coedges }).collect(),
            normal,
            attrs.clone(),
        );
    }
    for e in edges {
        if ents.edges.get(e).is_some_and(|x| x.faces.is_empty()) {
            let (a, b) = (ents.edges[e].start, ents.edges[e].end);
            ents.unlink_edge(e);
            ents.remove_vertex_if_unused(a);
            ents.remove_vertex_if_unused(b);
        }
    }
    true
}

/// `outer` started at the corner whose height along `normal` is nearest the middle of
/// the face's corners' heights: a face's plane goes through its first corner, and
/// corners within the tolerance of the group's plane on both sides can be farther
/// than that from a corner on one side.
fn steady_start(
    ents: &Entities,
    mut outer: Vec<CoEdge>,
    holes: &[Vec<CoEdge>],
    normal: DVec3,
) -> Vec<CoEdge> {
    let height = |c: &CoEdge| normal.dot(ents.pos(ents.coedge_ends(*c).0));
    let (lo, hi) = outer
        .iter()
        .chain(holes.iter().flatten())
        .map(height)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| {
            (lo.min(h), hi.max(h))
        });
    let mid = (lo + hi) / 2.0;
    let best = (0..outer.len())
        .min_by(|&i, &j| {
            (height(&outer[i]) - mid)
                .abs()
                .total_cmp(&(height(&outer[j]) - mid).abs())
                .then(i.cmp(&j))
        })
        .unwrap_or(0);
    outer.rotate_left(best);
    outer
}

/// Moves the ends of `edges` (the edges of coplanar group `faces`) farther than
/// `TOL / 2` from `plane` onto it, so the faces made in the plane are flat whatever
/// corner they start at (faces at a tiny angle along a long side can have a corner
/// just out of the plane of a face they overlap): when they are within [`MERGE`] of
/// it, no vertex comes within the tolerance of another or half of it of an edge, and
/// every other face at them stays planar (turned to the plane through its corners
/// when it has to). Returns whether it did (or nothing had to move).
fn snap_into_plane(
    ents: &mut Entities,
    faces: &[FaceId],
    edges: &BTreeSet<EdgeId>,
    plane: &Plane,
) -> bool {
    let mut moves: BTreeMap<crate::entities::VertexId, DVec3> = BTreeMap::new();
    for &e in edges {
        for v in [ents.edges[e].start, ents.edges[e].end] {
            let p = ents.pos(v);
            let d = plane.distance(p);
            if d.abs() > TOL / 2.0 {
                if d.abs() > MERGE {
                    return false;
                }
                moves.insert(v, p - plane.normal * d);
            }
        }
    }
    if moves.is_empty() {
        return true;
    }
    // Moved vertices must stay apart from every other vertex.
    let mut moved = Welder::default();
    for (&v, &q) in &moves {
        let i = moved.weld(q);
        if i + 1 != moved.points.len()
            || ents
                .vertices_near(q, TOL)
                .into_iter()
                .any(|u| u != v && !moves.contains_key(&u) && ents.pos(u).distance(q) <= TOL)
        {
            return false;
        }
    }
    let at = |v: crate::entities::VertexId| moves.get(&v).copied().unwrap_or(ents.pos(v));
    // Nor onto an edge, nor an edge at them onto a vertex.
    let clear = |v: crate::entities::VertexId, e: EdgeId| {
        let (a, b) = (ents.edges[e].start, ents.edges[e].end);
        v == a || v == b || point_segment(at(v), at(a), at(b)).0 > TOL / 2.0
    };
    for (&v, &q) in &moves {
        if !ents.edges_near(q, MERGE).into_iter().all(|e| clear(v, e)) {
            return false;
        }
        for &e in &ents.vertices[v].edges {
            let (a, b) = (at(ents.edges[e].start), at(ents.edges[e].end));
            let near = ents.vertices_near_segment(a, b, MERGE);
            if !near.into_iter().all(|u| clear(u, e)) {
                return false;
            }
        }
    }
    let group: BTreeSet<FaceId> = faces.iter().copied().collect();
    let mut others: BTreeSet<FaceId> = BTreeSet::new();
    for &v in moves.keys() {
        for &e in &ents.vertices[v].edges {
            others.extend(ents.edges[e].faces.iter().filter(|f| !group.contains(f)));
        }
    }
    // Other faces at moved vertices stay planar, turned to the plane through their
    // moved corners when they have to.
    let mut normals: Vec<(FaceId, DVec3)> = Vec::new();
    for f in others {
        let face = &ents.faces[f];
        let first = at(ents.coedge_ends(face.outer.coedges[0]).0);
        let flat = |normal: DVec3| {
            let own = Plane::new(normal, first);
            face.loops().all(|l| {
                l.coedges
                    .iter()
                    .all(|&c| own.contains(at(ents.coedge_ends(c).0)))
            })
        };
        if flat(face.normal) {
            continue;
        }
        let pts: Vec<DVec3> = face
            .outer
            .coedges
            .iter()
            .map(|&c| at(ents.coedge_ends(c).0))
            .collect();
        let n = (0..pts.len())
            .map(|k| pts[k].cross(pts[(k + 1) % pts.len()]))
            .sum::<DVec3>()
            .normalize_or_zero();
        let n = if n.dot(face.normal) < 0.0 { -n } else { n };
        if n.dot(face.normal) < 0.9 || !flat(n) {
            return false;
        }
        normals.push((f, n));
    }
    let moves: Vec<_> = moves.into_iter().collect();
    ents.set_vertex_positions(&moves);
    for (f, n) in normals {
        ents.faces[f].normal = n;
    }
    true
}

/// Whether faces of a coplanar group may overlap (else they surely do not, provided
/// their sides do not cross): two of them run along an edge the same way, or a vertex
/// or the middle of an edge of the group lies inside one of them, away from its sides.
fn may_overlap(ents: &Entities, faces: &[FaceId], boxes: &[(DVec3, DVec3)], plane: &Plane) -> bool {
    let mut ways: BTreeMap<EdgeId, [u32; 2]> = BTreeMap::new();
    let mut points: Vec<DVec3> = Vec::new();
    for &f in faces {
        let up = ents.faces[f].normal.dot(plane.normal) >= 0.0;
        for l in ents.faces[f].loops() {
            for c in &l.coedges {
                let w = ways.entry(c.edge).or_default();
                w[usize::from(c.reversed == up)] += 1;
                if w.iter().any(|&n| n > 1) {
                    return true;
                }
            }
        }
    }
    let mut vertices: BTreeSet<crate::entities::VertexId> = BTreeSet::new();
    for &e in ways.keys() {
        let (a, b) = (ents.edges[e].start, ents.edges[e].end);
        vertices.extend([a, b]);
        points.push((ents.pos(a) + ents.pos(b)) / 2.0);
    }
    points.extend(vertices.into_iter().map(|v| ents.pos(v)));
    let regions: Vec<Vec<Vec<DVec2>>> = faces
        .iter()
        .map(|&f| {
            let (outer, holes) = ents.face_polygons(f, plane);
            std::iter::once(outer).chain(holes).collect()
        })
        .collect();
    let grid = Grid::new(boxes);
    let eps = TOL * 1e-3;
    points.iter().any(|&p| {
        let q = plane.project(p);
        grid.query(p, p).into_iter().any(|k| {
            let r = &regions[k];
            overlaps(&boxes[k], &(p, p))
                && region_contains(q, &r[0], &r[1..])
                && r.iter().all(|l| {
                    (0..l.len()).all(|i| {
                        let (a, b) = (l[i], l[(i + 1) % l.len()]);
                        let ab = b - a;
                        let t = ((q - a).dot(ab) / ab.length_squared().max(1e-300)).clamp(0.0, 1.0);
                        q.distance(a + ab * t) > eps
                    })
                })
        })
    })
}

/// Pairs of `edges` (indices, with the parameters of the crossing along each) that
/// cross away from their ends, projected into `plane`, without a shared vertex.
fn edges_crossing_in_plane(
    ents: &Entities,
    edges: &[EdgeId],
    plane: &Plane,
) -> Vec<(usize, usize, f64, f64)> {
    let ends: Vec<(crate::entities::VertexId, crate::entities::VertexId)> = edges
        .iter()
        .map(|&e| (ents.edges[e].start, ents.edges[e].end))
        .collect();
    let flat: Vec<(DVec2, DVec2)> = ends
        .iter()
        .map(|&(a, b)| (plane.project(ents.pos(a)), plane.project(ents.pos(b))))
        .collect();
    let boxes: Vec<(DVec3, DVec3)> = ends
        .iter()
        .map(|&(a, b)| {
            let (p, q) = (ents.pos(a), ents.pos(b));
            (p.min(q) - DVec3::splat(TOL), p.max(q) + DVec3::splat(TOL))
        })
        .collect();
    let grid = Grid::new(&boxes);
    let mut out = Vec::new();
    for i in 0..edges.len() {
        for j in grid.query(boxes[i].0, boxes[i].1) {
            let ((a, b), (c, d)) = (ends[i], ends[j]);
            if j <= i || a == c || a == d || b == c || b == d {
                continue;
            }
            let ((p0, p1), (q0, q1)) = (flat[i], flat[j]);
            let (r, e) = (p1 - p0, q1 - q0);
            let den = r.perp_dot(e);
            if den.abs() <= 1e-12 * r.length() * e.length() {
                continue;
            }
            let s = (q0 - p0).perp_dot(e) / den;
            let t = (q0 - p0).perp_dot(r) / den;
            if s > 0.0 && s < 1.0 && t > 0.0 && t < 1.0 {
                out.push((i, j, s, t));
            }
        }
    }
    out
}

/// Splits `edges` at `crossings` (from [`edges_crossing_in_plane`]): at a new vertex
/// halfway between the two edges' points, or a vertex already within `TOL` of it.
fn split_at_crossings(
    ents: &mut Entities,
    edges: &[EdgeId],
    crossings: Vec<(usize, usize, f64, f64)>,
) {
    let at = |ents: &Entities, e: EdgeId, t: f64| {
        let (a, b) = (ents.pos(ents.edges[e].start), ents.pos(ents.edges[e].end));
        a + (b - a) * t
    };
    let mut splits: BTreeMap<usize, Vec<(f64, crate::entities::VertexId)>> = BTreeMap::new();
    for (i, j, s, t) in crossings {
        let p = (at(ents, edges[i], s) + at(ents, edges[j], t)) / 2.0;
        // (All of `edges` exist here: nothing is split before every point is known.)
        let v = ents.find_vertex(p).unwrap_or_else(|| ents.add_vertex(p));
        splits.entry(i).or_default().push((s, v));
        splits.entry(j).or_default().push((t, v));
    }
    for (i, mut list) in splits {
        list.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        list.dedup_by_key(|x| x.1);
        let mut e = edges[i];
        for (_, v) in list {
            // Gone when an earlier split's half joined it and split it: the next round
            // finds what is left.
            let Some(edge) = ents.edges.get(e) else {
                break;
            };
            if v == edge.start || v == edge.end {
                continue;
            }
            let (first, rest) = ents.split_edge(e, v);
            let rest = use_existing_edge(ents, rest);
            use_existing_edge(ents, first);
            e = rest;
        }
    }
}

/// When another edge joins the ends of `e`, moves `e`'s faces onto it and removes `e`.
/// Returns the edge left.
fn use_existing_edge(ents: &mut Entities, e: EdgeId) -> EdgeId {
    let (a, b) = (ents.edges[e].start, ents.edges[e].end);
    let Some(other) = ents.vertices[a]
        .edges
        .iter()
        .copied()
        .find(|&x| x != e && ents.edges[x].other(a) == b)
    else {
        return e;
    };
    let other_start = ents.edges[other].start;
    for f in std::mem::take(&mut ents.edges[e].faces) {
        for l in ents.faces[f].loops_mut() {
            for c in l.coedges.iter_mut().filter(|c| c.edge == e) {
                let start = if c.reversed { b } else { a };
                *c = CoEdge {
                    edge: other,
                    reversed: start != other_start,
                };
            }
        }
        if !ents.edges[other].faces.contains(&f) {
            ents.edges[other].faces.push(f);
        }
    }
    ents.unlink_edge(e);
    other
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use crate::mesh_import::{Mesh, MeshOptions, Polygon, to_entities};
    use crate::{Checks, Entities, validate};

    fn mesh(points: &[[f64; 3]], polys: &[(&[usize], Option<usize>)]) -> Mesh {
        Mesh {
            positions: points.iter().map(|p| DVec3::from_array(*p)).collect(),
            polygons: polys
                .iter()
                .map(|(vs, m)| Polygon {
                    vertices: vs.to_vec(),
                    material: *m,
                    uvs: Vec::new(),
                })
                .collect(),
            materials: Vec::new(),
        }
    }

    fn import(m: &Mesh, merge: bool) -> Entities {
        let opts = MeshOptions {
            inches_per_unit: 1.0,
            y_up: false,
            merge_coplanar: merge,
        };
        let ents = to_entities(m, &opts, &[10, 11]);
        let errs = validate(&ents, Checks::ALL);
        assert!(errs.is_empty(), "{errs:?}");
        ents
    }

    fn area(ents: &Entities) -> f64 {
        ents.faces
            .keys()
            .map(|f| {
                let plane = ents.face_plane(f);
                let (outer, holes) = ents.face_polygons(f, &plane);
                (crate::geom::signed_area2(&outer)
                    + holes
                        .iter()
                        .map(|h| crate::geom::signed_area2(h))
                        .sum::<f64>())
                    / 2.0
            })
            .sum()
    }

    /// A vertex on another polygon's side (a T-junction) splits the side.
    #[test]
    fn t_junction_splits_the_side() {
        // A 2x1 upright rectangle with a floor below it and a roof triangle whose
        // corner lies on the middle of the rectangle's top side.
        let m = mesh(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0],
                [2.0, -1.0, 0.0],
                [2.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            &[
                (&[0, 1, 2, 4], None),
                (&[0, 5, 6, 1], None),
                (&[3, 7, 8], None),
            ],
        );
        let ents = import(&m, false);
        // The rectangle's top side is split at the middle vertex.
        let top = ents
            .faces
            .values()
            .find(|f| f.normal.y.abs() > 0.9)
            .expect("rectangle");
        assert_eq!(top.outer.coedges.len(), 5);
        assert!((area(&ents) - 5.0).abs() < 1e-9);
    }

    /// Sides of two polygons that cross get a shared vertex.
    #[test]
    fn crossing_sides_are_split() {
        // Two triangles in perpendicular planes whose sides cross at (1, 0, 0).
        let m = mesh(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 0.0, 2.0],
                [1.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, 0.0, -2.0],
            ],
            &[(&[0, 1, 2], None), (&[3, 4, 5], None)],
        );
        let ents = import(&m, false);
        assert_eq!(ents.vertices.len(), 7);
        assert!(
            ents.vertices
                .values()
                .any(|v| v.pos.distance(DVec3::new(1.0, 0.0, 0.0)) < 1e-9)
        );
    }

    /// Points closer than the tolerance weld, even across a welding cell boundary.
    #[test]
    fn near_points_weld() {
        let e = crate::TOL * 0.3;
        let m = mesh(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0 + e, -e, 0.0],
                [1.0, 1.0, 0.0],
                [-e, 1.0 + e, 0.0],
            ],
            &[(&[0, 1, 2], None), (&[3, 4, 5], None)],
        );
        let ents = import(&m, false);
        assert_eq!(ents.vertices.len(), 4);
        assert_eq!(ents.edges.len(), 5);
    }

    /// Coplanar polygons that overlap become the cells of their arrangement; the
    /// overlap takes the first polygon's material.
    #[test]
    fn coplanar_overlaps_are_resolved() {
        let m = mesh(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
                [1.0, 1.0, 0.0],
                [3.0, 1.0, 0.0],
                [3.0, 3.0, 0.0],
                [1.0, 3.0, 0.0],
            ],
            &[(&[0, 1, 2, 3], Some(0)), (&[4, 5, 6, 7], Some(1))],
        );
        let ents = import(&m, false);
        assert_eq!(ents.faces.len(), 3);
        assert!((area(&ents) - 7.0).abs() < 1e-9);
        let first: f64 = ents
            .faces
            .keys()
            .filter(|&f| ents.faces[f].attrs.material == Some(10))
            .map(|f| {
                let plane = ents.face_plane(f);
                crate::geom::signed_area2(&ents.face_polygons(f, &plane).0) / 2.0
            })
            .sum();
        assert!((first - 4.0).abs() < 1e-9, "{first}");
        // A polygon inside another one leaves a hole in it, filled by a face of the
        // first one's material (which merging joins to it).
        let m = mesh(
            &[
                [0.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 4.0, 0.0],
                [0.0, 4.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
                [2.0, 2.0, 0.0],
                [1.0, 2.0, 0.0],
            ],
            &[(&[0, 1, 2, 3], Some(0)), (&[4, 5, 6, 7], Some(1))],
        );
        for merge in [false, true] {
            let ents = import(&m, merge);
            assert_eq!(ents.faces.len(), if merge { 1 } else { 2 });
            assert!((area(&ents) - 16.0).abs() < 1e-9);
            assert!(ents.faces.values().all(|f| f.attrs.material == Some(10)));
            assert_eq!(ents.faces.values().any(|f| f.holes.len() == 1), !merge);
        }
    }

    /// Healing gives the same result every time.
    #[test]
    fn healing_is_deterministic() {
        let mut points = Vec::new();
        let mut polys: Vec<Vec<usize>> = Vec::new();
        // A fan of slightly jittered triangles that overlap and cross each other.
        for k in 0..12 {
            let a = k as f64 * 0.5;
            let base = points.len();
            points.push([a.cos(), a.sin(), 0.0]);
            points.push([2.0 * (a + 0.9).cos(), 2.0 * (a + 0.9).sin(), 0.0]);
            points.push([0.3 * a.sin(), 0.3 * a.cos(), (k % 3) as f64 * 0.2]);
            polys.push(vec![base, base + 1, base + 2]);
        }
        let polys: Vec<(&[usize], Option<usize>)> =
            polys.iter().map(|p| (p.as_slice(), None)).collect();
        let m = mesh(&points, &polys);
        let opts = MeshOptions {
            inches_per_unit: 1.0,
            y_up: false,
            merge_coplanar: true,
        };
        let a = to_entities(&m, &opts, &[]);
        let b = to_entities(&m, &opts, &[]);
        let dump = |e: &Entities| {
            let mut v: Vec<String> = e
                .vertices
                .values()
                .map(|v| format!("{:.9?}", v.pos))
                .collect();
            v.extend(
                e.faces
                    .values()
                    .map(|f| format!("{:?}", f.outer.coedges.len())),
            );
            v
        };
        assert_eq!(dump(&a), dump(&b));
        let errs = validate(&a, Checks::ALL);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// A polygon routed through one of its own vertices again splits into its simple
    /// parts; a two-vertex part (a spike) and a part turning the other way go.
    #[test]
    fn pinched_polygons_split_into_parts() {
        assert_eq!(
            super::simple_cycles(&[0, 1, 2, 3, 2, 4]),
            vec![vec![0, 1, 2, 4]]
        );
        assert_eq!(
            super::simple_cycles(&[0, 1, 2, 0, 3, 4]),
            vec![vec![0, 1, 2], vec![0, 3, 4]]
        );
        let points = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 1.0, 0.0),
            DVec3::new(1.0, 0.5, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(1.5, 0.6, 0.0),
        ];
        let p = super::Piece {
            vs: vec![0, 1, 2, 3, 4],
            normal: DVec3::Z,
            src: 0,
        };
        // Through 3 again after 5: the spike 3-5 goes.
        let out = super::reroute(&points, p, vec![0, 1, 2, 3, 5, 3, 4]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].vs, vec![0, 1, 2, 3, 4]);
    }

    /// The two sides of a fold, facing away from each other with planes a little
    /// apart (not coplanar by the plane test), overlap once exported as triangles:
    /// they are resolved like coplanar faces, so the overlap is covered once.
    #[test]
    fn folds_are_resolved() {
        let m = mesh(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.9, 0.5, 0.0008],
            ],
            &[(&[0, 1, 2, 3], Some(0)), (&[0, 3, 4], Some(1))],
        );
        let ents = import(&m, false);
        assert!((area(&ents) - 1.0).abs() < 1e-6, "{}", area(&ents));
        assert!(ents.faces.values().all(|f| f.normal.z > 0.99));
        assert!(ents.faces.values().all(|f| f.attrs.material == Some(10)));
    }

    /// A long face at a tiny angle to one it overlaps, with a corner just over the
    /// tolerance out of that face's plane: the corner moves into the plane and the
    /// overlap is resolved.
    #[test]
    fn corners_just_out_of_plane_snap_into_it() {
        let m = mesh(
            &[
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 100.0, 0.0],
                [0.0, 100.0, 0.0],
                [0.0, 10.0, 0.0],
                [90.0, 10.0, -0.00106],
            ],
            &[(&[0, 1, 2, 3], None), (&[0, 4, 5], None)],
        );
        let ents = import(&m, false);
        assert!((area(&ents) - 10_000.0).abs() < 1e-3, "{}", area(&ents));
        assert!(ents.vertices.values().all(|v| v.pos.z.abs() < 1e-9));
    }
}
