//! SketchUp's Offset tool on a face.

use std::collections::HashSet;

use glam::{DVec2, DVec3};

use crate::TOL;
use crate::entities::{Arc, CurveId, EdgeId, Entities, FaceId};
use crate::geom::{Plane, point_in_polygon, point_segment, signed_area2};
use crate::ops::{add_curve, add_line};

/// SketchUp's Offset on a face: a copy of every loop of the face, offset within the face
/// plane by `distance` (positive = inward, shrinking the outer loop and growing holes;
/// negative = outward), with mitered corners, added with add_line so the face splits
/// into a ring and an inner face. Arcs in the loop (edges of one curve with an Arc)
/// stay arcs: they are offset as concentric arcs (new curve). Returns the new edges;
/// empty if the offset collapses (e.g. inward past the inradius) — then nothing is added.
///
/// Differences from SketchUp: SketchUp trims a partly collapsing offset (a narrow part
/// of an L disappears, the rest is offset); here any collapse, self-crossing or loops
/// running into each other cancels the whole offset.
pub fn offset_face(ents: &mut Entities, face: FaceId, distance: f64) -> Vec<EdgeId> {
    if !ents.faces.contains_key(face) || distance.abs() <= TOL {
        return Vec::new();
    }
    let plane = ents.face_plane(face);
    let Some(loops) = offset_loops(ents, face, &plane, distance) else {
        return Vec::new();
    };

    let before = EdgeSnapshot::of(ents);
    let mut segments = Vec::new();
    for l in &loops {
        let n = l.points.len();
        let lift = |q: DVec2| lift(&plane, q);
        for piece in pieces(&l.curves) {
            match piece {
                Piece::Line(k) => {
                    add_line(ents, lift(l.points[k]), lift(l.points[(k + 1) % n]));
                }
                Piece::Arc(arc, ks) => {
                    let mut pts: Vec<DVec3> = ks.iter().map(|&k| lift(l.points[k])).collect();
                    pts.push(lift(l.points[(ks[ks.len() - 1] + 1) % n]));
                    let arc = concentric_arc(&arc, l.radius_of(&arc), &mut pts);
                    add_curve(ents, &pts, Some(arc));
                }
            }
        }
        for k in 0..n {
            segments.push((lift(l.points[k]), lift(l.points[(k + 1) % n])));
        }
    }
    before.added_on(ents, &segments)
}

/// The edges of a model before an operation, to tell which edges it added.
pub(crate) struct EdgeSnapshot {
    ids: HashSet<EdgeId>,
    segments: Vec<(DVec3, DVec3)>,
}

impl EdgeSnapshot {
    pub(crate) fn of(ents: &Entities) -> Self {
        Self {
            ids: ents.edges.keys().collect(),
            segments: ents
                .edges
                .values()
                .map(|e| (ents.pos(e.start), ents.pos(e.end)))
                .collect(),
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
        ents.edges
            .iter()
            .filter(|(id, _)| !self.ids.contains(id))
            .filter(|(_, e)| {
                let (p, q) = (ents.pos(e.start), ents.pos(e.end));
                on(p, q, segments) && !on(p, q, &self.segments)
            })
            .map(|(id, _)| id)
            .collect()
    }
}

fn lift(plane: &Plane, q: DVec2) -> DVec3 {
    let (u, v) = plane.basis();
    u * q.x + v * q.y + plane.normal * plane.d
}

/// One offset loop in plane coordinates: point `k` is the offset of the loop's vertex
/// `k`; edge `k` runs from point `k` to point `k + 1` and belongs to `curves[k]` (an arc
/// curve) or to no arc.
struct OffsetLoop {
    points: Vec<DVec2>,
    curves: Vec<Option<(CurveId, Arc)>>,
    /// New radius per arc curve.
    radii: Vec<(CurveId, f64)>,
}

impl OffsetLoop {
    fn radius_of(&self, arc: &(CurveId, Arc)) -> f64 {
        self.radii
            .iter()
            .find(|(c, _)| *c == arc.0)
            .map_or(arc.1.radius, |r| r.1)
    }
}

enum Piece {
    Line(usize),
    Arc((CurveId, Arc), Vec<usize>),
}

/// Splits a loop's edges into straight edges and runs of consecutive edges of one arc.
fn pieces(curves: &[Option<(CurveId, Arc)>]) -> Vec<Piece> {
    let n = curves.len();
    let same = |a: usize, b: usize| match (&curves[a], &curves[b]) {
        (Some(x), Some(y)) => x.0 == y.0,
        _ => false,
    };
    // Start at a boundary between runs (or anywhere for a loop of one curve).
    let start = (0..n).find(|&k| !same((k + n - 1) % n, k)).unwrap_or(0);
    let mut out: Vec<Piece> = Vec::new();
    for i in 0..n {
        let k = (start + i) % n;
        match &curves[k] {
            None => out.push(Piece::Line(k)),
            Some(c) => match out.last_mut() {
                Some(Piece::Arc(prev, ks))
                    if i > 0 && prev.0 == c.0 && same((k + n - 1) % n, k) =>
                {
                    ks.push(k)
                }
                _ => out.push(Piece::Arc(*c, vec![k])),
            },
        }
    }
    out
}

/// Arc data for an offset run: same center, normal and axes, new radius. Reorders
/// `pts` so that the angle increases from start to end.
fn concentric_arc(arc: &(CurveId, Arc), radius: f64, pts: &mut [DVec3]) -> Arc {
    let a = arc.1;
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
        ..a
    }
}

fn left(d: DVec2) -> DVec2 {
    DVec2::new(-d.y, d.x)
}

/// Offsets every loop of the face, or `None` if the result is degenerate.
fn offset_loops(
    ents: &Entities,
    face: FaceId,
    plane: &Plane,
    distance: f64,
) -> Option<Vec<OffsetLoop>> {
    let f = &ents.faces[face];
    let mut out = Vec::new();
    let mut originals: Vec<Vec<DVec2>> = Vec::new();
    for l in f.loops() {
        let pts: Vec<DVec2> = ents
            .loop_points(l)
            .into_iter()
            .map(|p| plane.project(p))
            .collect();
        let curves: Vec<Option<(CurveId, Arc)>> = l
            .coedges
            .iter()
            .map(|c| {
                let id = ents.edges[c.edge].attrs.curve?;
                let arc = ents.curves.get(id)?.arc?;
                Some((id, arc))
            })
            .collect();
        out.push(offset_loop(plane, &pts, curves, distance)?);
        originals.push(pts);
    }

    // Collapse checks: every edge keeps its direction, every loop its winding, and no
    // offset edge crosses another offset edge or an original edge.
    let mut new_segs: Vec<(usize, usize, DVec2, DVec2)> = Vec::new();
    for (i, (l, orig)) in out.iter().zip(&originals).enumerate() {
        let n = l.points.len();
        let a0 = signed_area2(orig);
        let a1 = signed_area2(&l.points);
        if a0 * a1 <= 0.0 {
            return None;
        }
        for k in 0..n {
            let (p, q) = (l.points[k], l.points[(k + 1) % n]);
            let d0 = orig[(k + 1) % n] - orig[k];
            if (q - p).length() <= TOL || (q - p).dot(d0) <= 0.0 {
                return None;
            }
            new_segs.push((i, k, p, q));
        }
    }
    // Holes stay inside the outer loop and outside each other.
    for (i, h) in out.iter().enumerate().skip(1) {
        let q = h.points[0];
        if !point_in_polygon(q, &out[0].points)
            || out[1..]
                .iter()
                .enumerate()
                .any(|(j, g)| j + 1 != i && point_in_polygon(q, &g.points))
        {
            return None;
        }
    }
    for (x, &(li, ki, p0, p1)) in new_segs.iter().enumerate() {
        for &(lj, kj, q0, q1) in &new_segs[x + 1..] {
            let n = out[li].points.len();
            let adjacent = li == lj && (kj == (ki + 1) % n || ki == (kj + 1) % n);
            if !adjacent && segments_cross(p0, p1, q0, q1) {
                return None;
            }
        }
        for orig in &originals {
            for k in 0..orig.len() {
                if segments_cross(p0, p1, orig[k], orig[(k + 1) % orig.len()]) {
                    return None;
                }
            }
        }
    }
    Some(out)
}

/// Whether two 2D segments touch or cross (within tolerance).
fn segments_cross(p0: DVec2, p1: DVec2, q0: DVec2, q1: DVec2) -> bool {
    let to3 = |p: DVec2| DVec3::new(p.x, p.y, 0.0);
    match crate::geom::segment_segment(to3(p0), to3(p1), to3(q0), to3(q1)) {
        Some((d, _, _)) => d <= TOL,
        None => {
            // Parallel: touching only if collinear and overlapping.
            let pd = |p: DVec2| point_segment(to3(p), to3(q0), to3(q1)).0;
            let qd = |q: DVec2| point_segment(to3(q), to3(p0), to3(p1)).0;
            pd(p0) <= TOL || pd(p1) <= TOL || qd(q0) <= TOL || qd(q1) <= TOL
        }
    }
}

fn offset_loop(
    plane: &Plane,
    pts: &[DVec2],
    curves: Vec<Option<(CurveId, Arc)>>,
    distance: f64,
) -> Option<OffsetLoop> {
    let n = pts.len();
    let dirs: Vec<DVec2> = (0..n)
        .map(|k| (pts[(k + 1) % n] - pts[k]).normalize_or_zero())
        .collect();
    if dirs.iter().any(|d| *d == DVec2::ZERO) {
        return None;
    }
    // The face is on the left of every loop edge (outer loop counter-clockwise, holes
    // clockwise), so moving left is moving into the face.
    let normals: Vec<DVec2> = dirs.iter().map(|d| left(*d)).collect();

    // New radius per arc: an arc whose center is on the face side shrinks when the
    // offset moves into the face.
    let mut radii: Vec<(CurveId, f64)> = Vec::new();
    for (k, c) in curves.iter().enumerate() {
        let Some((id, arc)) = c else { continue };
        if radii.iter().any(|r| r.0 == *id) {
            continue;
        }
        let center = plane.project(arc.center);
        let mid = (pts[k] + pts[(k + 1) % n]) / 2.0;
        let toward = normals[k].dot(center - mid).signum();
        let r = arc.radius - distance * toward;
        if r <= TOL {
            return None;
        }
        radii.push((*id, r));
    }
    let radius = |id: CurveId| radii.iter().find(|r| r.0 == id).map(|r| r.1);

    let mut points = Vec::with_capacity(n);
    for k in 0..n {
        let prev = (k + n - 1) % n;
        let (n1, n2) = (normals[prev], normals[k]);
        let denom = 1.0 + n1.dot(n2);
        if denom < 1e-9 {
            return None; // the loop doubles back on itself
        }
        let miter = pts[k] + (n1 + n2) / denom * distance;
        let q = match (&curves[prev], &curves[k]) {
            (Some(a), Some(b)) if a.0 == b.0 => {
                let c = plane.project(a.1.center);
                c + (pts[k] - c).normalize() * radius(a.0)?
            }
            (Some(a), None) => line_circle(
                pts[k] + n2 * distance,
                dirs[k],
                plane,
                a,
                radius(a.0)?,
                miter,
            ),
            (None, Some(b)) => line_circle(
                pts[k] + n1 * distance,
                dirs[prev],
                plane,
                b,
                radius(b.0)?,
                miter,
            ),
            _ => miter,
        };
        points.push(q);
    }
    Some(OffsetLoop {
        points,
        curves,
        radii,
    })
}

/// Where the offset line `o + t d` meets the offset circle of `arc`: the intersection
/// nearest `fallback` (the plain miter point), or `fallback` if they miss.
fn line_circle(
    o: DVec2,
    d: DVec2,
    plane: &Plane,
    arc: &(CurveId, Arc),
    r: f64,
    fallback: DVec2,
) -> DVec2 {
    let c = plane.project(arc.1.center);
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
