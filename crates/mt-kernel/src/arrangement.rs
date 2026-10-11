//! Regions enclosed by edges lying in one plane.
//!
//! Edges in a plane split it into cells. Every face-building operation (auto-face,
//! splitting a face, merging faces) recomputes the cells of the affected plane and
//! creates faces for the cells it wants. Edges never cross (sticky geometry splits them),
//! so the edges form a planar graph and its cells come from a half-edge walk.

use std::collections::BTreeMap;

use glam::{DVec2, DVec3};

use crate::entities::{CoEdge, EdgeId, Entities, VertexId};
use crate::geom::{Plane, point_in_polygon, signed_area2};

/// A bounded region: outer cycle counter-clockwise around the plane normal, hole
/// cycles clockwise.
#[derive(Debug, Clone)]
pub struct Cell {
    pub outer: Vec<CoEdge>,
    pub holes: Vec<Vec<CoEdge>>,
    /// A point strictly inside the region.
    pub sample: DVec3,
    /// Area in square inches.
    pub area: f64,
}

/// Edges lying in `plane`.
pub fn edges_in_plane(ents: &Entities, plane: &Plane) -> Vec<EdgeId> {
    ents.edges_near_plane(plane, crate::ops::PLANE_SLACK)
        .into_iter()
        .filter(|&id| {
            let e = &ents.edges[id];
            plane.contains(ents.pos(e.start)) && plane.contains(ents.pos(e.end))
        })
        .collect()
}

/// Drops edges with a free end, repeatedly: they cannot bound a region.
fn two_core(ents: &Entities, edges: &[EdgeId]) -> Vec<EdgeId> {
    let mut degree: BTreeMap<VertexId, usize> = BTreeMap::new();
    for &e in edges {
        *degree.entry(ents.edges[e].start).or_default() += 1;
        *degree.entry(ents.edges[e].end).or_default() += 1;
    }
    let mut alive: Vec<bool> = vec![true; edges.len()];
    loop {
        let mut changed = false;
        for (i, &e) in edges.iter().enumerate() {
            if !alive[i] {
                continue;
            }
            let edge = &ents.edges[e];
            if degree[&edge.start] == 1 || degree[&edge.end] == 1 || edge.start == edge.end {
                alive[i] = false;
                *degree.get_mut(&edge.start).unwrap() -= 1;
                *degree.get_mut(&edge.end).unwrap() -= 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    edges
        .iter()
        .zip(alive)
        .filter(|(_, a)| *a)
        .map(|(e, _)| *e)
        .collect()
}

fn find(parent: &mut BTreeMap<VertexId, VertexId>, v: VertexId) -> VertexId {
    let p = parent[&v];
    if p == v {
        return v;
    }
    let root = find(parent, p);
    parent.insert(v, root);
    root
}

/// All bounded cells formed by `edges` (which must lie in `plane`).
pub fn cells(ents: &Entities, plane: &Plane, edges: &[EdgeId]) -> Vec<Cell> {
    let edges = two_core(ents, edges);
    if edges.is_empty() {
        return Vec::new();
    }
    let pt = |v: VertexId| plane.project(ents.pos(v));

    // Outgoing half-edges per vertex, sorted counter-clockwise by direction.
    let mut outgoing: BTreeMap<VertexId, Vec<CoEdge>> = BTreeMap::new();
    for &e in &edges {
        let edge = &ents.edges[e];
        outgoing.entry(edge.start).or_default().push(CoEdge {
            edge: e,
            reversed: false,
        });
        outgoing.entry(edge.end).or_default().push(CoEdge {
            edge: e,
            reversed: true,
        });
    }
    for (v, list) in &mut outgoing {
        let origin = pt(*v);
        list.sort_by(|a, b| {
            let angle = |c: &CoEdge| {
                let d = pt(ents.coedge_ends(*c).1) - origin;
                d.y.atan2(d.x)
            };
            angle(a).total_cmp(&angle(b))
        });
    }

    // Walk cycles with the region on the left: after arriving at `b`, leave by the
    // half-edge just clockwise of the way back.
    let next = |h: CoEdge| -> CoEdge {
        let b = ents.coedge_ends(h).1;
        let list = &outgoing[&b];
        let twin = CoEdge {
            edge: h.edge,
            reversed: !h.reversed,
        };
        let i = list
            .iter()
            .position(|c| *c == twin)
            .expect("twin is outgoing at b");
        list[(i + list.len() - 1) % list.len()]
    };
    let mut visited: BTreeMap<CoEdge, bool> = BTreeMap::new();
    let mut cycles: Vec<(Vec<CoEdge>, Vec<DVec2>, f64)> = Vec::new();
    for list in outgoing.values() {
        for &start in list {
            if visited.contains_key(&start) {
                continue;
            }
            let mut cycle = Vec::new();
            let mut h = start;
            loop {
                visited.insert(h, true);
                cycle.push(h);
                h = next(h);
                if h == start || cycle.len() > 4 * edges.len() {
                    break;
                }
            }
            let poly: Vec<DVec2> = cycle.iter().map(|c| pt(ents.coedge_ends(*c).0)).collect();
            let area = signed_area2(&poly) / 2.0;
            cycles.push((cycle, poly, area));
        }
    }

    // Connected components, to attach each component's outline as a hole.
    let mut parent: BTreeMap<VertexId, VertexId> = outgoing.keys().map(|v| (*v, *v)).collect();
    for &e in &edges {
        let (a, b) = (ents.edges[e].start, ents.edges[e].end);
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        parent.insert(ra, rb);
    }
    let component = |parent: &mut BTreeMap<VertexId, VertexId>, c: &[CoEdge]| {
        find(parent, ents.coedge_ends(c[0]).0)
    };

    let mut bounded: Vec<usize> = (0..cycles.len()).filter(|&i| cycles[i].2 > 0.0).collect();
    bounded.sort_by(|&a, &b| cycles[a].2.total_cmp(&cycles[b].2));
    let mut holes: Vec<Vec<usize>> = vec![Vec::new(); cycles.len()];
    for i in (0..cycles.len()).filter(|&i| cycles[i].2 <= 0.0) {
        let comp = component(&mut parent, &cycles[i].0);
        let probe = cycles[i].1[0];
        // Smallest region of another component that contains this outline.
        let container = bounded.iter().copied().find(|&j| {
            component(&mut parent, &cycles[j].0) != comp && point_in_polygon(probe, &cycles[j].1)
        });
        if let Some(j) = container {
            holes[j].push(i);
        }
    }

    let (u, v) = plane.basis();
    let lift = |q: DVec2| u * q.x + v * q.y + plane.normal * plane.d;
    bounded
        .into_iter()
        .map(|i| {
            let hole_polys: Vec<&Vec<DVec2>> = holes[i].iter().map(|&h| &cycles[h].1).collect();
            let area = cycles[i].2
                - hole_polys
                    .iter()
                    .map(|p| -signed_area2(p) / 2.0)
                    .sum::<f64>();
            Cell {
                outer: cycles[i].0.clone(),
                holes: holes[i].iter().map(|&h| cycles[h].0.clone()).collect(),
                sample: lift(interior_point(&cycles[i].1, &hole_polys)),
                area,
            }
        })
        .collect()
}

/// Centroid of the largest triangle of a triangulation that is inside the region.
///
/// Ear clipping can return triangles outside the region when a loop touches itself (a
/// hole around two blocks joined by a bridge edge runs along the bridge twice), so each
/// centroid is checked against the loops, largest triangle first.
pub fn interior_point(outer: &[DVec2], holes: &[&Vec<DVec2>]) -> DVec2 {
    let mut flat: Vec<f64> = outer.iter().flat_map(|p| [p.x, p.y]).collect();
    let mut starts = Vec::new();
    for h in holes {
        starts.push(flat.len() / 2);
        flat.extend(h.iter().flat_map(|p| [p.x, p.y]));
    }
    let at = |i: usize| DVec2::new(flat[2 * i], flat[2 * i + 1]);
    let tris = earcutr::earcut(&flat, &starts, 2).unwrap_or_default();
    let mut tris: Vec<(DVec2, DVec2, DVec2)> = tris
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| (at(t[0]), at(t[1]), at(t[2])))
        .collect();
    let centroid = |(a, b, c): (DVec2, DVec2, DVec2)| (a + b + c) / 3.0;
    let largest = |tris: &[(DVec2, DVec2, DVec2)]| {
        tris.iter()
            .copied()
            .max_by(|a, b| tri_area(*a).total_cmp(&tri_area(*b)))
    };
    let Some(best) = largest(&tris) else {
        return outer[0];
    };
    let inside =
        |q: DVec2| point_in_polygon(q, outer) && !holes.iter().any(|h| point_in_polygon(q, h));
    if inside(centroid(best)) {
        return centroid(best);
    }
    // Rare: try the others, largest first (ties keep the triangulation's order).
    tris.sort_by(|a, b| tri_area(*b).total_cmp(&tri_area(*a)));
    tris.iter()
        .map(|t| centroid(*t))
        .find(|q| inside(*q))
        .unwrap_or(centroid(best))
}

fn tri_area((a, b, c): (DVec2, DVec2, DVec2)) -> f64 {
    (b - a).perp_dot(c - a).abs()
}
