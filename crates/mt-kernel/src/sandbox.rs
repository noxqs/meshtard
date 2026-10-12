//! Sandbox editing tools on a triangulated terrain (TIN): Smoove, Stamp, Drape, Add
//! Detail and Flip Edge (the Sandbox toolbar besides From Scratch and From Contours,
//! which are in [`crate::terrain`]).
//!
//! Terrain is the faces that are not walls (normal not level); heights are along +Z of
//! the entities' own coordinates (the blue axis of the open group). Faces made here are
//! triangles whose new edges are soft and smooth, like the TINs of `terrain`. All from
//! memory of SketchUp's Sandbox and its help pages (unverified):
//! - Smoove raises the vertices around a point (or the selected ones, with a falloff
//!   around them) by a height, fading out smoothly at the radius;
//! - Stamp cuts a level pad in the shape of a footprint into the terrain, joined to the
//!   terrain around it by a sloped band as wide as the offset;
//! - Drape projects edges straight down onto the terrain, splitting its faces;
//! - Add Detail splits each selected triangle into four (neighbors are split so the
//!   terrain stays triangles), or adds one vertex on a face or edge;
//! - Flip Edge swaps the diagonal of the two triangles sharing an edge.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{CoEdge, EdgeAttrs, EdgeId, Entities, FaceAttrs, FaceId, Loop, VertexId};
use crate::geom::{Plane, point_in_polygon, point_segment, signed_area2};

/// Faces whose normal's height is at most this are walls, not terrain.
const WALL: f64 = 1e-3;

/// Whether `f` is terrain (not a wall).
pub fn is_terrain(ents: &Entities, f: FaceId) -> bool {
    ents.faces.get(f).is_some_and(|x| x.normal.z.abs() > WALL)
}

fn plan(p: DVec3) -> DVec2 {
    p.truncate()
}

/// The edge between `a` and `b`, created (soft and smooth when `soft`) if missing.
fn edge_or_new(ents: &mut Entities, a: VertexId, b: VertexId, soft: bool) -> EdgeId {
    if let Some(e) = ents.edge_between(a, b) {
        return e;
    }
    let attrs = EdgeAttrs {
        soft,
        smooth: soft,
        ..Default::default()
    };
    ents.add_raw_edge(a, b, attrs)
}

/// Adds the face `vs` (in loop order; its normal follows the winding), creating
/// missing edges (soft and smooth when `soft`). `None` when degenerate.
fn add_face(ents: &mut Entities, vs: &[VertexId], attrs: &FaceAttrs, soft: bool) -> Option<FaceId> {
    let pts: Vec<DVec3> = vs.iter().map(|&v| ents.pos(v)).collect();
    let normal = Plane::from_polygon(&pts)?.normal;
    let coedges = (0..vs.len())
        .map(|i| {
            let (s, t) = (vs[i], vs[(i + 1) % vs.len()]);
            let edge = edge_or_new(ents, s, t, soft);
            CoEdge {
                edge,
                reversed: ents.edges[edge].start != s,
            }
        })
        .collect();
    let attrs = FaceAttrs {
        uv: None,
        ..attrs.clone()
    };
    Some(ents.add_raw_face(Loop { coedges }, Vec::new(), normal, attrs))
}

/// Twice the area of `a b c` divided by its longest side: the triangle's smallest
/// height (negative when clockwise).
fn tri_height(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    let area = (b - a).perp_dot(c - a);
    let longest = (b - a).length().max((c - b).length()).max((a - c).length());
    if longest <= 0.0 { 0.0 } else { area / longest }
}

/// Whether `p` lies in the closed triangle `a b c` (either winding).
fn in_triangle(p: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    let (b, c) = if (b - a).perp_dot(c - a) < 0.0 {
        (c, b)
    } else {
        (b, c)
    };
    let s = |u: DVec2, v: DVec2| (v - u).perp_dot(p - u);
    let eps = -1e-12 * (1.0 + (b - a).length_squared() + (c - a).length_squared());
    s(a, b) >= eps && s(b, c) >= eps && s(c, a) >= eps
}

/// Ear clipping of a counter-clockwise polygon that may have collinear vertices and
/// repeated points (a hole bridged into it). Ears are taken best first (largest
/// smallest height relative to size) and must be at least TOL high. `None` when stuck.
fn ear_clip(pts: &[DVec2]) -> Option<Vec<[usize; 3]>> {
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::new();
    while idx.len() > 3 {
        let n = idx.len();
        let mut best: Option<(f64, usize)> = None;
        for k in 0..n {
            let (i0, i1, i2) = (idx[(k + n - 1) % n], idx[k], idx[(k + 1) % n]);
            let (a, b, c) = (pts[i0], pts[i1], pts[i2]);
            let h = tri_height(a, b, c);
            if h <= TOL {
                continue;
            }
            let blocked = idx.iter().enumerate().any(|(j, &q)| {
                if j == k || j == (k + n - 1) % n || j == (k + 1) % n {
                    return false;
                }
                let p = pts[q];
                let same = |x: DVec2| x.distance_squared(p) <= 1e-18;
                !(same(a) || same(b) || same(c)) && in_triangle(p, a, b, c)
            });
            if blocked {
                continue;
            }
            let size = (b - a).length().max((c - b).length()).max((a - c).length());
            let quality = h / size;
            if best.is_none_or(|(q, _)| quality > q + 1e-12) {
                best = Some((quality, k));
            }
        }
        let (_, k) = best?;
        out.push([idx[(k + n - 1) % n], idx[k], idx[(k + 1) % n]]);
        idx.remove(k);
    }
    if idx.len() == 3 {
        if tri_height(pts[idx[0]], pts[idx[1]], pts[idx[2]]) <= TOL {
            return None;
        }
        out.push([idx[0], idx[1], idx[2]]);
    }
    Some(out)
}

/// Splits a face without holes into triangles along new soft and smooth edges.
/// Returns the triangles (the face itself when it is one), or `None` for a face with
/// holes or one the triangulation cannot handle (left as it is).
pub fn triangulate_face(ents: &mut Entities, f: FaceId) -> Option<Vec<FaceId>> {
    let face = ents.faces.get(f)?;
    if !face.holes.is_empty() {
        return None;
    }
    let vs = ents.loop_vertices(&face.outer);
    if vs.len() == 3 {
        return Some(vec![f]);
    }
    let plane = ents.face_plane(f);
    let pts: Vec<DVec2> = vs.iter().map(|&v| plane.project(ents.pos(v))).collect();
    if signed_area2(&pts) <= 0.0 {
        return None;
    }
    let tris = ear_clip(&pts)?;
    let attrs = face.attrs.clone();
    ents.remove_face(f);
    Some(
        tris.into_iter()
            .filter_map(|t| add_face(ents, &t.map(|i| vs[i]), &attrs, true))
            .collect(),
    )
}

/// Moves vertices and gives the faces using them their new normals.
fn move_vertices(ents: &mut Entities, moves: &[(VertexId, DVec3)]) {
    ents.set_vertex_positions(moves);
    let faces: BTreeSet<FaceId> = moves
        .iter()
        .flat_map(|(v, _)| ents.vertices[*v].edges.iter())
        .flat_map(|e| ents.edges[*e].faces.iter().copied())
        .collect();
    for f in faces {
        if let Some(p) = Plane::from_polygon(&ents.loop_points(&ents.faces[f].outer)) {
            ents.faces[f].normal = p.normal;
        }
    }
}

/// Faces using `v`.
fn vertex_faces(ents: &Entities, v: VertexId) -> BTreeSet<FaceId> {
    ents.vertices[v]
        .edges
        .iter()
        .flat_map(|e| ents.edges[*e].faces.iter().copied())
        .collect()
}

/// Smoove's falloff: 1 at the center, 0 at the radius, smooth at both (`t` is the
/// distance over the radius).
pub fn falloff(t: f64) -> f64 {
    if t >= 1.0 {
        0.0
    } else {
        0.5 * (1.0 + (std::f64::consts::PI * t.max(0.0)).cos())
    }
}

/// The height change Smoove gives each vertex: around `center` (plan) or, when
/// `selected` is not empty, the selected vertices fully and the others by their plan
/// distance to the nearest selected one. Only terrain vertices (all their faces are
/// terrain, at least one face) within `radius` move.
pub fn smoove_offsets(
    ents: &Entities,
    center: DVec3,
    selected: &BTreeSet<VertexId>,
    radius: f64,
    height: f64,
) -> BTreeMap<VertexId, f64> {
    let sources: Vec<DVec2> = if selected.is_empty() {
        vec![plan(center)]
    } else {
        selected
            .iter()
            .filter_map(|v| ents.vertices.get(*v))
            .map(|v| plan(v.pos))
            .collect()
    };
    let mut out = BTreeMap::new();
    if radius <= TOL || height == 0.0 {
        return out;
    }
    for (id, v) in &ents.vertices {
        let faces = vertex_faces(ents, id);
        if faces.is_empty() || !faces.iter().all(|&f| is_terrain(ents, f)) {
            continue;
        }
        let w = if selected.contains(&id) {
            1.0
        } else {
            let d = sources
                .iter()
                .map(|s| s.distance(plan(v.pos)))
                .fold(f64::INFINITY, f64::min);
            falloff(d / radius)
        };
        if w > 0.0 {
            out.insert(id, height * w);
        }
    }
    out
}

/// Sandbox Smoove: raises the terrain by `height` (lowers when negative) around
/// `center` with a smooth falloff to `radius`, or around the selected vertices (see
/// [`smoove_offsets`]). Faces of moved vertices that are not triangles are split into
/// triangles first (a face that cannot be keeps its vertices still). Returns how many
/// vertices moved.
pub fn smoove(
    ents: &mut Entities,
    center: DVec3,
    selected: &BTreeSet<VertexId>,
    radius: f64,
    height: f64,
) -> usize {
    let mut offsets = smoove_offsets(ents, center, selected, radius, height);
    let faces: BTreeSet<FaceId> = offsets
        .keys()
        .flat_map(|&v| vertex_faces(ents, v))
        .collect();
    for f in faces {
        if triangulate_face(ents, f).is_none() {
            let vs = ents.loop_vertices(&ents.faces[f].outer);
            for v in vs {
                offsets.remove(&v);
            }
        }
    }
    let moves: Vec<(VertexId, DVec3)> = offsets
        .iter()
        .map(|(&v, &dz)| (v, ents.pos(v) + DVec3::Z * dz))
        .collect();
    move_vertices(ents, &moves);
    moves.len()
}

// ------------------------------------------------------------------ Drape

/// Whether plan point `q` lies on face `f` seen from above (boundary included).
fn covers(ents: &Entities, f: FaceId, q: DVec2) -> bool {
    let face = &ents.faces[f];
    let outer: Vec<DVec2> = ents
        .loop_points(&face.outer)
        .into_iter()
        .map(plan)
        .collect();
    let near_edge = |poly: &[DVec2]| {
        (0..poly.len()).any(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            point_segment(q.extend(0.0), a.extend(0.0), b.extend(0.0)).0 <= TOL
        })
    };
    if near_edge(&outer) {
        return true;
    }
    if !point_in_polygon(q, &outer) {
        return false;
    }
    face.holes.iter().all(|h| {
        let hole: Vec<DVec2> = ents.loop_points(h).into_iter().map(plan).collect();
        near_edge(&hole) || !point_in_polygon(q, &hole)
    })
}

/// The height of the highest terrain face over plan point `q`.
pub fn height_at(ents: &Entities, q: DVec2) -> Option<f64> {
    let mut best: Option<f64> = None;
    for (f, face) in &ents.faces {
        if face.normal.z.abs() <= WALL {
            continue;
        }
        let b = ents.face_bounds(f);
        if q.x < b.min.x - TOL || q.x > b.max.x + TOL || q.y < b.min.y - TOL || q.y > b.max.y + TOL
        {
            continue;
        }
        if !covers(ents, f, q) {
            continue;
        }
        let pl = ents.face_plane(f);
        let z = (pl.d - pl.normal.x * q.x - pl.normal.y * q.y) / pl.normal.z;
        if best.is_none_or(|h| z > h) {
            best = Some(z);
        }
    }
    best
}

/// The points where plan segment `a b` meets the terrain's edges and vertices, plus
/// its ends lifted onto the terrain, in order along it (`None`: off the terrain).
fn drape_points(ents: &Entities, a: DVec2, b: DVec2) -> Vec<(f64, Option<DVec3>)> {
    let len = a.distance(b);
    let d = b - a;
    let mut pts: Vec<(f64, Option<DVec3>)> = Vec::new();
    let edges: BTreeSet<EdgeId> = ents
        .faces
        .iter()
        .filter(|(_, f)| f.normal.z.abs() > WALL)
        .flat_map(|(_, f)| f.loops().flat_map(|l| l.coedges.iter().map(|c| c.edge)))
        .collect();
    let along = |p: DVec2| (p - a).dot(d) / (len * len);
    let on_line = |p: DVec2| {
        let t = along(p);
        (-TOL / len..=1.0 + TOL / len).contains(&t) && (a + d * t).distance(p) <= TOL
    };
    for e in edges {
        let edge = &ents.edges[e];
        let (p, q) = (ents.pos(edge.start), ents.pos(edge.end));
        for v in [p, q] {
            if on_line(plan(v)) {
                pts.push((along(plan(v)).clamp(0.0, 1.0), Some(v)));
            }
        }
        let r = plan(q) - plan(p);
        let den = d.perp_dot(r);
        if den.abs() <= 1e-12 * len * r.length() {
            continue;
        }
        let w = plan(p) - a;
        let t = w.perp_dot(r) / den;
        let s = w.perp_dot(d) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&s) {
            pts.push((t, Some(p.lerp(q, s))));
        }
    }
    for (t, end) in [(0.0, a), (1.0, b)] {
        pts.push((t, height_at(ents, end).map(|z| end.extend(z))));
    }
    pts.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, Option<DVec3>)> = Vec::new();
    for (t, p) in pts {
        match out.last_mut() {
            Some(last) if (t - last.0) * len <= TOL => {
                if last.1.is_none() {
                    last.1 = p;
                }
            }
            _ => out.push((t, p)),
        }
    }
    out
}

/// Sandbox Drape: projects each segment straight down onto the terrain (its highest
/// faces) as edges that split the faces they cross. Parts off the terrain are left
/// out. Returns the edges made or reused, in order.
pub fn drape(ents: &mut Entities, segments: &[(DVec3, DVec3)]) -> Vec<EdgeId> {
    let mut out = Vec::new();
    for &(a, b) in segments {
        let (a, b) = (plan(a), plan(b));
        if a.distance(b) <= TOL {
            continue;
        }
        let pts = drape_points(ents, a, b);
        let mut pieces = Vec::new();
        for w in pts.windows(2) {
            let (Some(p), Some(q)) = (w[0].1, w[1].1) else {
                continue;
            };
            let mid = a.lerp(b, 0.5 * (w[0].0 + w[1].0));
            if height_at(ents, mid).is_some() {
                pieces.push((p, q));
            }
        }
        for (p, q) in pieces {
            for e in crate::add_line(ents, p, q) {
                if !out.contains(&e) {
                    out.push(e);
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ Stamp

/// Why Stamp could not be made.
#[derive(Debug, Clone, PartialEq)]
pub enum StampError {
    /// The footprint has no area.
    NoFootprint,
    /// The offset must be more than zero.
    NoOffset,
    /// Part of the footprint or its offset is not over the terrain.
    OffTerrain,
    /// The terrain could not be rebuilt around the footprint.
    Failed,
}

/// The polygon counter-clockwise, without repeated points or straight corners.
fn clean_polygon(poly: &[DVec2]) -> Vec<DVec2> {
    let mut pts: Vec<DVec2> = Vec::new();
    for &p in poly {
        if pts.last().is_none_or(|l| l.distance(p) > TOL) {
            pts.push(p);
        }
    }
    while pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) <= TOL {
        pts.pop();
    }
    let mut changed = true;
    while changed && pts.len() >= 3 {
        changed = false;
        for i in 0..pts.len() {
            let n = pts.len();
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            if (b - a).perp_dot(c - a).abs() <= TOL * a.distance(c) {
                pts.remove(i);
                changed = true;
                break;
            }
        }
    }
    if signed_area2(&pts) < 0.0 {
        pts.reverse();
    }
    pts
}

/// The counter-clockwise polygon grown outwards by `d` (mitered corners, limited to
/// four times `d`).
pub fn offset_polygon(poly: &[DVec2], d: f64) -> Vec<DVec2> {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b, c) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            let out = |u: DVec2, v: DVec2| {
                let t = (v - u).normalize_or_zero();
                DVec2::new(t.y, -t.x)
            };
            let (n1, n2) = (out(a, b), out(b, c));
            let m = n1 + n2;
            let k = 1.0 + n1.dot(n2);
            let miter = if k <= 1e-9 { n1 } else { m / k };
            let miter = if miter.length() > 4.0 {
                miter.normalize() * 4.0
            } else {
                miter
            };
            b + miter * d
        })
        .collect()
}

/// The footprint of a stamp object: the outer loop (plan) of its lowest-looking face
/// (most downward, then largest), or the convex hull of its vertices when it has no
/// face; and its lowest height. Points are taken through `xf`.
pub fn footprint(ents: &Entities, xf: glam::DMat4) -> Option<(Vec<DVec2>, f64)> {
    let pts: Vec<DVec3> = ents
        .vertices
        .values()
        .map(|v| xf.transform_point3(v.pos))
        .collect();
    let low = pts.iter().map(|p| p.z).reduce(f64::min)?;
    let mut best: Option<(f64, f64, Vec<DVec2>)> = None;
    for f in ents.faces.values() {
        let loop_pts: Vec<DVec3> = ents
            .loop_points(&f.outer)
            .into_iter()
            .map(|p| xf.transform_point3(p))
            .collect();
        let Some(pl) = Plane::from_polygon(&loop_pts) else {
            continue;
        };
        if pl.normal.z.abs() <= WALL {
            continue;
        }
        let poly: Vec<DVec2> = loop_pts.into_iter().map(plan).collect();
        let area = signed_area2(&poly).abs();
        let down = -pl.normal.z;
        let better = best.as_ref().is_none_or(|(bd, ba, _)| {
            down > *bd + 1e-6 || ((down - *bd).abs() <= 1e-6 && area > *ba)
        });
        if better {
            best = Some((down, area, poly));
        }
    }
    let poly = match best {
        Some((_, _, p)) => p,
        None => convex_hull(pts.iter().map(|p| plan(*p)).collect()),
    };
    let poly = clean_polygon(&poly);
    (poly.len() >= 3).then_some((poly, low))
}

/// Convex hull (counter-clockwise, monotone chain).
fn convex_hull(mut pts: Vec<DVec2>) -> Vec<DVec2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| a.distance(*b) <= TOL);
    if pts.len() < 3 {
        return pts;
    }
    let mut hull: Vec<DVec2> = Vec::new();
    let order: Vec<DVec2> = pts
        .iter()
        .chain(pts.iter().rev().skip(1))
        .copied()
        .collect();
    let mut lower_len = 0;
    for (i, p) in order.into_iter().enumerate() {
        if i == pts.len() {
            lower_len = hull.len();
        }
        let floor = if i >= pts.len() { lower_len } else { 1 };
        while hull.len() > floor {
            let (a, b) = (hull[hull.len() - 2], hull[hull.len() - 1]);
            if (b - a).perp_dot(p - a) <= 0.0 {
                hull.pop();
            } else {
                break;
            }
        }
        hull.push(p);
    }
    hull.pop();
    hull
}

/// Distance from plan point `q` to the closed polygon's outline.
fn outline_distance(poly: &[DVec2], q: DVec2) -> f64 {
    (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            point_segment(q.extend(0.0), a.extend(0.0), b.extend(0.0)).0
        })
        .fold(f64::INFINITY, f64::min)
}

/// Whether plan segments `a b` and `c d` cross or touch.
fn segments_meet(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    point_segment_2d(a, b, c, d) <= 1e-9
}

fn point_segment_2d(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> f64 {
    crate::geom::segment_segment(a.extend(0.0), b.extend(0.0), c.extend(0.0), d.extend(0.0))
        .map_or_else(
            || {
                [(a, c, d), (b, c, d), (c, a, b), (d, a, b)]
                    .iter()
                    .map(|(p, u, v)| point_segment(p.extend(0.0), u.extend(0.0), v.extend(0.0)).0)
                    .fold(f64::INFINITY, f64::min)
            },
            |(dist, _, _)| dist,
        )
}

/// Sandbox Stamp: a level pad in the shape of `footprint` (plan polygon) at `height`,
/// with the terrain within `offset` around it removed and rebuilt as a sloped band of
/// triangles from the pad's outline to the offset outline draped on the terrain.
/// Nothing changes on error. Returns the pad face.
pub fn stamp(
    ents: &mut Entities,
    footprint: &[DVec2],
    offset: f64,
    height: f64,
) -> Result<FaceId, StampError> {
    let pad = clean_polygon(footprint);
    if pad.len() < 3 || signed_area2(&pad).abs() <= TOL * TOL {
        return Err(StampError::NoFootprint);
    }
    if offset <= TOL {
        return Err(StampError::NoOffset);
    }
    let ring = offset_polygon(&pad, offset);
    if ring.iter().any(|&q| height_at(ents, q).is_none()) {
        return Err(StampError::OffTerrain);
    }
    let mut w = ents.clone();
    let segs: Vec<(DVec3, DVec3)> = (0..ring.len())
        .map(|i| (ring[i].extend(0.0), ring[(i + 1) % ring.len()].extend(0.0)))
        .collect();
    drape(&mut w, &segs);

    // Terrain inside the ring goes.
    let inside: Vec<FaceId> = w
        .faces
        .keys()
        .filter(|&f| is_terrain(&w, f))
        .filter(|&f| {
            let s = plan(crate::transform::face_sample(&w, f));
            point_in_polygon(s, &ring) && outline_distance(&ring, s) > TOL
        })
        .collect();
    if inside.is_empty() {
        return Err(StampError::Failed);
    }
    let mut touched: BTreeSet<EdgeId> = BTreeSet::new();
    for f in inside {
        if let Some(face) = w.remove_face(f) {
            touched.extend(face.loops().flat_map(|l| l.coedges.iter().map(|c| c.edge)));
        }
    }
    let mut chain: Vec<EdgeId> = Vec::new();
    for e in touched {
        let edge = &w.edges[e];
        let (a, b) = (w.pos(edge.start), w.pos(edge.end));
        let on_ring = outline_distance(&ring, plan(a.lerp(b, 0.5))) <= 4.0 * TOL;
        if on_ring {
            chain.push(e);
        } else if edge.faces.is_empty() {
            let (s, t) = (edge.start, edge.end);
            w.unlink_edge(e);
            w.remove_vertex_if_unused(s);
            w.remove_vertex_if_unused(t);
        }
    }
    let outer = order_chain(&w, &chain).ok_or(StampError::Failed)?;

    // The pad, then the band between it and the ring.
    let pad_vs: Vec<VertexId> = pad.iter().map(|q| w.add_vertex(q.extend(height))).collect();
    let pad_face =
        add_face(&mut w, &pad_vs, &FaceAttrs::default(), false).ok_or(StampError::Failed)?;
    if w.faces[pad_face].normal.z < 0.0 {
        crate::transform::flip_face(&mut w, pad_face);
    }
    let merged = bridge(&w, &outer, &pad_vs).ok_or(StampError::Failed)?;
    let pts: Vec<DVec2> = merged.iter().map(|&v| plan(w.pos(v))).collect();
    let tris = ear_clip(&pts).ok_or(StampError::Failed)?;
    for t in tris {
        add_face(&mut w, &t.map(|i| merged[i]), &FaceAttrs::default(), true)
            .ok_or(StampError::Failed)?;
    }
    w.invalidate_index();
    *ents = w;
    Ok(pad_face)
}

/// The vertices of edges forming one closed chain, in order, counter-clockwise in plan.
fn order_chain(ents: &Entities, chain: &[EdgeId]) -> Option<Vec<VertexId>> {
    let mut next: BTreeMap<VertexId, Vec<VertexId>> = BTreeMap::new();
    for &e in chain {
        let edge = &ents.edges[e];
        next.entry(edge.start).or_default().push(edge.end);
        next.entry(edge.end).or_default().push(edge.start);
    }
    if next.values().any(|n| n.len() != 2) {
        return None;
    }
    let (&start, _) = next.iter().next()?;
    let mut out = vec![start];
    let mut prev = start;
    let mut cur = next[&start][0];
    while cur != start {
        out.push(cur);
        let n = &next[&cur];
        let nx = if n[0] == prev { n[1] } else { n[0] };
        prev = cur;
        cur = nx;
        if out.len() > chain.len() {
            return None;
        }
    }
    if out.len() != chain.len() {
        return None;
    }
    let pts: Vec<DVec2> = out.iter().map(|&v| plan(ents.pos(v))).collect();
    if signed_area2(&pts) < 0.0 {
        out.reverse();
    }
    Some(out)
}

/// One polygon from `outer` (counter-clockwise) with the hole `hole` (counter-clockwise,
/// inside it) joined by a bridge from the hole's rightmost vertex to the nearest outer
/// vertex it can see; the bridge's ends appear twice.
fn bridge(ents: &Entities, outer: &[VertexId], hole: &[VertexId]) -> Option<Vec<VertexId>> {
    let p = |v: VertexId| plan(ents.pos(v));
    let h = (0..hole.len()).max_by(|&i, &j| {
        p(hole[i])
            .x
            .total_cmp(&p(hole[j]).x)
            .then(p(hole[j]).y.total_cmp(&p(hole[i]).y))
    })?;
    let hp = p(hole[h]);
    let segs: Vec<(DVec2, DVec2, VertexId, VertexId)> = [outer, hole]
        .iter()
        .flat_map(|l| (0..l.len()).map(|i| (l[i], l[(i + 1) % l.len()])))
        .map(|(a, b)| (p(a), p(b), a, b))
        .collect();
    let mut order: Vec<usize> = (0..outer.len()).collect();
    order.sort_by(|&i, &j| {
        hp.distance(p(outer[i]))
            .total_cmp(&hp.distance(p(outer[j])))
    });
    let o = order.into_iter().find(|&i| {
        let op = p(outer[i]);
        segs.iter().all(|&(a, b, va, vb)| {
            let touches = [va, vb].contains(&outer[i]) || [va, vb].contains(&hole[h]);
            touches || !segments_meet(hp, op, a, b)
        })
    })?;
    let mut out: Vec<VertexId> = outer[..=o].to_vec();
    // The hole runs clockwise in the merged polygon.
    for k in 0..=hole.len() {
        out.push(hole[(h + hole.len() - k % hole.len()) % hole.len()]);
    }
    out.push(outer[o]);
    out.extend(&outer[o + 1..]);
    Some(out)
}

// ------------------------------------------------------------------ Add Detail

/// Sandbox Add Detail on selected faces: each terrain triangle (other faces are split
/// into triangles first) becomes four through the midpoints of its edges; neighbors
/// that got a midpoint are split into triangles too. Returns the new triangles.
pub fn add_detail(ents: &mut Entities, faces: &[FaceId]) -> Vec<FaceId> {
    let mut tris: BTreeSet<FaceId> = BTreeSet::new();
    for &f in faces {
        if is_terrain(ents, f)
            && let Some(ts) = triangulate_face(ents, f)
        {
            tris.extend(ts);
        }
    }
    let corners: Vec<(FaceId, [VertexId; 3])> = tris
        .iter()
        .map(|&f| {
            let vs = ents.loop_vertices(&ents.faces[f].outer);
            (f, [vs[0], vs[1], vs[2]])
        })
        .collect();
    let edges: BTreeSet<EdgeId> = tris
        .iter()
        .flat_map(|&f| ents.faces[f].outer.coedges.iter().map(|c| c.edge))
        .collect();
    let key = |a: VertexId, b: VertexId| if a < b { (a, b) } else { (b, a) };
    let mut mids: BTreeMap<(VertexId, VertexId), VertexId> = BTreeMap::new();
    let mut neighbors: BTreeSet<FaceId> = BTreeSet::new();
    for e in edges {
        let edge = ents.edges[e].clone();
        let m = ents.add_vertex(ents.pos(edge.start).lerp(ents.pos(edge.end), 0.5));
        neighbors.extend(edge.faces.iter().copied().filter(|f| !tris.contains(f)));
        ents.split_edge(e, m);
        mids.insert(key(edge.start, edge.end), m);
    }
    let mut out = Vec::new();
    for (f, [a, b, c]) in corners {
        let attrs = ents.faces[f].attrs.clone();
        let (ab, bc, ca) = (mids[&key(a, b)], mids[&key(b, c)], mids[&key(c, a)]);
        ents.remove_face(f);
        for t in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            out.extend(add_face(ents, &t, &attrs, true));
        }
    }
    for f in neighbors {
        if is_terrain(ents, f) {
            triangulate_face(ents, f);
        }
    }
    out
}

/// Where Add Detail puts one vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DetailAt {
    Face(FaceId),
    Edge(EdgeId),
}

/// Sandbox Add Detail clicked on a face or edge: a vertex at `p` (on it) joined to the
/// corners of the triangles around it, then moved up by `dz`. A point at an existing
/// vertex moves that vertex. Returns the vertex.
pub fn add_detail_vertex(ents: &mut Entities, at: DetailAt, p: DVec3, dz: f64) -> Option<VertexId> {
    let v = match at {
        DetailAt::Edge(e) => split_at(ents, e, p)?,
        DetailAt::Face(f) => {
            if !is_terrain(ents, f) {
                return None;
            }
            let tris = triangulate_face(ents, f)?;
            let &t = tris.iter().find(|&&t| {
                let vs = ents.loop_vertices(&ents.faces[t].outer);
                let [a, b, c] = [0, 1, 2].map(|i| plan(ents.pos(vs[i])));
                in_triangle(plan(p), a, b, c)
            })?;
            let vs = ents.loop_vertices(&ents.faces[t].outer);
            if let Some(&near) = vs.iter().find(|&&v| ents.pos(v).distance(p) <= TOL) {
                near
            } else if let Some(e) = ents.faces[t]
                .outer
                .coedges
                .iter()
                .map(|c| c.edge)
                .find(|&e| {
                    let edge = &ents.edges[e];
                    point_segment(p, ents.pos(edge.start), ents.pos(edge.end)).0 <= TOL
                })
            {
                split_at(ents, e, p)?
            } else {
                let attrs = ents.faces[t].attrs.clone();
                let pl = ents.face_plane(t);
                let on = p - pl.normal * pl.distance(p);
                let v = ents.add_vertex(on);
                ents.remove_face(t);
                for (a, b) in [(vs[0], vs[1]), (vs[1], vs[2]), (vs[2], vs[0])] {
                    add_face(ents, &[a, b, v], &attrs, true);
                }
                v
            }
        }
    };
    if dz != 0.0 {
        move_vertices(ents, &[(v, ents.pos(v) + DVec3::Z * dz)]);
    }
    Some(v)
}

/// Splits edge `e` at the point of it nearest `p` (or returns the end vertex there),
/// splitting its terrain faces into triangles.
fn split_at(ents: &mut Entities, e: EdgeId, p: DVec3) -> Option<VertexId> {
    let edge = ents.edges.get(e)?.clone();
    let (a, b) = (ents.pos(edge.start), ents.pos(edge.end));
    let (_, t) = point_segment(p, a, b);
    let q = a.lerp(b, t.clamp(0.0, 1.0));
    if q.distance(a) <= TOL {
        return Some(edge.start);
    }
    if q.distance(b) <= TOL {
        return Some(edge.end);
    }
    let v = ents.add_vertex(q);
    ents.split_edge(e, v);
    for f in edge.faces {
        if is_terrain(ents, f) {
            triangulate_face(ents, f);
        }
    }
    Some(v)
}

// ------------------------------------------------------------------ Flip Edge

/// The edge of face `f` nearest `p` that Flip Edge can flip, if any.
pub fn flippable_edge_near(ents: &Entities, f: FaceId, p: DVec3) -> Option<EdgeId> {
    let face = ents.faces.get(f)?;
    face.outer
        .coedges
        .iter()
        .map(|c| c.edge)
        .filter(|&e| flip_quad(ents, e).is_some())
        .min_by(|&x, &y| {
            let d = |e: EdgeId| {
                let edge = &ents.edges[e];
                point_segment(p, ents.pos(edge.start), ents.pos(edge.end)).0
            };
            d(x).total_cmp(&d(y))
        })
}

/// The quad around edge `e`: `(a, d, b, c)` counter-clockwise, `a b` the edge, and the
/// two faces, when `e` is shared by exactly two triangles and the other diagonal `c d`
/// gives two sound triangles facing the same way.
fn flip_quad(ents: &Entities, e: EdgeId) -> Option<([VertexId; 4], [FaceId; 2])> {
    let edge = ents.edges.get(e)?;
    let [f1, f2] = edge.faces[..] else {
        return None;
    };
    let tri = |f: FaceId| {
        let face = &ents.faces[f];
        (face.holes.is_empty() && face.outer.coedges.len() == 3)
            .then(|| ents.loop_vertices(&face.outer))
    };
    let (t1, t2) = (tri(f1)?, tri(f2)?);
    // Rotate f1 so that it runs a -> b -> c.
    let i = t1.iter().position(|&v| v == edge.start)?;
    let (a, b, c) = if t1[(i + 1) % 3] == edge.end {
        (edge.start, edge.end, t1[(i + 2) % 3])
    } else {
        (edge.end, edge.start, t1[(i + 1) % 3])
    };
    let d = *t2.iter().find(|&&v| v != a && v != b)?;
    // f2 must run b -> a (the faces agree).
    let j = t2.iter().position(|&v| v == b)?;
    if t2[(j + 1) % 3] != a || c == d || ents.edge_between(c, d).is_some() {
        return None;
    }
    let n = (ents.faces[f1].normal + ents.faces[f2].normal).normalize_or_zero();
    if n == DVec3::ZERO {
        return None;
    }
    let pl = Plane::new(n, ents.pos(a));
    let q = |v: VertexId| pl.project(ents.pos(v));
    for [x, y, z] in [[a, d, c], [d, b, c]] {
        if tri_height(q(x), q(y), q(z)) <= TOL {
            return None;
        }
    }
    Some(([a, d, b, c], [f1, f2]))
}

/// Sandbox Flip Edge: the two triangles sharing `e` are rebuilt along their other
/// diagonal, which keeps `e`'s softening. Returns the new edge, or `None` when `e`
/// cannot be flipped (not between two triangles, or the quad is not convex).
pub fn flip_edge(ents: &mut Entities, e: EdgeId) -> Option<EdgeId> {
    let ([a, d, b, c], [f1, f2]) = flip_quad(ents, e)?;
    let attrs = EdgeAttrs {
        curve: None,
        ..ents.edges[e].attrs.clone()
    };
    let (fa1, fa2) = (ents.faces[f1].attrs.clone(), ents.faces[f2].attrs.clone());
    ents.remove_face(f1);
    ents.remove_face(f2);
    ents.unlink_edge(e);
    let ne = ents.add_raw_edge(c, d, attrs);
    add_face(ents, &[a, d, c], &fa1, true)?;
    add_face(ents, &[d, b, c], &fa2, true)?;
    Some(ne)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::grid;
    use crate::{Checks, validate};

    fn valid(e: &Entities) {
        assert_eq!(validate(e, Checks::ALL), Vec::<String>::new());
    }

    /// A 6 x 6 grid of 10" squares from the origin.
    fn terrain() -> Entities {
        grid(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 10.0, 6, 6)
    }

    fn top(e: &Entities) -> f64 {
        e.vertices
            .values()
            .map(|v| v.pos.z)
            .fold(f64::MIN, f64::max)
    }

    #[test]
    fn falloff_is_smooth() {
        assert_eq!(falloff(0.0), 1.0);
        assert!((falloff(0.5) - 0.5).abs() < 1e-12);
        assert_eq!(falloff(1.0), 0.0);
        assert_eq!(falloff(2.0), 0.0);
    }

    #[test]
    fn smoove_raises_a_bump() {
        let mut t = terrain();
        let n = smoove(
            &mut t,
            DVec3::new(30.0, 30.0, 0.0),
            &BTreeSet::new(),
            15.0,
            8.0,
        );
        // The center and its 4 neighbors at 10 and 4 diagonal ones at 14.1.
        assert_eq!(n, 9);
        assert!((top(&t) - 8.0).abs() < 1e-9);
        let low = t
            .vertices
            .values()
            .map(|v| v.pos.z)
            .fold(f64::MAX, f64::min);
        assert_eq!(low, 0.0);
        assert!(t.faces.values().all(|f| f.normal.z > 0.0));
        valid(&t);
        // A dent the other way, deterministic.
        let mut u = terrain();
        smoove(
            &mut u,
            DVec3::new(30.0, 30.0, 0.0),
            &BTreeSet::new(),
            15.0,
            -8.0,
        );
        assert!(
            (u.vertices
                .values()
                .map(|v| v.pos.z)
                .fold(f64::MAX, f64::min)
                + 8.0)
                .abs()
                < 1e-9
        );
        valid(&u);
    }

    #[test]
    fn smoove_selected_vertices_move_fully() {
        let mut t = terrain();
        let sel: BTreeSet<VertexId> = t
            .vertices
            .iter()
            .filter(|(_, v)| v.pos.x == 0.0)
            .map(|(id, _)| id)
            .collect();
        smoove(&mut t, DVec3::ZERO, &sel, 15.0, 5.0);
        for (id, v) in &t.vertices {
            let want = if v.pos.x == 0.0 {
                5.0
            } else {
                5.0 * falloff(v.pos.x / 15.0)
            };
            assert!((v.pos.z - want).abs() < 1e-9, "{id:?}");
        }
        valid(&t);
    }

    #[test]
    fn smoove_triangulates_quads() {
        // One square face of 4 vertices, raised at a corner.
        let mut e = Entities::new();
        let pts = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            crate::add_line(&mut e, DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0));
        }
        assert_eq!(e.faces.len(), 1);
        smoove(&mut e, DVec3::ZERO, &BTreeSet::new(), 10.0, 3.0);
        assert_eq!(e.faces.len(), 2);
        assert_eq!(e.edges.values().filter(|x| x.attrs.soft).count(), 1);
        valid(&e);
    }

    #[test]
    fn walls_stay_put() {
        let mut t = terrain();
        // A wall face standing on the grid's edge y = 0.
        let a = DVec3::new(0.0, 0.0, 0.0);
        let b = DVec3::new(10.0, 0.0, 0.0);
        crate::add_line(&mut t, a, a + DVec3::Z * 10.0);
        crate::add_line(&mut t, a + DVec3::Z * 10.0, b + DVec3::Z * 10.0);
        crate::add_line(&mut t, b + DVec3::Z * 10.0, b);
        let walls = t.faces.values().filter(|f| f.normal.z.abs() < 0.5).count();
        assert_eq!(walls, 1);
        smoove(&mut t, DVec3::ZERO, &BTreeSet::new(), 15.0, 4.0);
        // The wall's foot did not move.
        assert!(t.find_vertex(a).is_some() && t.find_vertex(b).is_some());
        valid(&t);
    }

    #[test]
    fn drape_splits_faces() {
        let mut t = terrain();
        smoove(
            &mut t,
            DVec3::new(30.0, 30.0, 0.0),
            &BTreeSet::new(),
            25.0,
            10.0,
        );
        let before = t.faces.len();
        let a = DVec3::new(5.0, 31.0, 100.0);
        let b = DVec3::new(55.0, 31.0, 100.0);
        let edges = drape(&mut t, &[(a, b)]);
        assert!(edges.len() > 5);
        assert!(t.faces.len() > before);
        for &e in &edges {
            let edge = &t.edges[e];
            for v in [edge.start, edge.end] {
                let p = t.pos(v);
                assert!((p.y - 31.0).abs() < 1e-9);
                let h = height_at(&t, p.truncate()).unwrap();
                assert!((p.z - h).abs() < 1e-6, "{p} vs {h}");
            }
        }
        valid(&t);
        // A segment partly off the terrain is draped where it is over it.
        let mut g = terrain();
        let edges = drape(
            &mut g,
            &[(DVec3::new(-20.0, 5.0, 9.0), DVec3::new(25.0, 5.0, 9.0))],
        );
        let xs: Vec<f64> = edges
            .iter()
            .flat_map(|&e| [g.edges[e].start, g.edges[e].end])
            .map(|v| g.pos(v).x)
            .collect();
        assert!(xs.iter().all(|x| (0.0..=25.0).contains(x)));
        assert!(xs.contains(&0.0) && xs.contains(&25.0));
        valid(&g);
    }

    #[test]
    fn drape_a_closed_shape() {
        let mut t = terrain();
        smoove(
            &mut t,
            DVec3::new(30.0, 30.0, 0.0),
            &BTreeSet::new(),
            25.0,
            10.0,
        );
        let sq = [(17.0, 17.0), (43.0, 18.0), (42.0, 44.0), (16.0, 41.0)]
            .map(|(x, y)| DVec3::new(x, y, 50.0));
        let segs: Vec<_> = (0..4).map(|i| (sq[i], sq[(i + 1) % 4])).collect();
        let edges = drape(&mut t, &segs);
        assert!(!edges.is_empty());
        valid(&t);
    }

    #[test]
    fn stamp_makes_a_pad() {
        let mut t = terrain();
        smoove(
            &mut t,
            DVec3::new(30.0, 30.0, 0.0),
            &BTreeSet::new(),
            25.0,
            10.0,
        );
        let fp =
            [(22.0, 23.0), (38.0, 23.0), (38.0, 37.0), (22.0, 37.0)].map(|(x, y)| DVec2::new(x, y));
        let pad = stamp(&mut t, &fp, 6.0, 4.0).expect("stamp");
        let face = &t.faces[pad];
        assert!(face.normal.z > 0.999);
        let pts = t.loop_points(&face.outer);
        assert_eq!(pts.len(), 4);
        assert!(pts.iter().all(|p| (p.z - 4.0).abs() < 1e-12));
        // Nothing of the old terrain is left inside the offset outline.
        for v in t.vertices.values() {
            let q = v.pos.truncate();
            let inside =
                q.x > 16.0 + 1e-6 && q.x < 44.0 - 1e-6 && q.y > 17.0 + 1e-6 && q.y < 43.0 - 1e-6;
            if inside {
                assert!((v.pos.z - 4.0).abs() < 1e-9, "{}", v.pos);
            }
        }
        assert!(t.faces.values().all(|f| f.normal.z > 0.0));
        valid(&t);
    }

    #[test]
    fn stamp_errors_leave_terrain_alone() {
        let mut t = terrain();
        let n = t.faces.len();
        let off = [(50.0, 50.0), (70.0, 50.0), (70.0, 70.0)].map(|(x, y)| DVec2::new(x, y));
        assert_eq!(stamp(&mut t, &off, 2.0, 0.0), Err(StampError::OffTerrain));
        let fp = [(20.0, 20.0), (30.0, 20.0), (30.0, 30.0)].map(|(x, y)| DVec2::new(x, y));
        assert_eq!(stamp(&mut t, &fp, 0.0, 0.0), Err(StampError::NoOffset));
        let line = [(20.0, 20.0), (30.0, 20.0)].map(|(x, y)| DVec2::new(x, y));
        assert_eq!(stamp(&mut t, &line, 2.0, 0.0), Err(StampError::NoFootprint));
        assert_eq!(t.faces.len(), n);
        // A triangle footprint works.
        stamp(&mut t, &fp, 3.0, -2.0).expect("stamp");
        valid(&t);
    }

    #[test]
    fn footprint_of_a_box_is_its_bottom() {
        let mut e = Entities::new();
        let pts = [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)];
        for z in [2.0, 6.0] {
            for i in 0..4 {
                let (a, b) = (pts[i], pts[(i + 1) % 4]);
                crate::add_line(&mut e, DVec3::new(a.0, a.1, z), DVec3::new(b.0, b.1, z));
            }
        }
        let xf = glam::DMat4::from_translation(DVec3::new(100.0, 0.0, 1.0));
        let (poly, low) = footprint(&e, xf).unwrap();
        assert_eq!(low, 3.0);
        assert_eq!(poly.len(), 4);
        assert!(signed_area2(&poly) > 0.0);
        assert!(poly.iter().all(|p| p.x >= 100.0));
        // Loose edges only: their hull.
        let mut l = Entities::new();
        crate::add_line(&mut l, DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0));
        crate::add_line(&mut l, DVec3::new(4.0, 0.0, 0.0), DVec3::new(2.0, 3.0, 0.0));
        let (poly, _) = footprint(&l, glam::DMat4::IDENTITY).unwrap();
        assert_eq!(poly.len(), 3);
    }

    #[test]
    fn add_detail_quadruples_triangles() {
        let mut t = terrain();
        let n = t.faces.len();
        let picked: Vec<FaceId> = t
            .faces
            .iter()
            .filter(|(f, _)| {
                let c = crate::transform::face_sample(&t, *f);
                c.x < 10.0 && c.y < 10.0
            })
            .map(|(f, _)| f)
            .collect();
        assert_eq!(picked.len(), 2);
        let made = add_detail(&mut t, &picked);
        assert_eq!(made.len(), 8);
        // The 2 triangles became 8, the 2 neighbors across the square's sides got split
        // in two each.
        assert_eq!(t.faces.len(), n - 2 + 8 + 2);
        assert!(t.faces.values().all(|f| f.outer.coedges.len() == 3));
        valid(&t);
    }

    #[test]
    fn add_detail_vertex_on_face_and_edge() {
        let mut t = terrain();
        let f = t
            .faces
            .keys()
            .find(|&f| {
                let c = crate::transform::face_sample(&t, f);
                c.x > 20.0 && c.x < 30.0 && c.y > 20.0 && c.y < 30.0
            })
            .unwrap();
        let n = t.faces.len();
        let c = crate::transform::face_sample(&t, f);
        let v = add_detail_vertex(&mut t, DetailAt::Face(f), c, 5.0).unwrap();
        assert!((t.pos(v).z - 5.0).abs() < 1e-12);
        assert_eq!(t.faces.len(), n + 2);
        valid(&t);
        // On a grid line between two triangles: both split.
        let e = t
            .edges
            .iter()
            .find(|(_, x)| {
                let (a, b) = (t.pos(x.start), t.pos(x.end));
                a.y == 0.0 && b.y == 0.0 && a.x.min(b.x) == 0.0
            })
            .map(|(id, _)| id)
            .unwrap();
        let n = t.faces.len();
        add_detail_vertex(&mut t, DetailAt::Edge(e), DVec3::new(4.0, 0.0, 0.0), -2.0).unwrap();
        assert_eq!(t.faces.len(), n + 1);
        valid(&t);
    }

    #[test]
    fn flip_edge_swaps_the_diagonal() {
        let mut t = grid(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 10.0, 1, 1);
        let diag = t
            .edges
            .iter()
            .find(|(_, e)| e.attrs.soft)
            .map(|(id, _)| id)
            .unwrap();
        let ne = flip_edge(&mut t, diag).unwrap();
        let (a, b) = (t.pos(t.edges[ne].start), t.pos(t.edges[ne].end));
        assert_eq!(a.x + b.x, 10.0);
        assert_ne!(a.x, b.x);
        assert!(t.edges[ne].attrs.soft);
        assert_eq!(t.faces.len(), 2);
        assert!(t.faces.values().all(|f| f.normal.z > 0.99));
        valid(&t);
        // A border edge has one face: no flip.
        let border = t
            .edges
            .iter()
            .find(|(_, e)| e.faces.len() == 1)
            .map(|(id, _)| id)
            .unwrap();
        assert!(flip_edge(&mut t, border).is_none());
        // Flipping back gives the first diagonal again.
        let back = flip_edge(&mut t, ne).unwrap();
        let (a, b) = (t.pos(t.edges[back].start), t.pos(t.edges[back].end));
        assert_eq!((a.x - b.x).abs(), 10.0);
        assert_eq!(a.x + b.x, 10.0);
        valid(&t);
    }

    #[test]
    fn drape_through_grid_vertices() {
        // A diagonal through grid corners and along the soft diagonals.
        let mut t = terrain();
        smoove(
            &mut t,
            DVec3::new(30.0, 30.0, 0.0),
            &BTreeSet::new(),
            25.0,
            7.0,
        );
        let edges = drape(
            &mut t,
            &[
                (DVec3::new(0.0, 0.0, 0.0), DVec3::new(60.0, 60.0, 0.0)),
                (DVec3::new(0.0, 20.0, 0.0), DVec3::new(60.0, 20.0, 0.0)),
            ],
        );
        assert!(edges.len() >= 12);
        valid(&t);
    }

    #[test]
    fn stamp_an_l_shape() {
        let mut t = terrain();
        smoove(
            &mut t,
            DVec3::new(30.0, 30.0, 0.0),
            &BTreeSet::new(),
            25.0,
            10.0,
        );
        let fp = [
            (20.0, 20.0),
            (40.0, 20.0),
            (40.0, 28.0),
            (28.0, 28.0),
            (28.0, 40.0),
            (20.0, 40.0),
        ]
        .map(|(x, y)| DVec2::new(x, y));
        let pad = stamp(&mut t, &fp, 3.0, 12.0).expect("stamp");
        assert_eq!(t.faces[pad].outer.coedges.len(), 6);
        assert!(t.faces.values().all(|f| f.normal.z > 0.0));
        valid(&t);
    }

    #[test]
    fn concave_quad_does_not_flip() {
        let mut e = Entities::new();
        let p =
            [(0.0, 0.0), (10.0, 0.0), (2.0, 2.0), (0.0, 10.0)].map(|(x, y)| DVec3::new(x, y, 0.0));
        let vs: Vec<VertexId> = p.iter().map(|&q| e.add_vertex(q)).collect();
        add_face(&mut e, &[vs[0], vs[1], vs[2]], &FaceAttrs::default(), false).unwrap();
        add_face(&mut e, &[vs[0], vs[2], vs[3]], &FaceAttrs::default(), false).unwrap();
        let shared = e.edge_between(vs[0], vs[2]).unwrap();
        assert!(flip_edge(&mut e, shared).is_none());
        let f = e.faces.keys().next().unwrap();
        assert!(flippable_edge_near(&e, f, DVec3::new(1.0, 1.0, 0.0)).is_none());
        valid(&e);
    }
}
