//! SketchUp's Offset tool on a face.
//!
//! The offset is computed in the face's plane:
//! 1. every loop is offset as a closed polyline (the raw offset): straight edges move
//!    sideways by the distance and meet at mitered corners, arc edges become concentric
//!    arcs; nothing is checked, so the raw offset may cross itself and the other loops'
//!    raw offsets where parts of the face collapse;
//! 2. the raw offsets of all loops are split where they cross or touch each other;
//! 3. a piece is kept when it bounds the offset region: it is part of an edge's offset
//!    that still runs the way the edge does (an edge whose offset turned around has
//!    collapsed), the winding number of all raw loops is positive on its left and not on
//!    its right (so inverted parts, where an arm of the face vanished, and parts
//!    swallowed by another loop's offset go), it lies at the offset distance or more
//!    from the face's boundary and on the side the offset moves to (inside the face for
//!    an inward offset, outside for an outward one);
//! 4. kept straight pieces on one line are united (pieces of two edges' offsets meet
//!    or overlap where a part of the face between them vanished), and everything is
//!    added with `add_line` / `add_curve`.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{Arc, CurveId, EdgeId, Entities, FaceId};
use crate::geom::{Plane, point_in_polygon, point_segment};
use crate::ops::{add_curve, add_line};

/// SketchUp's Offset on a face: a copy of every loop of the face, offset within the face
/// plane by `distance` (positive = inward, shrinking the outer loop and growing holes;
/// negative = outward), with mitered corners, added with add_line so the face splits
/// into a ring and an inner face. Arcs in the loop (edges of one curve with an Arc)
/// stay arcs where they survive: they are offset as concentric arcs (new curve).
///
/// Where part of the face collapses the offset is trimmed, as in SketchUp: an arm of an
/// L narrower than twice the distance disappears and the rest is offset, a narrow neck
/// splits the offset into separate loops, a hole growing into the outer loop's offset
/// merges with it. Returns the new edges; empty when the whole offset collapses (e.g.
/// inward past the inradius), and then nothing is added.
///
/// Differences from SketchUp (unverified): corners are always mitered, however sharp
/// (no miter limit or rounding); where an arc is cut by the trimming, the cut piece is
/// a chord piece rather than an exact arc end.
pub fn offset_face(ents: &mut Entities, face: FaceId, distance: f64) -> Vec<EdgeId> {
    if !ents.faces.contains_key(face) || distance.abs() <= TOL || !distance.is_finite() {
        return Vec::new();
    }
    ents.ensure_index();
    let plane = ents.face_plane(face);
    let Some(input) = face_loops(ents, face, &plane) else {
        return Vec::new();
    };
    let raw: Vec<Vec<Seg>> = input
        .iter()
        .map(|l| raw_offset(&plane, l, distance))
        .collect();
    let kept = trim(&input, &raw, distance);

    // Straight pieces on one line are merged: pieces of different raw segments can
    // overlap or continue each other where a vanished part of the face was.
    let mut arcs: Vec<(ArcRef, Vec<Piece>)> = Vec::new();
    let mut lines: Vec<(DVec2, DVec2)> = Vec::new();
    for pieces in &kept {
        for run in runs(pieces) {
            match run {
                Run::Line(p) => lines.push((p.a, p.b)),
                Run::Arc(arc, ps) => arcs.push((arc, ps)),
            }
        }
    }
    let lines = merge_collinear(&lines);

    let lift = |q: DVec2| lift(&plane, q);
    let segments: Vec<(DVec3, DVec3)> = arcs
        .iter()
        .flat_map(|(_, ps)| ps.iter().map(|p| (p.a, p.b)))
        .chain(lines.iter().copied())
        .map(|(a, b)| (lift(a), lift(b)))
        .collect();
    if segments.is_empty() {
        return Vec::new();
    }
    let before = EdgeSnapshot::near(ents, &segments);
    for (arc, ps) in &arcs {
        let mut pts: Vec<DVec3> = ps.iter().map(|p| lift(p.a)).collect();
        pts.push(lift(ps[ps.len() - 1].b));
        let arc = concentric_arc(&arc.arc, arc.radius, &mut pts);
        add_curve(ents, &pts, Some(arc));
    }
    for &(a, b) in &lines {
        add_line(ents, lift(a), lift(b));
    }
    before.added_on(ents, &segments)
}

/// A 2D segment.
type Seg2 = (DVec2, DVec2);

/// Unites segments lying on one line where they overlap or touch.
fn merge_collinear(segs: &[Seg2]) -> Vec<Seg2> {
    let eps = 1e-9;
    // Groups of segments on one line: (point on the line, unit direction, members).
    let mut groups: Vec<(DVec2, DVec2, Vec<Seg2>)> = Vec::new();
    for &(a, b) in segs {
        let Some(d) = (b - a).try_normalize() else {
            continue;
        };
        let group = groups
            .iter_mut()
            .find(|(o, dir, _)| dir.perp_dot(d).abs() <= eps && dir.perp_dot(a - *o).abs() <= 1e-7);
        match group {
            Some(g) => g.2.push((a, b)),
            None => groups.push((a, d, vec![(a, b)])),
        }
    }
    let mut out = Vec::new();
    for (o, d, members) in groups {
        let mut spans: Vec<(f64, f64)> = members
            .iter()
            .map(|&(a, b)| {
                let (s, t) = ((a - o).dot(d), (b - o).dot(d));
                (s.min(t), s.max(t))
            })
            .collect();
        spans.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (s, t) in spans {
            match merged.last_mut() {
                Some(m) if s <= m.1 + 1e-7 => m.1 = m.1.max(t),
                _ => merged.push((s, t)),
            }
        }
        // Ends are taken from the members' own points where possible, so pieces that
        // were not merged keep their exact coordinates.
        let at = |t: f64| {
            members
                .iter()
                .flat_map(|&(a, b)| [a, b])
                .find(|p| ((*p - o).dot(d) - t).abs() <= 1e-12)
                .unwrap_or(o + d * t)
        };
        out.extend(merged.into_iter().map(|(s, t)| (at(s), at(t))));
    }
    out
}

/// Edges near some segments before an operation, to tell which edges it added along them.
pub(crate) struct EdgeSnapshot {
    ids: BTreeSet<EdgeId>,
    segments: Vec<(DVec3, DVec3)>,
}

impl EdgeSnapshot {
    /// The edges near `segments` (the only ones `added_on` needs to know about).
    pub(crate) fn near(ents: &mut Entities, segments: &[(DVec3, DVec3)]) -> Self {
        ents.ensure_index();
        let ids = edges_near_all(ents, segments);
        Self {
            segments: ids
                .iter()
                .map(|&e| (ents.pos(ents.edges[e].start), ents.pos(ents.edges[e].end)))
                .collect(),
            ids: ids.into_iter().collect(),
        }
    }

    /// Edges added since the snapshot that lie on one of `segments`: what an operation
    /// added along them, after any splitting by later lines. Pieces of edges that
    /// existed before (split, so with new ids) do not count.
    pub(crate) fn added_on(&self, ents: &Entities, segments: &[(DVec3, DVec3)]) -> Vec<EdgeId> {
        let on = |p: DVec3, q: DVec3, list: &[(DVec3, DVec3)]| {
            list.iter()
                .any(|&(a, b)| point_segment(p, a, b).0 <= TOL && point_segment(q, a, b).0 <= TOL)
        };
        edges_near_all(ents, segments)
            .into_iter()
            .filter(|id| !self.ids.contains(id))
            .filter(|&id| {
                let e = &ents.edges[id];
                let (p, q) = (ents.pos(e.start), ents.pos(e.end));
                on(p, q, segments) && !on(p, q, &self.segments)
            })
            .collect()
    }
}

/// Candidate edges within 2 TOL of any of the segments, in key order.
fn edges_near_all(ents: &Entities, segments: &[(DVec3, DVec3)]) -> Vec<EdgeId> {
    let mut ids: Vec<EdgeId> = segments
        .iter()
        .flat_map(|&(a, b)| ents.edges_near_segment(a, b, 2.0 * TOL))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn lift(plane: &Plane, q: DVec2) -> DVec3 {
    let (u, v) = plane.basis();
    u * q.x + v * q.y + plane.normal * plane.d
}

/// An arc curve of the face and its offset radius.
#[derive(Debug, Clone, Copy)]
struct ArcRef {
    id: CurveId,
    arc: Arc,
    radius: f64,
}

/// A loop of the face in plane coordinates: edge `k` runs from point `k` to point
/// `k + 1` and is a chord of `arcs[k]` when it belongs to an arc curve.
struct InLoop {
    points: Vec<DVec2>,
    arcs: Vec<Option<(CurveId, Arc)>>,
}

fn face_loops(ents: &Entities, face: FaceId, plane: &Plane) -> Option<Vec<InLoop>> {
    let f = &ents.faces[face];
    let mut out = Vec::new();
    for l in f.loops() {
        let points: Vec<DVec2> = ents
            .loop_points(l)
            .into_iter()
            .map(|p| plane.project(p))
            .collect();
        let n = points.len();
        if n < 3 || (0..n).any(|k| points[k].distance(points[(k + 1) % n]) <= 1e-12) {
            return None;
        }
        let arcs: Vec<Option<(CurveId, Arc)>> = l
            .coedges
            .iter()
            .map(|c| {
                let id = ents.edges[c.edge].attrs.curve?;
                let arc = ents.curves.get(id)?.arc?;
                Some((id, arc))
            })
            .collect();
        out.push(InLoop { points, arcs });
    }
    Some(out)
}

/// A segment of a raw offset loop; `arc` is set for chords of an offset arc.
#[derive(Debug, Clone, Copy)]
struct Seg {
    a: DVec2,
    b: DVec2,
    arc: Option<ArcRef>,
    /// Runs the way its original edge does. An edge whose offset turned around has
    /// collapsed (its neighbors' offsets meet before it), so no piece of it is kept.
    forward: bool,
}

fn left(d: DVec2) -> DVec2 {
    DVec2::new(-d.y, d.x)
}

/// The raw offset of one loop: a closed chain of segments (each one's end is the next
/// one's start), possibly crossing itself.
fn raw_offset(plane: &Plane, l: &InLoop, distance: f64) -> Vec<Seg> {
    let pts = &l.points;
    let n = pts.len();
    let dirs: Vec<DVec2> = (0..n)
        .map(|k| (pts[(k + 1) % n] - pts[k]).normalize())
        .collect();
    // The face is on the left of every loop edge (outer loop counter-clockwise, holes
    // clockwise), so moving left is moving into the face.
    let normals: Vec<DVec2> = dirs.iter().map(|d| left(*d)).collect();

    // New radius per arc: an arc whose center is on the face side shrinks when the
    // offset moves into the face. An arc shrinking to nothing is offset like straight
    // edges (its chords invert and are trimmed away).
    let mut arcs: Vec<Option<ArcRef>> = vec![None; n];
    for k in 0..n {
        let Some((id, arc)) = l.arcs[k] else { continue };
        let center = plane.project(arc.center);
        let mid = (pts[k] + pts[(k + 1) % n]) / 2.0;
        let toward = normals[k].dot(center - mid).signum();
        let radius = arc.radius - distance * toward;
        if radius > TOL {
            arcs[k] = Some(ArcRef { id, arc, radius });
        }
    }

    // Offset corner point(s) per vertex: one, or two for an edge doubling back on itself.
    let corners: Vec<Vec<DVec2>> = (0..n)
        .map(|k| {
            let prev = (k + n - 1) % n;
            let (n1, n2) = (normals[prev], normals[k]);
            let denom = 1.0 + n1.dot(n2);
            if denom < 1e-9 {
                return vec![pts[k] + n1 * distance, pts[k] + n2 * distance];
            }
            let miter = pts[k] + (n1 + n2) / denom * distance;
            let q = match (&arcs[prev], &arcs[k]) {
                (Some(a), Some(b)) if a.id == b.id => {
                    let c = plane.project(a.arc.center);
                    c + (pts[k] - c).normalize() * a.radius
                }
                (Some(a), _) => line_circle(pts[k] + n2 * distance, dirs[k], plane, a, miter),
                (None, Some(b)) => line_circle(pts[k] + n1 * distance, dirs[prev], plane, b, miter),
                (None, None) => miter,
            };
            vec![q]
        })
        .collect();

    let mut segs = Vec::with_capacity(n + 2);
    for k in 0..n {
        if let [p, q] = corners[k][..] {
            segs.push(Seg {
                a: p,
                b: q,
                arc: None,
                forward: true,
            });
        }
        let a = *corners[k].last().expect("a corner");
        let b = corners[(k + 1) % n][0];
        segs.push(Seg {
            a,
            b,
            arc: arcs[k],
            forward: (b - a).dot(dirs[k]) > 0.0,
        });
    }
    segs
}

/// Where the offset line `o + t d` meets the offset circle of `arc`: the intersection
/// nearest `fallback` (the plain miter point), or `fallback` if they miss.
fn line_circle(o: DVec2, d: DVec2, plane: &Plane, arc: &ArcRef, fallback: DVec2) -> DVec2 {
    let c = plane.project(arc.arc.center);
    let r = arc.radius;
    let f = o - c;
    let b = f.dot(d);
    let disc = b * b - (f.length_squared() - r * r);
    if disc < 0.0 {
        return fallback;
    }
    let s = disc.sqrt();
    [-b - s, -b + s]
        .into_iter()
        .map(|t| o + d * t)
        .min_by(|x, y| x.distance(fallback).total_cmp(&y.distance(fallback)))
        .unwrap_or(fallback)
}

/// A kept piece of a raw segment.
#[derive(Debug, Clone, Copy)]
struct Piece {
    a: DVec2,
    b: DVec2,
    arc: Option<ArcRef>,
    /// The piece ends where the raw segment ends (so it joins the next segment's piece
    /// if that is kept from its start).
    to_end: bool,
    from_start: bool,
    /// Index of the raw segment it is a piece of.
    seg: usize,
}

/// Splits the raw loops at their crossings and keeps the pieces bounding the offset
/// region; per loop, in loop order.
fn trim(input: &[InLoop], raw: &[Vec<Seg>], distance: f64) -> Vec<Vec<Piece>> {
    let all: Vec<(usize, usize)> = raw
        .iter()
        .enumerate()
        .flat_map(|(i, l)| (0..l.len()).map(move |k| (i, k)))
        .collect();
    let seg = |(i, k): (usize, usize)| raw[i][k];

    // Split parameters per segment, from every crossing or touching pair. Segments are
    // swept along x so only pairs with overlapping x ranges are tested.
    let mut params: Vec<Vec<f64>> = vec![vec![0.0, 1.0]; all.len()];
    let mut order: Vec<usize> = (0..all.len()).collect();
    let xmin = |s: &Seg| s.a.x.min(s.b.x);
    let xmax = |s: &Seg| s.a.x.max(s.b.x);
    order.sort_by(|&x, &y| xmin(&seg(all[x])).total_cmp(&xmin(&seg(all[y]))));
    let eps = 1e-9;
    for (oi, &x) in order.iter().enumerate() {
        let s = seg(all[x]);
        for &y in &order[oi + 1..] {
            let t = seg(all[y]);
            if xmin(&t) > xmax(&s) + eps {
                break;
            }
            for (u, v) in crossings(s.a, s.b, t.a, t.b) {
                params[x].push(u);
                params[y].push(v);
            }
        }
    }

    // Arcs are polylines, their offsets concentric polylines: those come nearer to the
    // original chords than the distance, by up to the sagitta of a chord of the bigger
    // of the two arcs.
    let sag = input
        .iter()
        .flat_map(|l| l.arcs.iter().flatten())
        .map(|(_, a)| {
            let step = (a.end_angle - a.start_angle).abs() / a.segments.max(1) as f64;
            (a.radius + distance.abs()) * (1.0 - (step.min(std::f64::consts::PI) / 2.0).cos())
        })
        .fold(0.0, f64::max);
    let min_dist = distance.abs() - sag - TOL;

    let mut out: Vec<Vec<Piece>> = vec![Vec::new(); raw.len()];
    for (x, &(i, k)) in all.iter().enumerate() {
        let s = seg((i, k));
        if !s.forward {
            continue;
        }
        let ps = &mut params[x];
        ps.sort_by(f64::total_cmp);
        ps.dedup_by(|b, a| (*b - *a) * s.a.distance(s.b) <= 1e-9);
        let last = ps.len() - 1;
        for j in 0..last {
            let (t0, t1) = (ps[j], if j + 1 == last { 1.0 } else { ps[j + 1] });
            let (a, b) = (s.a.lerp(s.b, t0), s.a.lerp(s.b, t1));
            if a.distance(b) <= 1e-9 {
                continue;
            }
            if !bounds_region(raw, input, a, b, distance, min_dist) {
                continue;
            }
            // A piece continuing the previous kept piece of the same segment extends it.
            match out[i].last_mut() {
                Some(p) if p.seg == x && p.b == a => {
                    p.b = b;
                    p.to_end = j + 1 == last;
                }
                _ => out[i].push(Piece {
                    a,
                    b,
                    arc: s.arc,
                    to_end: j + 1 == last,
                    from_start: j == 0,
                    seg: x,
                }),
            }
        }
    }
    out
}

/// Parameters `(s, t)` along `p0-p1` and `q0-q1` where they cross or touch; the ends of
/// the shared part for collinear overlapping segments.
fn crossings(p0: DVec2, p1: DVec2, q0: DVec2, q1: DVec2) -> Vec<(f64, f64)> {
    let r = p1 - p0;
    let w = q1 - q0;
    let denom = r.perp_dot(w);
    let (lr, lw) = (r.length(), w.length());
    let eps = 1e-9;
    let on = |u: f64, len: f64| u * len >= -eps && (u - 1.0) * len <= eps;
    if denom.abs() > 1e-12 * lr * lw {
        let s = (q0 - p0).perp_dot(w) / denom;
        let t = (q0 - p0).perp_dot(r) / denom;
        return if on(s, lr) && on(t, lw) {
            vec![(s.clamp(0.0, 1.0), t.clamp(0.0, 1.0))]
        } else {
            Vec::new()
        };
    }
    // Parallel: only collinear overlaps matter.
    if (q0 - p0).perp_dot(r).abs() > eps * lr {
        return Vec::new();
    }
    let proj = |p: DVec2, o: DVec2, d: DVec2| (p - o).dot(d) / d.length_squared();
    let mut out = Vec::new();
    for q in [q0, q1] {
        let s = proj(q, p0, r);
        if on(s, lr) {
            out.push((s.clamp(0.0, 1.0), proj(q, q0, w).clamp(0.0, 1.0)));
        }
    }
    for p in [p0, p1] {
        let t = proj(p, q0, w);
        if on(t, lw) {
            out.push((proj(p, p0, r).clamp(0.0, 1.0), t.clamp(0.0, 1.0)));
        }
    }
    out
}

/// Whether the piece `a-b` of a raw offset bounds the offset region: winding number of
/// the raw loops positive on its left and not on its right, far enough from the face's
/// boundary and on the side the offset moves to.
fn bounds_region(
    raw: &[Vec<Seg>],
    input: &[InLoop],
    a: DVec2,
    b: DVec2,
    distance: f64,
    min_dist: f64,
) -> bool {
    let mid = (a + b) / 2.0;
    let side = left((b - a).normalize()) * (TOL * 1e-2).min(a.distance(b) / 4.0);
    if winding(raw, mid + side) < 1 || winding(raw, mid - side) > 0 {
        return false;
    }
    if boundary_distance(input, mid) < min_dist {
        return false;
    }
    let outer = &input[0].points;
    let inside = point_in_polygon(mid, outer)
        && !input[1..].iter().any(|h| point_in_polygon(mid, &h.points));
    inside == (distance > 0.0)
}

/// Winding number of all raw loops around `p`.
fn winding(raw: &[Vec<Seg>], p: DVec2) -> i32 {
    let mut w = 0;
    for s in raw.iter().flatten() {
        let side = (s.b - s.a).perp_dot(p - s.a);
        if s.a.y <= p.y {
            if s.b.y > p.y && side > 0.0 {
                w += 1;
            }
        } else if s.b.y <= p.y && side < 0.0 {
            w -= 1;
        }
    }
    w
}

/// Distance from `p` to the face's boundary (arcs as their chords).
fn boundary_distance(input: &[InLoop], p: DVec2) -> f64 {
    let to3 = |q: DVec2| DVec3::new(q.x, q.y, 0.0);
    input
        .iter()
        .flat_map(|l| {
            let n = l.points.len();
            (0..n).map(move |k| (l.points[k], l.points[(k + 1) % n]))
        })
        .map(|(a, b)| point_segment(to3(p), to3(a), to3(b)).0)
        .fold(f64::INFINITY, f64::min)
}

/// What to add for a loop's kept pieces: straight pieces one by one, consecutive joined
/// pieces of one offset arc as one curve.
enum Run {
    Line(Piece),
    Arc(ArcRef, Vec<Piece>),
}

fn runs(pieces: &[Piece]) -> Vec<Run> {
    let n = pieces.len();
    if n == 0 {
        return Vec::new();
    }
    // Piece `j` continues piece `j - 1` (cyclically) on the same arc.
    let joins = |j: usize| {
        let (p, q) = (pieces[(j + n - 1) % n], pieces[j]);
        let same_arc = matches!((p.arc, q.arc), (Some(x), Some(y)) if x.id == y.id);
        same_arc && (p.to_end && q.from_start || !p.to_end && !q.from_start) && p.b == q.a
    };
    let start = (0..n).find(|&j| !joins(j)).unwrap_or(0);
    let mut out: Vec<Run> = Vec::new();
    for i in 0..n {
        let j = (start + i) % n;
        let p = pieces[j];
        match p.arc {
            None => out.push(Run::Line(p)),
            Some(arc) => match out.last_mut() {
                Some(Run::Arc(_, ps)) if i > 0 && joins(j) => ps.push(p),
                _ => out.push(Run::Arc(arc, vec![p])),
            },
        }
    }
    out
}

/// Arc data for an offset run: same center, normal and axes, new radius. Reorders
/// `pts` so that the angle increases from start to end.
fn concentric_arc(a: &Arc, radius: f64, pts: &mut [DVec3]) -> Arc {
    let y = a.y_axis();
    let angle = |p: DVec3| {
        let d = p - a.center;
        d.dot(y).atan2(d.dot(a.x_axis))
    };
    let step = |p: DVec3, q: DVec3| {
        let (dp, dq) = (p - a.center, q - a.center);
        dp.cross(dq).dot(a.normal).atan2(dp.dot(dq))
    };
    if pts.len() >= 2 && step(pts[0], pts[1]) < 0.0 {
        pts.reverse();
    }
    let sweep: f64 = pts.windows(2).map(|w| step(w[0], w[1])).sum();
    let sweep = if (sweep - std::f64::consts::TAU).abs() < 1e-6 {
        std::f64::consts::TAU
    } else {
        sweep
    };
    let start = angle(pts[0]);
    Arc {
        radius,
        start_angle: start,
        end_angle: start + sweep,
        segments: pts.len() - 1,
        ..*a
    }
}
