//! SketchUp's geometry behavior: sticky edges, automatic faces, splitting and healing.
//!
//! Behaviors marked "unverified" are from memory of SketchUp and must be checked during
//! the trial (`spec/parity/STATUS.md`).

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::arrangement::{self, Cell};
use crate::entities::{CoEdge, EdgeId, Entities, FaceAttrs, FaceId, Loop, VertexId};
use crate::geom::{Plane, point_in_polygon, point_segment, segment_segment, signed_area2};

/// Slack for plane queries of the spatial index: a face or edge coinciding with a plane
/// (within TOL, with normals equal to 1e-9) lies within this of it, far out from the
/// origin too.
pub(crate) const PLANE_SLACK: f64 = 4.0 * TOL;

/// Adds a straight line from `a` to `b` and returns the edges covering it, in order.
///
/// Like SketchUp:
/// - endpoints within tolerance of an existing vertex reuse it; an endpoint on an
///   edge splits that edge;
/// - edges crossing the line are split at the crossing, and so is the line;
/// - existing edges along the line are reused;
/// - a closed planar loop gets a face; a line across a face splits the face.
pub fn add_line(ents: &mut Entities, a: DVec3, b: DVec3) -> Vec<EdgeId> {
    insert_line(ents, a, b, true).0
}

/// `add_line` that only creates faces when `auto_face` is set (faces crossed by the
/// line are split either way). Returns the edges covering the line and, among them,
/// those that were created.
pub(crate) fn insert_line(
    ents: &mut Entities,
    a: DVec3,
    b: DVec3,
    auto_face: bool,
) -> (Vec<EdgeId>, Vec<EdgeId>) {
    if a.distance(b) <= TOL {
        return (Vec::new(), Vec::new());
    }
    ents.ensure_index();
    let va = vertex_at(ents, a);
    let vb = vertex_at(ents, b);
    if va == vb {
        return (Vec::new(), Vec::new());
    }
    let mut all = Vec::new();
    let mut created = Vec::new();
    add_piece(ents, va, vb, 0, &mut all, &mut created);
    update_faces(ents, &created, auto_face);
    (all, created)
}

/// How many times a piece of a line is checked again after its ends were snapped.
const PIECE_DEPTH: usize = 3;

/// Covers `u`-`v` with edges: splits what it crosses, runs through the vertices on it,
/// and checks each piece again (its ends were snapped to vertices or edges up to TOL
/// off the line, so the piece can graze an edge the whole line missed).
fn add_piece(
    ents: &mut Entities,
    u: VertexId,
    v: VertexId,
    depth: usize,
    all: &mut Vec<EdgeId>,
    created: &mut Vec<EdgeId>,
) {
    if depth > 0
        && let Some(e) = ents.edge_between(u, v)
    {
        all.push(e);
        return;
    }
    let chain = if depth < PIECE_DEPTH {
        chain_between(ents, u, v)
    } else {
        vec![u, v]
    };
    if chain.len() > 2 {
        for w in chain.windows(2) {
            add_piece(ents, w[0], w[1], depth + 1, all, created);
        }
        return;
    }
    match ents.edge_between(u, v) {
        Some(e) => all.push(e),
        None => {
            let e = ents.add_raw_edge(u, v, Default::default());
            all.push(e);
            created.push(e);
        }
    }
}

/// The vertices a line from `va` to `vb` runs through, in order (`va` and `vb`
/// included): edges crossing it are split at the crossing, and edges running along it
/// are split at its vertices.
fn chain_between(ents: &mut Entities, va: VertexId, vb: VertexId) -> Vec<VertexId> {
    let (pa, pb) = (ents.pos(va), ents.pos(vb));
    let len = pa.distance(pb);

    // Split edges that cross the line in both interiors.
    let crossing: Vec<(EdgeId, DVec3)> = ents
        .edges_near_segment(pa, pb, TOL)
        .into_iter()
        .filter_map(|id| {
            let e = &ents.edges[id];
            let (ps, pt) = (ents.pos(e.start), ents.pos(e.end));
            let (d, s, t) = segment_segment(pa, pb, ps, pt)?;
            let elen = ps.distance(pt);
            let interior = |u: f64, l: f64| u * l > TOL && (1.0 - u) * l > TOL;
            let p = ps + (pt - ps) * t;
            // Near an end of the line the crossing still counts when the crossed edge
            // passes more than TOL from that end: the end vertex is not snapped onto
            // the edge, and a steep line would graze the edge just before reaching it.
            let off_ends = p.distance(pa) > TOL && p.distance(pb) > TOL;
            (d <= TOL && (interior(s, len) || off_ends) && interior(t, elen)).then_some((id, p))
        })
        .collect();
    for (_, p) in crossing {
        // A crossing within tolerance of a vertex goes through that vertex. Every edge
        // passing within tolerance of the crossing is split there (the crossed edge, or
        // its piece if an earlier crossing split it, and edges a shallow crossing grazes).
        let w = ents.find_vertex(p).unwrap_or_else(|| ents.add_vertex(p));
        snap_edges_to(ents, w);
    }

    // Every vertex on the line, in order.
    let mut on_line: Vec<(f64, VertexId)> = ents
        .vertices_near_segment(pa, pb, TOL)
        .into_iter()
        .filter(|id| *id != va && *id != vb)
        .filter_map(|id| {
            let (d, t) = point_segment(ents.pos(id), pa, pb);
            (d <= TOL && t > 0.0 && t < 1.0).then_some((t, id))
        })
        .collect();
    on_line.sort_by(|x, y| x.0.total_cmp(&y.0));
    let chain: Vec<VertexId> = std::iter::once(va)
        .chain(on_line.into_iter().map(|(_, v)| v))
        .chain(std::iter::once(vb))
        .collect();
    // An existing edge running along the line within tolerance is split at the line's
    // vertices, so the line reuses its pieces instead of doubling it.
    for &w in &chain {
        snap_edges_to(ents, w);
    }
    chain
}

/// Splits every edge that passes within tolerance of vertex `w` (in its interior) at `w`.
fn snap_edges_to(ents: &mut Entities, w: VertexId) {
    let p = ents.pos(w);
    let near: Vec<EdgeId> = ents
        .edges_near(p, TOL)
        .into_iter()
        .filter(|&id| {
            let e = &ents.edges[id];
            e.start != w
                && e.end != w
                && point_segment(p, ents.pos(e.start), ents.pos(e.end)).0 <= TOL
        })
        .collect();
    for e in near {
        if can_split_at(ents, e, w) {
            split_edge_merging(ents, e, w);
        }
    }
}

/// Whether `e` can be split at `w` (which is not one of its ends).
fn can_split_at(ents: &Entities, e: EdgeId, w: VertexId) -> bool {
    ents.edges
        .get(e)
        .is_some_and(|edge| edge.start != w && edge.end != w)
}

/// Splits `e` at `w`; a half that duplicates an existing edge between the same vertices
/// is merged into it (its faces use the existing edge).
fn split_edge_merging(ents: &mut Entities, e: EdgeId, w: VertexId) {
    let (s0, s1) = (ents.edges[e].start, ents.edges[e].end);
    let twins = [ents.edge_between(s0, w), ents.edge_between(w, s1)];
    let (a, b) = ents.split_edge(e, w);
    for (half, twin) in [(a, twins[0]), (b, twins[1])] {
        if let Some(y) = twin {
            merge_edge_into(ents, half, y);
        }
    }
}

/// Replaces edge `x` by edge `y` (same two vertices) in every face, then removes `x`.
fn merge_edge_into(ents: &mut Entities, x: EdgeId, y: EdgeId) {
    let flip = ents.edges[x].start != ents.edges[y].start;
    let faces = std::mem::take(&mut ents.edges[x].faces);
    for f in faces {
        for l in ents.faces[f].loops_mut() {
            for c in &mut l.coedges {
                if c.edge == x {
                    *c = CoEdge {
                        edge: y,
                        reversed: c.reversed != flip,
                    };
                }
            }
        }
        if !ents.edges[y].faces.contains(&f) {
            ents.edges[y].faces.push(f);
        }
    }
    ents.remove_edge(x);
}

/// The vertex at `p`: an existing one within tolerance, a new one splitting the edge
/// `p` lies on, or a new free vertex.
fn vertex_at(ents: &mut Entities, p: DVec3) -> VertexId {
    if let Some(v) = ents.find_vertex(p) {
        return v;
    }
    let on_edge = ents.edges_near(p, TOL).into_iter().find_map(|id| {
        let e = &ents.edges[id];
        let (a, b) = (ents.pos(e.start), ents.pos(e.end));
        let (d, t) = point_segment(p, a, b);
        (d <= TOL).then_some((id, a + (b - a) * t))
    });
    match on_edge {
        Some((_, q)) => {
            // The foot on the edge may be within tolerance of a vertex `p` was not.
            let w = ents.find_vertex(q).unwrap_or_else(|| ents.add_vertex(q));
            snap_edges_to(ents, w);
            w
        }
        None => ents.add_vertex(p),
    }
}

/// Planes that may hold new faces next to `e`: planes of faces it lies on, and planes
/// spanned by `e` and each non-parallel edge at its ends.
fn candidate_planes(ents: &Entities, e: EdgeId, out: &mut Vec<Plane>) {
    let edge = &ents.edges[e];
    let (a, b) = (ents.pos(edge.start), ents.pos(edge.end));
    let dir = (b - a).normalize();
    let mut push = |p: Plane| {
        if !out.iter().any(|q| q.coincides(&p)) {
            out.push(p);
        }
    };
    let mid = (a + b) / 2.0;
    for f in ents.faces_near(mid, TOL) {
        if ents.face_contains(f, mid)
            && ents.face_plane(f).contains(a)
            && ents.face_plane(f).contains(b)
        {
            push(ents.face_plane(f));
        }
    }
    for v in [edge.start, edge.end] {
        for &e2 in &ents.vertices[v].edges {
            if e2 == e {
                continue;
            }
            let other = ents.pos(ents.edges[e2].other(v));
            let d2 = (other - ents.pos(v)).normalize();
            let n = dir.cross(d2);
            // Nearly collinear edges span no reliable plane.
            if n.length() > 1e-3 {
                push(Plane::new(n, a));
            }
        }
    }
}

/// A face's region saved before it is removed: loops in 3D.
struct Region {
    outer: Vec<DVec3>,
    holes: Vec<Vec<DVec3>>,
}

impl Region {
    fn of(ents: &Entities, f: FaceId) -> Self {
        let face = &ents.faces[f];
        Self {
            outer: ents.loop_points(&face.outer),
            holes: face.holes.iter().map(|l| ents.loop_points(l)).collect(),
        }
    }

    fn contains(&self, plane: &Plane, p: DVec3) -> bool {
        let proj = |l: &Vec<DVec3>| -> Vec<DVec2> { l.iter().map(|q| plane.project(*q)).collect() };
        let q = plane.project(p);
        point_in_polygon(q, &proj(&self.outer))
            && !self.holes.iter().any(|h| point_in_polygon(q, &proj(h)))
    }
}

fn cell_edges(cell: &Cell) -> Vec<EdgeId> {
    let mut v: Vec<EdgeId> = std::iter::once(&cell.outer)
        .chain(&cell.holes)
        .flat_map(|l| l.iter().map(|c| c.edge))
        .collect();
    v.sort();
    v.dedup();
    v
}

fn face_edges(ents: &Entities, f: FaceId) -> Vec<EdgeId> {
    let mut v: Vec<EdgeId> = ents.faces[f]
        .loops()
        .flat_map(|l| l.coedges.iter().map(|c| c.edge))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Whether a face on exactly the cell's edges exists, maybe in a plane that does not
/// quite coincide with the cell's (a loop planar only within tolerance, or a sliver
/// that fits many planes): the cell must not get a second face.
fn has_face_on(ents: &Entities, cell: &Cell) -> bool {
    let edges = cell_edges(cell);
    ents.edges[cell.outer[0].edge]
        .faces
        .iter()
        .any(|&g| face_edges(ents, g) == edges)
}

fn reversed(cycle: &[CoEdge]) -> Vec<CoEdge> {
    cycle
        .iter()
        .rev()
        .map(|c| CoEdge {
            edge: c.edge,
            reversed: !c.reversed,
        })
        .collect()
}

/// Creates a face from a cell, with the given front normal (along or against the
/// plane's normal).
pub(crate) fn face_from_cell(
    ents: &mut Entities,
    plane: &Plane,
    cell: &Cell,
    normal: DVec3,
    attrs: FaceAttrs,
) -> FaceId {
    let flip = normal.dot(plane.normal) < 0.0;
    let fix = |c: &Vec<CoEdge>| Loop {
        coedges: if flip { reversed(c) } else { c.clone() },
    };
    let holes = cell.holes.iter().map(fix).collect();
    ents.add_raw_face(fix(&cell.outer), holes, normal, attrs)
}

/// Whether a new face is planar about its own plane (through its first vertex) and its
/// loops wind properly there. Edges within tolerance of a cell's plane, or a sliver cell
/// of almost no area, can fail this; such cells get no face.
pub(crate) fn face_is_sound(ents: &Entities, f: FaceId) -> bool {
    let plane = ents.face_plane(f);
    let face = &ents.faces[f];
    if !face
        .loops()
        .all(|l| ents.loop_points(l).iter().all(|p| plane.contains(*p)))
    {
        return false;
    }
    let (outer, holes) = ents.face_polygons(f, &plane);
    signed_area2(&outer) > 0.0 && holes.iter().all(|h| signed_area2(h) < 0.0)
}

/// Front normal for a brand-new face (unverified rules):
/// - on the ground plane it faces down, as SketchUp does;
/// - otherwise it agrees with a neighboring face across a shared edge;
/// - otherwise its largest component is positive.
fn new_face_normal(ents: &Entities, plane: &Plane, cell: &Cell) -> DVec3 {
    let n = plane.normal;
    let on_ground = n.z.abs() > 1.0 - 1e-9
        && cell
            .outer
            .iter()
            .all(|c| ents.pos(ents.coedge_ends(*c).0).z.abs() <= TOL);
    if on_ground {
        return DVec3::NEG_Z;
    }
    for c in &cell.outer {
        for &g in &ents.edges[c.edge].faces {
            let theirs = ents.faces[g]
                .loops()
                .flat_map(|l| &l.coedges)
                .find(|x| x.edge == c.edge);
            if let Some(t) = theirs {
                // Consistent orientation: a shared edge runs opposite ways in the two loops.
                return if t.reversed == c.reversed { -n } else { n };
            }
        }
    }
    let largest = if n.x.abs() >= n.y.abs() && n.x.abs() >= n.z.abs() {
        n.x
    } else if n.y.abs() >= n.z.abs() {
        n.y
    } else {
        n.z
    };
    if largest < 0.0 { -n } else { n }
}

/// Creates faces for closed loops through `new_edges` (when `create` is set) and splits
/// faces they cross.
pub(crate) fn update_faces(ents: &mut Entities, new_edges: &[EdgeId], create: bool) {
    let mut planes = Vec::new();
    for &e in new_edges {
        if ents.edges.contains_key(e) {
            candidate_planes(ents, e, &mut planes);
        }
    }
    for plane in planes {
        // A cell can only touch a new edge that survives pruning of dangling edges, so
        // one with both ends on other edges of the plane: when there is none, the plane's
        // cells need not be computed.
        let in_plane = |e: EdgeId| {
            let edge = &ents.edges[e];
            plane.contains(ents.pos(edge.start)) && plane.contains(ents.pos(edge.end))
        };
        let anchored = |v: VertexId, e: EdgeId| {
            ents.vertices[v]
                .edges
                .iter()
                .any(|&x| x != e && in_plane(x))
        };
        let live = new_edges.iter().any(|&e| {
            ents.edges
                .get(e)
                .is_some_and(|edge| in_plane(e) && anchored(edge.start, e) && anchored(edge.end, e))
        });
        if !live {
            continue;
        }
        let edges = arrangement::edges_in_plane(ents, &plane);
        let cells = arrangement::cells(ents, &plane, &edges);
        let touches_new = |c: &Cell| cell_edges(c).iter().any(|e| new_edges.contains(e));

        // Faces in this plane that a new edge divides: replaced by their cells.
        let coplanar: Vec<FaceId> = ents
            .faces_near_plane(&plane, PLANE_SLACK)
            .into_iter()
            // As for edges: within tolerance of the plane everywhere (a plane spanned at
            // a vertex a hair off a face's plane is tilted by more than `coincides`
            // allows, and the face would get a twin in it).
            .filter(|&f| {
                ents.face_plane(f).coincides(&plane)
                    || ents.faces[f].loops().all(|l| {
                        l.coedges
                            .iter()
                            .all(|c| plane.contains(ents.pos(ents.coedge_ends(*c).0)))
                    })
            })
            .collect();
        let mut to_split: Vec<FaceId> = Vec::new();
        let mut to_create: Vec<usize> = Vec::new();
        for (i, cell) in cells.iter().enumerate().filter(|(_, c)| touches_new(c)) {
            match coplanar
                .iter()
                .find(|&&f| ents.face_contains(f, cell.sample))
            {
                Some(&f) if face_edges(ents, f) == cell_edges(cell) => {}
                Some(&f) => {
                    if !to_split.contains(&f) {
                        to_split.push(f);
                    }
                }
                None if create && !has_face_on(ents, cell) => to_create.push(i),
                None => {}
            }
        }
        for f in to_split {
            let region = Region::of(ents, f);
            let face = ents.remove_face(f).expect("face exists");
            for cell in cells.iter().filter(|c| region.contains(&plane, c.sample)) {
                if has_face_on(ents, cell) {
                    continue;
                }
                let g = face_from_cell(ents, &plane, cell, face.normal, face.attrs.clone());
                if !face_is_sound(ents, g) {
                    ents.remove_face(g);
                }
            }
        }
        for i in to_create {
            let normal = new_face_normal(ents, &plane, &cells[i]);
            let f = face_from_cell(ents, &plane, &cells[i], normal, FaceAttrs::default());
            if !face_is_sound(ents, f) {
                ents.remove_face(f);
            }
        }
    }
}

/// Erases an edge like SketchUp's Eraser:
/// - faces bounded by it are deleted, except that two coplanar faces on either side
///   merge into one;
/// - a vertex left with two collinear edges is healed into one edge (unverified);
/// - vertices left without edges are deleted.
pub fn erase_edge(ents: &mut Entities, e: EdgeId) {
    if !ents.edges.contains_key(e) {
        return;
    }
    ents.ensure_index();
    let edge = &ents.edges[e];
    let (v1, v2) = (edge.start, edge.end);
    let faces = edge.faces.clone();
    let merge = match faces.as_slice() {
        [f1, f2] if ents.face_plane(*f1).coincides(&ents.face_plane(*f2)) => Some((
            ents.face_plane(*f1),
            Region::of(ents, *f1),
            Region::of(ents, *f2),
            ents.faces[*f1].clone(),
        )),
        _ => None,
    };
    ents.remove_edge(e);
    if let Some((plane, r1, r2, keep)) = merge {
        let edges = arrangement::edges_in_plane(ents, &plane);
        for cell in arrangement::cells(ents, &plane, &edges) {
            let inside = r1.contains(&plane, cell.sample) || r2.contains(&plane, cell.sample);
            let taken = ents
                .faces_near(cell.sample, TOL)
                .into_iter()
                .any(|f| ents.face_contains(f, cell.sample));
            if inside && !taken {
                face_from_cell(ents, &plane, &cell, keep.normal, keep.attrs.clone());
            }
        }
    }
    for v in [v1, v2] {
        heal_vertex(ents, v);
        ents.remove_vertex_if_unused(v);
    }
}

/// Joins two collinear edges meeting at `v` (and nothing else) into one, when they bound
/// the same faces and have the same attributes.
pub(crate) fn heal_vertex(ents: &mut Entities, v: VertexId) {
    let Some(vert) = ents.vertices.get(v) else {
        return;
    };
    let [e1, e2] = vert.edges[..] else { return };
    let (a, b) = (ents.edges[e1].other(v), ents.edges[e2].other(v));
    let (pa, pb, pv) = (ents.pos(a), ents.pos(b), ents.pos(v));
    let (d, t) = point_segment(pv, pa, pb);
    if d > TOL || t <= 0.0 || t >= 1.0 || ents.edge_between(a, b).is_some() {
        return;
    }
    let mut f1 = ents.edges[e1].faces.clone();
    let mut f2 = ents.edges[e2].faces.clone();
    f1.sort();
    f2.sort();
    let (x1, x2) = (&ents.edges[e1].attrs, &ents.edges[e2].attrs);
    let same_attrs = x1.soft == x2.soft
        && x1.smooth == x2.smooth
        && x1.hidden == x2.hidden
        && x1.layer == x2.layer;
    if f1 != f2 || !same_attrs {
        return;
    }
    let attrs = ents.edges[e1].attrs.clone();
    // Loops as vertex sequences without `v`, rebuilt after swapping the edges.
    let loops: Vec<(FaceId, usize, Vec<VertexId>)> = f1
        .iter()
        .flat_map(|&f| {
            let face = &ents.faces[f];
            face.loops()
                .enumerate()
                .map(|(i, l)| {
                    let vs: Vec<VertexId> = ents
                        .loop_vertices(l)
                        .into_iter()
                        .filter(|&x| x != v)
                        .collect();
                    (f, i, vs)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for e in [e1, e2] {
        ents.edges[e].faces.clear();
        ents.unlink_edge(e);
    }
    ents.remove_vertex_if_unused(v);
    let joined = ents.add_raw_edge(a, b, attrs);
    for (f, i, vs) in loops {
        let coedges: Vec<CoEdge> = (0..vs.len())
            .filter_map(|k| {
                let (p, q) = (vs[k], vs[(k + 1) % vs.len()]);
                let e = ents.edge_between(p, q)?;
                Some(CoEdge {
                    edge: e,
                    reversed: ents.edges[e].start != p,
                })
            })
            .collect();
        let face = &mut ents.faces[f];
        let l = if i == 0 {
            &mut face.outer
        } else {
            &mut face.holes[i - 1]
        };
        l.coedges = coedges;
        if !ents.edges[joined].faces.contains(&f) {
            ents.edges[joined].faces.push(f);
        }
        ents.refresh_face_bounds(f);
    }
}

/// Adds a polyline through `points` as one curve and returns its edges. Edges it reuses
/// keep their old curve; pieces it creates join the new one. A closed planar polyline
/// gets a face like any other loop.
pub fn add_curve(
    ents: &mut Entities,
    points: &[DVec3],
    arc: Option<crate::entities::Arc>,
) -> Vec<EdgeId> {
    let curve = ents.curves.insert(crate::entities::Curve {
        arc,
        polygon: false,
    });
    let mut edges = Vec::new();
    for w in points.windows(2) {
        // Edges covering the segment that existed before it was added lie along it, so
        // only edges near it need remembering.
        ents.ensure_index();
        let before: std::collections::BTreeSet<EdgeId> = ents
            .edges_near_segment(w[0], w[1], 4.0 * TOL)
            .into_iter()
            .collect();
        for e in add_line(ents, w[0], w[1]) {
            if !before.contains(&e)
                && let Some(edge) = ents.edges.get_mut(e)
            {
                edge.attrs.curve = Some(curve);
            }
            edges.push(e);
        }
    }
    edges.retain(|e| ents.edges.contains_key(*e));
    if ents.curve_edges(curve).is_empty() {
        ents.curves.remove(curve);
    }
    edges
}

/// Erases every edge of a curve (SketchUp erases a curve as a whole).
pub fn erase_curve(ents: &mut Entities, c: crate::entities::CurveId) {
    for e in ents.curve_edges(c) {
        if ents.edges.contains_key(e) {
            erase_edge(ents, e);
        }
    }
    ents.curves.remove(c);
}
