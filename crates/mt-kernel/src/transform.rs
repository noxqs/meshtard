//! Move, Rotate, Scale and their copies: transforming vertices with SketchUp's stretching,
//! welding and folding, and copying geometry so it merges with what is there.
//!
//! Two strategies:
//! - in place: the vertices move and nothing else changes (ids are kept), used when a
//!   local check shows the result is valid (faces stay planar, nothing touches);
//! - rebuild: the touched edges and faces are taken out and inserted again through the
//!   sticky-geometry path (`ops::insert_line`), so welding, splitting and merging come
//!   from the same code as drawing. Faces that became non-planar are folded.

use std::collections::{HashMap, HashSet};

use glam::{DMat4, DVec2, DVec3};

use crate::TOL;
use crate::arrangement;
use crate::entities::{
    Arc, CoEdge, Curve, CurveId, EdgeAttrs, EdgeId, Entities, FaceAttrs, FaceId, Loop, VertexId,
};
use crate::geom::{Plane, point_in_polygon, point_segment, segment_segment, signed_area2};
use crate::ops::{face_from_cell, face_is_sound, insert_line, update_faces};

/// Vertices used by the given edges and faces (all loops), without duplicates.
pub fn vertices_of(ents: &Entities, edges: &[EdgeId], faces: &[FaceId]) -> Vec<VertexId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |v: VertexId| {
        if seen.insert(v) {
            out.push(v);
        }
    };
    for e in edges.iter().filter_map(|&e| ents.edges.get(e)) {
        push(e.start);
        push(e.end);
    }
    for f in faces.iter().filter_map(|&f| ents.faces.get(f)) {
        for l in f.loops() {
            for v in ents.loop_vertices(l) {
                push(v);
            }
        }
    }
    out
}

/// Applies `xf` to the vertices (connected geometry stretches, as SketchUp's Move does),
/// then welds each moved vertex onto any other vertex within TOL (merging duplicate
/// edges and rewiring face loops), and folds faces that became non-planar.
pub fn transform_vertices(ents: &mut Entities, vertices: &[VertexId], xf: DMat4) {
    let moves: HashMap<VertexId, DVec3> = vertices
        .iter()
        .filter_map(|&v| Some((v, xf.transform_point3(ents.vertices.get(v)?.pos))))
        .collect();
    if moves.is_empty() {
        return;
    }
    // A mirror turns loops around: faces that move as a whole are reversed so they keep
    // looking the same way relative to the geometry.
    let flip: Vec<FaceId> = if is_mirror(&xf) {
        let mut faces: Vec<FaceId> = moves
            .keys()
            .flat_map(|v| ents.vertices[*v].edges.iter())
            .flat_map(|e| ents.edges[*e].faces.iter().copied())
            .collect();
        faces.sort();
        faces.dedup();
        faces.retain(|&f| {
            ents.faces[f]
                .loops()
                .all(|l| ents.loop_vertices(l).iter().all(|v| moves.contains_key(v)))
        });
        faces
    } else {
        Vec::new()
    };
    update_arcs(ents, &moves, xf);
    if !try_in_place(ents, &moves, &flip) {
        rebuild(ents, &moves, &flip);
        drop_unsound_faces(ents);
    }
}

fn is_mirror(xf: &DMat4) -> bool {
    glam::DMat3::from_mat4(*xf).determinant() < 0.0
}

/// Copies the edges and faces (Ctrl+Move / Ctrl+Rotate) transformed by `xf`. The copy
/// merges with existing geometry like drawing does, keeps face materials/attributes and
/// curve membership (new curve ids for copied curves). Returns the new edges and faces.
pub fn copy_geometry(
    ents: &mut Entities,
    edges: &[EdgeId],
    faces: &[FaceId],
    xf: DMat4,
) -> (Vec<EdgeId>, Vec<FaceId>) {
    let faces: Vec<FaceId> = faces
        .iter()
        .copied()
        .filter(|&f| ents.faces.contains_key(f))
        .collect();
    let mut all_edges: Vec<EdgeId> = Vec::new();
    let mut seen = HashSet::new();
    for e in edges
        .iter()
        .copied()
        .chain(faces.iter().flat_map(|&f| face_edges(ents, f)))
    {
        if ents.edges.contains_key(e) && seen.insert(e) {
            all_edges.push(e);
        }
    }

    // New curves for copied curves.
    let mut curve_map: HashMap<CurveId, CurveId> = HashMap::new();
    for &e in &all_edges {
        if let Some(c) = ents.edges[e].attrs.curve
            && !curve_map.contains_key(&c)
            && let Some(curve) = ents.curves.get(c)
        {
            let arc = curve.arc.and_then(|a| transform_arc(&a, &xf));
            curve_map.insert(c, ents.curves.insert(Curve { arc }));
        }
    }
    let segments: Vec<(DVec3, DVec3, EdgeAttrs)> = all_edges
        .iter()
        .map(|&e| {
            let edge = &ents.edges[e];
            let mut attrs = edge.attrs.clone();
            attrs.curve = attrs.curve.and_then(|c| curve_map.get(&c).copied());
            (
                xf.transform_point3(ents.pos(edge.start)),
                xf.transform_point3(ents.pos(edge.end)),
                attrs,
            )
        })
        .collect();
    let records: Vec<FaceRecord> = faces
        .iter()
        .map(|&f| {
            let face = &ents.faces[f];
            FaceRecord {
                loops: face
                    .loops()
                    .map(|l| {
                        let mut pts: Vec<DVec3> = ents
                            .loop_points(l)
                            .into_iter()
                            .map(|p| xf.transform_point3(p))
                            .collect();
                        if is_mirror(&xf) {
                            pts.reverse();
                        }
                        pts
                    })
                    .collect(),
                attrs: face.attrs.clone(),
            }
        })
        .collect();

    let mut created = Vec::new();
    for (a, b, attrs) in &segments {
        let (_, new) = insert_line(ents, *a, *b, false);
        for &e in &new {
            ents.edges[e].attrs = attrs.clone();
        }
        created.extend(new);
    }
    let desired = desired_faces(ents, &records);
    let new_faces: Vec<FaceId> = fill(ents, &desired, false).into_iter().flatten().collect();
    // Loops closed by copied edges alone get faces, as when drawing.
    created.retain(|&e| ents.edges.contains_key(e));
    update_faces(ents, &created, true);
    drop_unsound_faces(ents);

    let on_copy = |p: DVec3| {
        segments
            .iter()
            .any(|(a, b, _)| point_segment(p, *a, *b).0 <= TOL)
    };
    let new_edges: Vec<EdgeId> = ents
        .edges
        .iter()
        .filter(|(_, e)| {
            let (a, b) = (ents.pos(e.start), ents.pos(e.end));
            on_copy(a) && on_copy(b) && on_copy((a + b) / 2.0)
        })
        .map(|(id, _)| id)
        .collect();
    for c in curve_map.into_values() {
        if ents.curve_edges(c).is_empty() {
            ents.curves.remove(c);
        }
    }
    let new_faces = new_faces
        .into_iter()
        .filter(|&f| ents.faces.contains_key(f))
        .collect();
    (new_edges, new_faces)
}

// ---------------------------------------------------------------------------------------
// Shared machinery (also used by push/pull).

/// A face to (re)create: the cells of `plane` whose sample lies inside one of `polys`
/// get a face with the plane's normal as front and `attrs`.
pub(crate) struct Desired {
    pub plane: Plane,
    /// Union of polygons (outer loop, holes), in 3D.
    pub polys: Vec<(Vec<DVec3>, Vec<Vec<DVec3>>)>,
    pub attrs: FaceAttrs,
}

impl Desired {
    fn contains(&self, p: DVec3) -> bool {
        let q = self.plane.project(p);
        let proj =
            |l: &Vec<DVec3>| -> Vec<DVec2> { l.iter().map(|x| self.plane.project(*x)).collect() };
        self.polys.iter().any(|(outer, holes)| {
            point_in_polygon(q, &proj(outer))
                && !holes.iter().any(|h| point_in_polygon(q, &proj(h)))
        })
    }
}

/// A face's loops as points (outer first) and attributes.
pub(crate) struct FaceRecord {
    pub loops: Vec<Vec<DVec3>>,
    pub attrs: FaceAttrs,
}

/// Creates faces for the desired regions from the cells of their planes. A cell already
/// covered by a face is left alone, unless `cancel_opposite` is set and that face looks
/// the other way: two coplanar faces back to back cancel (both go), as when Push/Pull
/// cuts through. Returns the faces created for each desired region.
pub(crate) fn fill(
    ents: &mut Entities,
    desired: &[Desired],
    cancel_opposite: bool,
) -> Vec<Vec<FaceId>> {
    let mut out = Vec::new();
    let mut orphans: Vec<EdgeId> = Vec::new();
    for d in desired {
        let mut made = Vec::new();
        let edges = arrangement::edges_in_plane(ents, &d.plane);
        let cells = arrangement::cells(ents, &d.plane, &edges);
        for cell in cells.iter().filter(|c| d.contains(c.sample)) {
            let taken = ents
                .faces
                .keys()
                .find(|&g| ents.face_contains(g, cell.sample));
            match taken {
                None => {
                    let f = face_from_cell(ents, &d.plane, cell, d.plane.normal, d.attrs.clone());
                    if face_is_sound(ents, f) {
                        made.push(f);
                    } else {
                        ents.remove_face(f);
                    }
                }
                Some(g) if cancel_opposite && ents.faces[g].normal.dot(d.plane.normal) < 0.0 => {
                    orphans.extend(face_edges(ents, g));
                    ents.remove_face(g);
                }
                Some(_) => {}
            }
        }
        out.push(made);
    }
    // Edges left without faces by cancelled faces go too.
    for e in orphans {
        if ents.edges.get(e).is_some_and(|x| x.faces.is_empty()) {
            let (a, b) = (ents.edges[e].start, ents.edges[e].end);
            ents.remove_edge(e);
            ents.remove_vertex_if_unused(a);
            ents.remove_vertex_if_unused(b);
        }
    }
    out
}

/// Turns face records into desired regions: one per planar face; non-planar faces are
/// folded into planar pieces along new soft and smooth edges (inserted here).
pub(crate) fn desired_faces(ents: &mut Entities, records: &[FaceRecord]) -> Vec<Desired> {
    let mut out = Vec::new();
    let mut folds: Vec<(DVec3, DVec3)> = Vec::new();
    for rec in records {
        let Some(fit) = Plane::from_polygon(&rec.loops[0]) else {
            continue;
        };
        if rec
            .loops
            .iter()
            .flatten()
            .all(|p| fit.distance(*p).abs() <= TOL / 2.0)
        {
            out.push(Desired {
                plane: fit,
                polys: vec![(rec.loops[0].clone(), rec.loops[1..].to_vec())],
                attrs: rec.attrs.clone(),
            });
        } else {
            fold(rec, &fit, &mut out, &mut folds);
        }
    }
    let attrs = EdgeAttrs {
        soft: true,
        smooth: true,
        ..Default::default()
    };
    for (a, b) in folds {
        for e in insert_line(ents, a, b, false).1 {
            ents.edges[e].attrs = attrs.clone();
        }
    }
    out
}

/// Splits a non-planar face into planar pieces: ear-clipping triangles grouped with
/// coplanar neighbors. Diagonals between pieces are pushed to `folds`.
fn fold(rec: &FaceRecord, fit: &Plane, out: &mut Vec<Desired>, folds: &mut Vec<(DVec3, DVec3)>) {
    let pts: Vec<DVec3> = rec.loops.iter().flatten().copied().collect();
    let mut flat = Vec::with_capacity(pts.len() * 2);
    for p in &pts {
        let q = fit.project(*p);
        flat.extend([q.x, q.y]);
    }
    let mut starts = Vec::new();
    let mut boundary: HashSet<(usize, usize)> = HashSet::new();
    let mut at = 0;
    for (i, l) in rec.loops.iter().enumerate() {
        if i > 0 {
            starts.push(at);
        }
        for k in 0..l.len() {
            let (a, b) = (at + k, at + (k + 1) % l.len());
            boundary.insert((a.min(b), a.max(b)));
        }
        at += l.len();
    }
    let idx = earcutr::earcut(&flat, &starts, 2).unwrap_or_default();
    // Triangles with their oriented plane.
    let tris: Vec<([usize; 3], Plane)> = idx
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| {
            let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
            let n = (b - a).cross(c - a);
            if n.length() <= 1e-9 {
                return None;
            }
            let n = if n.dot(fit.normal) < 0.0 { -n } else { n };
            Some(([t[0], t[1], t[2]], Plane::new(n, a)))
        })
        .collect();
    let mut by_side: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, (t, _)) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            by_side.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    // Grow coplanar groups across shared sides.
    let mut group = vec![usize::MAX; tris.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for seed in 0..tris.len() {
        if group[seed] != usize::MAX {
            continue;
        }
        let g = groups.len();
        let plane = tris[seed].1;
        group[seed] = g;
        let mut members = vec![seed];
        let mut stack = vec![seed];
        while let Some(i) = stack.pop() {
            let t = tris[i].0;
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                for &j in &by_side[&(a.min(b), a.max(b))] {
                    let fits = tris[j].1.normal.dot(plane.normal) > 0.0
                        && tris[j]
                            .0
                            .iter()
                            .all(|&v| plane.distance(pts[v]).abs() <= TOL / 2.0);
                    if group[j] == usize::MAX && fits {
                        group[j] = g;
                        members.push(j);
                        stack.push(j);
                    }
                }
            }
        }
        groups.push(members);
    }
    for (side, ts) in &by_side {
        if ts.len() == 2 && group[ts[0]] != group[ts[1]] && !boundary.contains(side) {
            folds.push((pts[side.0], pts[side.1]));
        }
    }
    for members in groups {
        let plane = tris[members[0]].1;
        out.push(Desired {
            plane,
            polys: members
                .iter()
                .map(|&i| (tris[i].0.iter().map(|&v| pts[v]).collect(), Vec::new()))
                .collect(),
            attrs: rec.attrs.clone(),
        });
    }
}

pub(crate) fn face_edges(ents: &Entities, f: FaceId) -> Vec<EdgeId> {
    let mut v: Vec<EdgeId> = ents.faces[f]
        .loops()
        .flat_map(|l| l.coedges.iter().map(|c| c.edge))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// A point strictly inside the face.
pub(crate) fn face_sample(ents: &Entities, f: FaceId) -> DVec3 {
    let plane = ents.face_plane(f);
    let (outer, holes) = ents.face_polygons(f, &plane);
    let holes: Vec<&Vec<DVec2>> = holes.iter().collect();
    let q = arrangement::interior_point(&outer, &holes);
    let (u, v) = plane.basis();
    u * q.x + v * q.y + plane.normal * plane.d
}

/// Removes faces that tolerance effects left unsound: a loop visiting a vertex twice (a
/// sliver folded onto itself), off its plane, or wound the wrong way. Removing a face
/// never breaks the other invariants; these faces are narrower than the tolerance.
pub(crate) fn drop_unsound_faces(ents: &mut Entities) {
    let bad: Vec<FaceId> = ents
        .faces
        .iter()
        .filter(|(f, face)| {
            let repeats = face.loops().any(|l| {
                let mut vs = ents.loop_vertices(l);
                vs.sort();
                vs.windows(2).any(|w| w[0] == w[1])
            });
            repeats || !face_is_sound(ents, *f)
        })
        .map(|(f, _)| f)
        .collect();
    for f in bad {
        ents.remove_face(f);
    }
}

/// Reverses a face: loops run the other way and the normal flips.
pub(crate) fn flip_face(ents: &mut Entities, f: FaceId) {
    let face = &mut ents.faces[f];
    face.normal = -face.normal;
    for l in face.loops_mut() {
        l.coedges = l
            .coedges
            .iter()
            .rev()
            .map(|c| CoEdge {
                edge: c.edge,
                reversed: !c.reversed,
            })
            .collect();
    }
}

fn face_is_planar(ents: &Entities, f: FaceId, tol: f64) -> bool {
    let plane = ents.face_plane(f);
    ents.faces[f].loops().all(|l| {
        ents.loop_points(l)
            .iter()
            .all(|p| plane.distance(*p).abs() <= tol)
    })
}

/// The arc transformed by `xf`, or `None` when `xf` does not keep it circular.
fn transform_arc(a: &Arc, xf: &DMat4) -> Option<Arc> {
    let x = xf.transform_vector3(a.x_axis * a.radius);
    let y = xf.transform_vector3(a.y_axis() * a.radius);
    let r = x.length();
    if r <= 1e-12 || (y.length() - r).abs() > 1e-9 * r.max(1.0) || x.dot(y).abs() > 1e-9 * r * r {
        return None;
    }
    Some(Arc {
        center: xf.transform_point3(a.center),
        normal: x.cross(y).normalize(),
        x_axis: x / r,
        radius: r,
        ..*a
    })
}

/// Curves touched by the move keep their arc only if all of their vertices move.
fn update_arcs(ents: &mut Entities, moves: &HashMap<VertexId, DVec3>, xf: DMat4) {
    let touched: HashSet<CurveId> = moves
        .keys()
        .flat_map(|v| ents.vertices[*v].edges.iter())
        .filter_map(|e| ents.edges[*e].attrs.curve)
        .collect();
    for c in touched {
        let Some(arc) = ents.curves.get(c).and_then(|x| x.arc) else {
            continue;
        };
        let all_moved = ents.curve_edges(c).iter().all(|&e| {
            moves.contains_key(&ents.edges[e].start) && moves.contains_key(&ents.edges[e].end)
        });
        let new = if all_moved {
            transform_arc(&arc, &xf)
        } else {
            None
        };
        ents.curves[c].arc = new;
    }
}

/// Moves the vertices if the result needs no topology change; otherwise leaves the
/// entities untouched and returns false.
fn try_in_place(ents: &mut Entities, moves: &HashMap<VertexId, DVec3>, flip: &[FaceId]) -> bool {
    let old: Vec<(VertexId, DVec3)> = moves.keys().map(|&v| (v, ents.pos(v))).collect();
    for &f in flip {
        flip_face(ents, f);
    }
    for (&v, &p) in moves {
        ents.vertices[v].pos = p;
    }
    let mut edges: Vec<EdgeId> = moves
        .keys()
        .flat_map(|v| ents.vertices[*v].edges.iter().copied())
        .collect();
    edges.sort();
    edges.dedup();
    let mut faces: Vec<FaceId> = edges
        .iter()
        .flat_map(|e| ents.edges[*e].faces.iter().copied())
        .collect();
    faces.sort();
    faces.dedup();
    let normals: Vec<(FaceId, DVec3)> = faces.iter().map(|&f| (f, ents.faces[f].normal)).collect();
    let ok = in_place_ok(ents, moves, &edges, &faces);
    if !ok {
        for (v, p) in old {
            ents.vertices[v].pos = p;
        }
        for (f, n) in normals {
            ents.faces[f].normal = n;
        }
        for &f in flip {
            flip_face(ents, f);
        }
    }
    ok
}

fn in_place_ok(
    ents: &mut Entities,
    moves: &HashMap<VertexId, DVec3>,
    edges: &[EdgeId],
    faces: &[FaceId],
) -> bool {
    // Moved vertices stay apart from all others and off all edges.
    for &v in moves.keys() {
        let p = ents.pos(v);
        if ents
            .vertices
            .iter()
            .any(|(w, x)| w != v && x.pos.distance(p) <= TOL)
        {
            return false;
        }
        let on_edge = ents.edges.values().any(|e| {
            e.start != v
                && e.end != v
                && point_segment(p, ents.pos(e.start), ents.pos(e.end)).0 <= TOL
        });
        if on_edge {
            return false;
        }
    }
    // Moved edges cross nothing and pass through no vertex.
    for &a in edges {
        let ea = &ents.edges[a];
        let (p0, p1) = (ents.pos(ea.start), ents.pos(ea.end));
        let crosses = ents.edges.iter().any(|(b, eb)| {
            if b == a
                || [eb.start, eb.end]
                    .iter()
                    .any(|v| *v == ea.start || *v == ea.end)
            {
                return false;
            }
            segment_segment(p0, p1, ents.pos(eb.start), ents.pos(eb.end))
                .is_some_and(|(d, _, _)| d <= TOL)
        });
        let through = ents
            .vertices
            .iter()
            .any(|(w, x)| w != ea.start && w != ea.end && point_segment(x.pos, p0, p1).0 <= TOL);
        if crosses || through {
            return false;
        }
    }
    // Faces stay planar and well wound; their normals follow.
    for &f in faces {
        let outer = ents.loop_points(&ents.faces[f].outer);
        let Some(fit) = Plane::from_polygon(&outer) else {
            return false;
        };
        ents.faces[f].normal = fit.normal;
        let plane = ents.face_plane(f);
        if !face_is_planar(ents, f, TOL / 2.0) {
            return false;
        }
        let (outer, holes) = ents.face_polygons(f, &plane);
        if signed_area2(&outer) <= 0.0 {
            return false;
        }
        for h in &holes {
            if signed_area2(h) >= 0.0 || !point_in_polygon(h[0], &outer) {
                return false;
            }
        }
    }
    // No coplanar overlaps with the moved faces or edges.
    for &f in faces {
        let s = face_sample(ents, f);
        if ents
            .faces
            .keys()
            .any(|g| g != f && ents.face_contains(g, s))
        {
            return false;
        }
        let plane = ents.face_plane(f);
        let mine = face_edges(ents, f);
        for (g, _) in ents.faces.iter() {
            if g != f
                && ents.face_plane(g).coincides(&plane)
                && ents.face_contains(f, face_sample(ents, g))
            {
                return false;
            }
        }
        let covered = ents.edges.iter().any(|(e, edge)| {
            let (a, b) = (ents.pos(edge.start), ents.pos(edge.end));
            !mine.contains(&e)
                && plane.contains(a)
                && plane.contains(b)
                && ents.face_contains(f, (a + b) / 2.0)
        });
        if covered {
            return false;
        }
    }
    for &a in edges {
        let edge = &ents.edges[a];
        let (p, q) = (ents.pos(edge.start), ents.pos(edge.end));
        let inside = ents.faces.keys().any(|g| {
            !edge.faces.contains(&g)
                && ents.face_plane(g).contains(p)
                && ents.face_plane(g).contains(q)
                && ents.face_contains(g, (p + q) / 2.0)
        });
        if inside {
            return false;
        }
    }
    true
}

/// Takes out every edge and face touching the moved vertices and inserts them again at
/// the new positions through the sticky-geometry path.
fn rebuild(ents: &mut Entities, moves: &HashMap<VertexId, DVec3>, flip: &[FaceId]) {
    let pos = |ents: &Entities, v: VertexId| moves.get(&v).copied().unwrap_or(ents.pos(v));
    let mut edges: Vec<EdgeId> = moves
        .keys()
        .flat_map(|v| ents.vertices[*v].edges.iter().copied())
        .collect();
    edges.sort();
    edges.dedup();
    let mut faces: Vec<FaceId> = edges
        .iter()
        .flat_map(|e| ents.edges[*e].faces.iter().copied())
        .collect();
    faces.sort();
    faces.dedup();

    let segments: Vec<(DVec3, DVec3, EdgeAttrs)> = edges
        .iter()
        .map(|&e| {
            let edge = &ents.edges[e];
            (
                pos(ents, edge.start),
                pos(ents, edge.end),
                edge.attrs.clone(),
            )
        })
        .collect();
    let records: Vec<FaceRecord> = faces
        .iter()
        .map(|&f| {
            let face = &ents.faces[f];
            let pts = |l: &Loop| -> Vec<DVec3> {
                let mut pts: Vec<DVec3> = ents
                    .loop_vertices(l)
                    .into_iter()
                    .map(|v| pos(ents, v))
                    .collect();
                if flip.contains(&f) {
                    pts.reverse();
                }
                pts
            };
            FaceRecord {
                loops: face.loops().map(pts).collect(),
                attrs: face.attrs.clone(),
            }
        })
        .collect();

    let mut ends = Vec::new();
    for &f in &faces {
        ents.remove_face(f);
    }
    for &e in &edges {
        ends.extend([ents.edges[e].start, ents.edges[e].end]);
        ents.remove_edge(e);
    }
    for v in ends {
        ents.remove_vertex_if_unused(v);
    }

    for (a, b, attrs) in &segments {
        for e in insert_line(ents, *a, *b, false).1 {
            ents.edges[e].attrs = attrs.clone();
        }
    }
    let desired = desired_faces(ents, &records);
    fill(ents, &desired, false);

    let live: HashSet<CurveId> = ents.edges.values().filter_map(|e| e.attrs.curve).collect();
    ents.curves.retain(|c, _| live.contains(&c));
}
