//! Divide, Weld Edges and Explode Curve (context menu of edges and curves).
//!
//! Drafts from memory of SketchUp (`spec/parity/STATUS.md`):
//! - Divide splits a straight edge into `n` equal edges. On an arc or circle it
//!   re-segments the curve into `n` equal segments and it stays one arc curve
//!   (unverified). Free-form curves (Freehand, welded edges) are not divided here.
//! - Weld Edges joins connected edges into one free-form curve; a chain that branches
//!   is not welded (unverified: SketchUp may weld the longest chain).
//! - Explode Curve turns a curve's segments into separate edges.

use std::collections::{HashMap, HashSet};

use glam::DVec3;

use crate::entities::{Arc, CoEdge, Curve, CurveId, EdgeId, Entities, FaceId, Loop, VertexId};

/// Most segments Divide makes (SketchUp's limit is unverified).
pub const MAX_DIVISIONS: usize = 1000;

/// The edges of `start`'s curve (or `start` alone) as an ordered chain: the edges with
/// the direction each runs along the chain, and the `edges + 1` vertices (the last
/// equals the first for a closed chain). `None` if they do not form one chain.
pub fn chain_of(ents: &Entities, edges: &[EdgeId]) -> Option<(Vec<EdgeId>, Vec<VertexId>)> {
    let set: HashSet<EdgeId> = edges.iter().copied().collect();
    if set.is_empty() || set.iter().any(|e| !ents.edges.contains_key(*e)) {
        return None;
    }
    // Vertex degree within the set.
    let mut degree: HashMap<VertexId, usize> = HashMap::new();
    for &e in &set {
        let edge = &ents.edges[e];
        *degree.entry(edge.start).or_default() += 1;
        *degree.entry(edge.end).or_default() += 1;
    }
    if degree.values().any(|&d| d > 2) {
        return None;
    }
    let ends: Vec<VertexId> = {
        let mut v: Vec<VertexId> = degree
            .iter()
            .filter(|&(_, &d)| d == 1)
            .map(|(&v, _)| v)
            .collect();
        v.sort();
        v
    };
    let first = match ends[..] {
        [a, _] => a,
        // Closed: start anywhere (the smallest key, for repeatable results).
        [] => *degree.keys().min()?,
        _ => return None,
    };
    let mut order = Vec::with_capacity(set.len());
    let mut verts = vec![first];
    let mut used: HashSet<EdgeId> = HashSet::new();
    let mut v = first;
    loop {
        let next = ents.vertices[v]
            .edges
            .iter()
            .copied()
            .filter(|e| set.contains(e) && !used.contains(e))
            .min();
        let Some(e) = next else { break };
        used.insert(e);
        order.push(e);
        v = ents.edges[e].other(v);
        verts.push(v);
    }
    (order.len() == set.len()).then_some((order, verts))
}

/// A divide target: its edges and, for an arc, the curve and its arc.
type Target = (Vec<EdgeId>, Option<(CurveId, Arc)>);

/// What Divide works on for edge `e`: its arc curve's edges, or `e` alone when it is
/// straight. `None` for an edge of a free-form curve.
fn divide_target(ents: &Entities, e: EdgeId) -> Option<Target> {
    let edge = ents.edges.get(e)?;
    match edge.attrs.curve {
        None => Some((vec![e], None)),
        Some(c) => {
            let arc = ents.curves.get(c)?.arc?;
            Some((ents.curve_edges(c), Some((c, arc))))
        }
    }
}

/// Whether Divide can split edge `e` (a straight edge or an arc's segment).
pub fn can_divide(ents: &Entities, e: EdgeId) -> bool {
    divide_target(ents, e).is_some_and(|(edges, _)| chain_of(ents, &edges).is_some())
}

/// The `n + 1` points Divide makes on edge `e` (or its arc), ends included, in chain
/// order. `None` when it cannot be divided.
pub fn divide_points(ents: &Entities, e: EdgeId, n: usize) -> Option<Vec<DVec3>> {
    if n == 0 {
        return None;
    }
    let (edges, arc) = divide_target(ents, e)?;
    let (_, verts) = chain_of(ents, &edges)?;
    let a = ents.pos(verts[0]);
    let b = ents.pos(*verts.last()?);
    Some(match arc {
        None => (0..=n).map(|i| a.lerp(b, i as f64 / n as f64)).collect(),
        Some((_, arc)) => {
            let mut pts = Arc { segments: n, ..arc }.points();
            // The chain may run from the arc's end to its start.
            if pts.first().is_some_and(|p| p.distance(a) > b.distance(*p)) && !arc.is_circle() {
                pts.reverse();
            }
            if arc.is_circle() {
                // Start the circle's points at the chain's first vertex and run them
                // the way the chain runs.
                let k = (0..n)
                    .min_by(|&i, &j| pts[i].distance(a).total_cmp(&pts[j].distance(a)))
                    .unwrap_or(0);
                let mut rot: Vec<DVec3> = (0..n).map(|i| pts[(i + k) % n]).collect();
                let along = ents.pos(verts[1]) - a;
                if n > 1 && (rot[1] - rot[0]).dot(along) < 0.0 {
                    rot[1..].reverse();
                }
                rot.push(rot[0]);
                pts = rot;
            }
            pts[0] = a;
            *pts.last_mut()? = b;
            pts
        }
    })
}

/// Divide: splits edge `e` (or its arc) into `n` equal pieces. Returns the new edges in
/// order, empty when nothing changed.
pub fn divide(ents: &mut Entities, e: EdgeId, n: usize) -> Vec<EdgeId> {
    let n = n.min(MAX_DIVISIONS);
    let Some((edges, arc)) = divide_target(ents, e) else {
        return Vec::new();
    };
    let Some((order, verts)) = chain_of(ents, &edges) else {
        return Vec::new();
    };
    let closed = verts.first() == verts.last();
    if n < 2 || (closed && n < 3) || (arc.is_some() && order.len() == n) {
        return Vec::new();
    }
    let Some(points) = divide_points(ents, e, n) else {
        return Vec::new();
    };
    let attrs = ents.edges[order[0]].attrs.clone();
    let new = replace_chain(ents, &order, &verts, &points, attrs);
    if let Some((c, arc)) = arc
        && !new.is_empty()
        && let Some(curve) = ents.curves.get_mut(c)
    {
        curve.arc = Some(Arc { segments: n, ..arc });
    }
    new
}

/// Replaces the chain `order` (through `verts`) by new edges through `points` (whose
/// first and last points are the chain's end vertices, kept), in every face loop that
/// uses it. Interior vertices of the chain must have no other edges. Returns the new
/// edges in chain order (empty if the chain cannot be replaced).
fn replace_chain(
    ents: &mut Entities,
    order: &[EdgeId],
    verts: &[VertexId],
    points: &[DVec3],
    attrs: crate::entities::EdgeAttrs,
) -> Vec<EdgeId> {
    let closed = verts.first() == verts.last();
    let interior = &verts[1..verts.len() - 1];
    if points.len() < 2 || interior.iter().any(|&v| ents.vertices[v].edges.len() != 2) {
        return Vec::new();
    }
    if closed && ents.vertices[verts[0]].edges.len() != 2 {
        return Vec::new();
    }
    // Direction each chain edge runs along the chain.
    let rev: HashMap<EdgeId, bool> = order
        .iter()
        .zip(verts)
        .map(|(&e, &v)| (e, ents.edges[e].start != v))
        .collect();
    let faces: Vec<FaceId> = {
        let mut f: Vec<FaceId> = order
            .iter()
            .flat_map(|&e| ents.edges[e].faces.clone())
            .collect();
        f.sort();
        f.dedup();
        f
    };
    // New vertices and edges.
    let first = verts[0];
    let last = *verts.last().expect("two or more vertices");
    let mut nv = vec![first];
    for p in &points[1..points.len() - 1] {
        nv.push(ents.add_vertex(*p));
    }
    nv.push(last);
    let new: Vec<EdgeId> = nv
        .windows(2)
        .map(|w| ents.add_raw_edge(w[0], w[1], attrs.clone()))
        .collect();
    let forward: Vec<CoEdge> = new
        .iter()
        .map(|&edge| CoEdge {
            edge,
            reversed: false,
        })
        .collect();
    let backward: Vec<CoEdge> = new
        .iter()
        .rev()
        .map(|&edge| CoEdge {
            edge,
            reversed: true,
        })
        .collect();
    for f in faces {
        for l in ents.faces[f].loops_mut() {
            let Some(k) = l.coedges.iter().position(|c| rev.contains_key(&c.edge)) else {
                continue;
            };
            let c = l.coedges[k];
            let along = c.reversed == rev[&c.edge];
            let replacement = if along { &forward } else { &backward };
            // Rotate so the loop starts outside the chain (a loop that is all chain is
            // replaced whole).
            let start = l.coedges.iter().position(|c| !rev.contains_key(&c.edge));
            let Some(start) = start else {
                *l = Loop {
                    coedges: replacement.clone(),
                };
                continue;
            };
            let n = l.coedges.len();
            let mut out = Vec::with_capacity(n + replacement.len());
            let mut placed = false;
            for i in 0..n {
                let c = l.coedges[(start + i) % n];
                if rev.contains_key(&c.edge) {
                    if !placed {
                        out.extend_from_slice(replacement);
                        placed = true;
                    }
                } else {
                    out.push(c);
                }
            }
            l.coedges = out;
        }
        for &e in &new {
            ents.edges[e].faces.push(f);
        }
    }
    for &e in order {
        ents.edges[e].faces.clear();
        ents.unlink_edge(e);
    }
    for &v in interior {
        ents.remove_vertex_if_unused(v);
    }
    let faces: Vec<FaceId> = new
        .iter()
        .flat_map(|&e| ents.edges[e].faces.clone())
        .collect();
    for f in faces {
        ents.refresh_face_bounds(f);
    }
    new
}

/// Weld Edges: joins each connected chain of `edges` (two or more edges, no branches)
/// into one new free-form curve. Returns the new curves.
pub fn weld_edges(ents: &mut Entities, edges: &[EdgeId]) -> Vec<CurveId> {
    let set: HashSet<EdgeId> = edges
        .iter()
        .copied()
        .filter(|e| ents.edges.contains_key(*e))
        .collect();
    // Connected components through shared vertices.
    let mut out = Vec::new();
    let mut seen: HashSet<EdgeId> = HashSet::new();
    let mut sorted: Vec<EdgeId> = set.iter().copied().collect();
    sorted.sort();
    for start in sorted {
        if !seen.insert(start) {
            continue;
        }
        let mut comp = vec![start];
        let mut stack = vec![start];
        while let Some(e) = stack.pop() {
            let edge = &ents.edges[e];
            for v in [edge.start, edge.end] {
                for &x in &ents.vertices[v].edges {
                    if set.contains(&x) && seen.insert(x) {
                        comp.push(x);
                        stack.push(x);
                    }
                }
            }
        }
        if comp.len() < 2 || chain_of(ents, &comp).is_none() {
            continue;
        }
        let curve = ents.curves.insert(Curve { arc: None });
        let mut old: HashSet<CurveId> = HashSet::new();
        for &e in &comp {
            let a = &mut ents.edges[e].attrs;
            if let Some(c) = a.curve.replace(curve) {
                old.insert(c);
            }
        }
        for c in old {
            if ents.curve_edges(c).is_empty() {
                ents.curves.remove(c);
            } else if let Some(x) = ents.curves.get_mut(c) {
                // What is left of a partly welded arc is no longer the whole arc.
                x.arc = None;
            }
        }
        out.push(curve);
    }
    out
}

/// Explode Curve: the curves of `edges` become separate edges. Returns whether any
/// curve was exploded.
pub fn explode_curves(ents: &mut Entities, edges: &[EdgeId]) -> bool {
    let curves: HashSet<CurveId> = edges
        .iter()
        .filter_map(|e| ents.edges.get(*e)?.attrs.curve)
        .collect();
    for &c in &curves {
        for e in ents.curve_edges(c) {
            ents.edges[e].attrs.curve = None;
        }
        ents.curves.remove(c);
    }
    !curves.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Checks, add_curve, add_line, validate};

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn check(ents: &Entities) {
        let errs = validate(ents, Checks::ALL);
        assert!(errs.is_empty(), "invalid: {errs:#?}");
    }

    fn rect(e: &mut Entities) {
        let c = [
            p(0.0, 0.0, 0.0),
            p(12.0, 0.0, 0.0),
            p(12.0, 6.0, 0.0),
            p(0.0, 6.0, 0.0),
        ];
        for i in 0..4 {
            add_line(e, c[i], c[(i + 1) % 4]);
        }
        assert_eq!(e.faces.len(), 1);
    }

    fn length(e: &Entities, id: EdgeId) -> f64 {
        let x = &e.edges[id];
        e.pos(x.start).distance(e.pos(x.end))
    }

    fn arc(n: usize, sweep: f64) -> Arc {
        Arc {
            center: p(0.0, 0.0, 0.0),
            normal: DVec3::Z,
            x_axis: DVec3::X,
            radius: 10.0,
            start_angle: 0.0,
            end_angle: sweep,
            segments: n,
        }
    }

    #[test]
    fn divide_a_face_edge_into_equal_edges() {
        let mut e = Entities::new();
        rect(&mut e);
        let bottom = e
            .edges
            .keys()
            .find(|&id| length(&e, id) > 11.0 && e.pos(e.edges[id].start).y == 0.0)
            .unwrap();
        let pts = divide_points(&e, bottom, 4).unwrap();
        assert_eq!(pts.len(), 5);
        let new = divide(&mut e, bottom, 4);
        assert_eq!(new.len(), 4);
        for &x in &new {
            assert!((length(&e, x) - 3.0).abs() < 1e-9);
            assert_eq!(e.edges[x].faces.len(), 1);
        }
        assert_eq!(e.edges.len(), 7);
        assert_eq!(e.faces.len(), 1);
        let f = e.faces.keys().next().unwrap();
        assert_eq!(e.faces[f].outer.coedges.len(), 7);
        check(&e);
        // One piece: nothing to do.
        assert!(divide(&mut e, new[0], 1).is_empty());
    }

    #[test]
    fn divide_an_arc_keeps_one_curve() {
        let mut e = Entities::new();
        let a = arc(12, std::f64::consts::PI);
        let edges = add_curve(&mut e, &a.points(), Some(a));
        // Close it with the diameter: a half disc face.
        add_line(&mut e, p(-10.0, 0.0, 0.0), p(10.0, 0.0, 0.0));
        assert_eq!(e.faces.len(), 1);
        let new = divide(&mut e, edges[3], 5);
        assert_eq!(new.len(), 5);
        let c = e.edges[new[0]].attrs.curve.unwrap();
        assert_eq!(e.curve_edges(c).len(), 5);
        assert_eq!(e.curves[c].arc.unwrap().segments, 5);
        let l0 = length(&e, new[0]);
        for &x in &new {
            assert!((length(&e, x) - l0).abs() < 1e-9);
            assert_eq!(e.edges[x].faces.len(), 1);
            // Points stay on the circle.
            let edge = &e.edges[x];
            assert!((e.pos(edge.start).length() - 10.0).abs() < 1e-9);
        }
        assert_eq!(e.edges.len(), 6);
        check(&e);
    }

    #[test]
    fn divide_a_circle_with_a_face() {
        let mut e = Entities::new();
        let a = arc(24, std::f64::consts::TAU);
        let edges = add_curve(&mut e, &a.points(), Some(a));
        assert_eq!(e.faces.len(), 1);
        let new = divide(&mut e, edges[0], 8);
        assert_eq!(new.len(), 8);
        assert_eq!(e.edges.len(), 8);
        assert_eq!(e.vertices.len(), 8);
        check(&e);
    }

    #[test]
    fn weld_and_explode_round_trip() {
        let mut e = Entities::new();
        let pts = [
            p(0.0, 0.0, 0.0),
            p(5.0, 0.0, 0.0),
            p(8.0, 3.0, 0.0),
            p(8.0, 9.0, 0.0),
        ];
        let mut edges = Vec::new();
        for w in pts.windows(2) {
            edges.extend(add_line(&mut e, w[0], w[1]));
        }
        let curves = weld_edges(&mut e, &edges);
        assert_eq!(curves.len(), 1);
        assert_eq!(e.curve_edges(curves[0]).len(), 3);
        check(&e);
        assert!(explode_curves(&mut e, &edges[..1]));
        assert!(e.curves.is_empty());
        assert!(e.edges.values().all(|x| x.attrs.curve.is_none()));
        assert_eq!(e.edges.len(), 3);
        check(&e);
        // A branch is not welded.
        let extra = add_line(&mut e, p(5.0, 0.0, 0.0), p(5.0, -4.0, 0.0));
        let mut all = edges.clone();
        all.extend(extra);
        assert!(weld_edges(&mut e, &all).is_empty());
        // A single edge is not a curve.
        assert!(weld_edges(&mut e, &edges[..1]).is_empty());
    }

    #[test]
    fn weld_part_of_an_arc_drops_its_arc() {
        let mut e = Entities::new();
        let a = arc(6, std::f64::consts::FRAC_PI_2);
        let edges = add_curve(&mut e, &a.points(), Some(a));
        let old = e.edges[edges[0]].attrs.curve.unwrap();
        let tail = add_line(&mut e, a.points()[6], p(0.0, 20.0, 0.0));
        let mut sel = edges[4..].to_vec();
        sel.extend(tail);
        assert_eq!(weld_edges(&mut e, &sel).len(), 1);
        assert_eq!(e.curves[old].arc, None);
        assert_eq!(e.curve_edges(old).len(), 4);
        // An arc that is no longer one cannot be divided.
        assert!(!can_divide(&e, edges[0]));
        check(&e);
    }
}
