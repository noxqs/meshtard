//! Solid Tools: the solid test and mesh booleans (Outer Shell, Union, Subtract,
//! Intersect, Trim, Split).
//!
//! A boolean copies both solids into one output space, splits each solid's faces where
//! the other solid's faces cross them (and along the other solid's edges where faces are
//! coplanar), classifies every piece as outside, inside, or on the other solid's surface
//! (same or opposite direction), keeps the pieces the operation needs, welds them into
//! one `Entities` and merges coplanar neighbors with the same attributes.
//!
//! Inside/outside is ray parity against the other solid's triangles through a [`Bvh`];
//! a ray that passes within rounding of a triangle edge or vertex is retried along
//! another direction.
//!
//! Operand conventions (all unverified, `spec/parity/TRIAL_CHECKLIST.md`):
//! - [`subtract`]`(first, second)` is `second - first`, as SketchUp's Subtract removes
//!   the first selected solid from the second and deletes the first;
//! - [`trim`]`(first, second)` is the same difference, but the app keeps `first`;
//! - [`split`] returns all three pieces.
//!
//! Faces keep their source face's attributes (material, tag, hidden). Faces without a
//! material take the instance's material ([`SolidInput::material`]) on both sides
//! (unverified). Faces of the cutter that become the walls of a cavity are reversed but
//! keep their front material (unverified). Edge attributes (soft, smooth, curves) are
//! not carried over.

use std::collections::{HashMap, HashSet, VecDeque};

use glam::{DMat4, DVec2, DVec3};

use crate::TOL;
use crate::arrangement;
use crate::bvh::Bvh;
use crate::entities::{
    CoEdge, EdgeAttrs, EdgeId, Entities, FaceAttrs, FaceId, Loop, MaterialId, VertexId,
};
use crate::geom::{point_in_polygon, point_segment, segment_segment};
use crate::intersect::{clip, face_face};
use crate::ops::{face_from_cell, face_is_sound, heal_vertex, insert_line};
use crate::transform::{face_sample, flip_face};

/// Whether `ents` is a solid in SketchUp's sense: it has faces and every edge bounds
/// exactly two faces (a closed 2-manifold surface, no stray edges). Nested instances
/// make it not a solid (unverified: SketchUp's rule for nested groups).
pub fn is_solid(ents: &Entities) -> bool {
    !ents.faces.is_empty()
        && ents.instances.is_empty()
        && ents.edges.values().all(|e| e.faces.len() == 2)
}

/// Enclosed volume in cubic inches by the divergence theorem (the sum of the signed
/// volumes of the tetrahedra from the origin to each face triangle). A solid's faces
/// are oriented outward first, so reversed faces do not matter; for other geometry the
/// faces are taken as they point.
pub fn volume(ents: &Entities) -> f64 {
    if is_solid(ents) {
        let mut e = ents.clone();
        orient_outward(&mut e);
        signed_volume(&e)
    } else {
        signed_volume(ents)
    }
}

fn signed_volume(ents: &Entities) -> f64 {
    ents.faces
        .keys()
        .flat_map(|f| crate::tessellate::triangulate(ents, f))
        .map(|[a, b, c]| a.dot(b.cross(c)) / 6.0)
        .sum()
}

/// One operand: a solid's geometry (a definition's entities) placed into the output
/// space by `transform`.
#[derive(Debug, Clone, Copy)]
pub struct SolidInput<'a> {
    pub ents: &'a Entities,
    /// Definition space to output space.
    pub transform: DMat4,
    /// The instance's material, painted on faces without one of their own.
    pub material: Option<MaterialId>,
}

impl<'a> SolidInput<'a> {
    pub fn new(ents: &'a Entities, transform: DMat4) -> Self {
        Self {
            ents,
            transform,
            material: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolidError {
    /// The input at this index (argument order; position in the slice for
    /// [`outer_shell`]) is not a solid.
    NotSolid(usize),
    /// The result came out not solid (tolerance trouble, or solids touching only along
    /// an edge or at a vertex, which gives a non-manifold result).
    Failed,
}

impl std::fmt::Display for SolidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolidError::NotSolid(i) => write!(f, "input {i} is not a solid"),
            SolidError::Failed => write!(f, "the result is not a solid"),
        }
    }
}

impl std::error::Error for SolidError {}

/// The three pieces of [`split`].
#[derive(Debug, Clone, Default)]
pub struct Split {
    /// `first - second`.
    pub first_only: Entities,
    /// `first ∩ second`.
    pub both: Entities,
    /// `second - first`.
    pub second_only: Entities,
}

/// SketchUp's Union: everything inside either solid.
pub fn union(first: SolidInput, second: SolidInput) -> Result<Entities, SolidError> {
    let cut = Cut::new(first, second)?;
    finish(&cut.union())
}

/// SketchUp's Intersect: what is inside both solids.
pub fn intersect(first: SolidInput, second: SolidInput) -> Result<Entities, SolidError> {
    let cut = Cut::new(first, second)?;
    finish(&cut.intersection())
}

/// SketchUp's Subtract: `second - first`; the app deletes `first` (unverified which
/// operand SketchUp keeps). An empty result (`second` inside `first`) is `Ok` with no
/// faces.
pub fn subtract(first: SolidInput, second: SolidInput) -> Result<Entities, SolidError> {
    let cut = Cut::new(first, second)?;
    finish(&Cut::difference(&cut.b, &cut.a))
}

/// SketchUp's Trim: `second - first` like [`subtract`], but the app keeps `first`
/// unchanged (unverified). Returns the trimmed `second`.
pub fn trim(first: SolidInput, second: SolidInput) -> Result<Entities, SolidError> {
    subtract(first, second)
}

/// SketchUp's Split: `first - second`, `first ∩ second` and `second - first`. Pieces
/// that come out empty have no faces.
pub fn split(first: SolidInput, second: SolidInput) -> Result<Split, SolidError> {
    let cut = Cut::new(first, second)?;
    Ok(Split {
        first_only: finish(&Cut::difference(&cut.a, &cut.b))?,
        both: finish(&cut.intersection())?,
        second_only: finish(&Cut::difference(&cut.b, &cut.a))?,
    })
}

/// SketchUp's Outer Shell: the union of all `solids`, without shells enclosed by other
/// shells (voids).
pub fn outer_shell(solids: &[SolidInput]) -> Result<Entities, SolidError> {
    let Some((first, rest)) = solids.split_first() else {
        return Ok(Entities::new());
    };
    let mut acc = prepare(*first, 0)?;
    for (i, s) in rest.iter().enumerate() {
        let current = SolidInput::new(&acc, DMat4::IDENTITY);
        let cut = Cut::from_prepared(prepare(current, 0)?, prepare(*s, i + 1)?);
        acc = finish(&cut.union())?;
    }
    drop_enclosed_shells(&mut acc);
    if !acc.faces.is_empty() && !is_solid(&acc) {
        return Err(SolidError::Failed);
    }
    Ok(acc)
}

/// Where a piece of one solid's surface lies relative to the other solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Outside,
    Inside,
    /// On the other solid's surface, facing the same way.
    Same,
    /// On the other solid's surface, facing the opposite way.
    Opposite,
}

/// A face of a split solid, as points.
#[derive(Debug, Clone)]
struct Piece {
    /// Outer loop first, then holes.
    loops: Vec<Vec<DVec3>>,
    normal: DVec3,
    attrs: FaceAttrs,
    class: Class,
}

/// Both solids split against each other, every piece classified.
struct Cut {
    a: Vec<Piece>,
    b: Vec<Piece>,
}

impl Cut {
    fn new(first: SolidInput, second: SolidInput) -> Result<Self, SolidError> {
        Ok(Self::from_prepared(prepare(first, 0)?, prepare(second, 1)?))
    }

    fn from_prepared(ea: Entities, eb: Entities) -> Self {
        let (ca, cb) = (Classifier::new(&ea), Classifier::new(&eb));
        let cuts = snapped_cuts(&ea, &eb);
        let mut wa = ea.clone();
        let mut wb = eb.clone();
        for &(p, q) in &cuts {
            insert_line(&mut wa, p, q, false);
            insert_line(&mut wb, p, q, false);
        }
        Self {
            a: classify(&wa, &eb, &cb),
            b: classify(&wb, &ea, &ca),
        }
    }

    fn union(&self) -> Vec<(&Piece, bool)> {
        let a = keep(&self.a, &[Class::Outside, Class::Same], false);
        a.chain(keep(&self.b, &[Class::Outside], false)).collect()
    }

    fn intersection(&self) -> Vec<(&Piece, bool)> {
        let a = keep(&self.a, &[Class::Inside, Class::Same], false);
        a.chain(keep(&self.b, &[Class::Inside], false)).collect()
    }

    /// `x - y`: `x` outside `y`, plus `y` inside `x` reversed.
    fn difference<'p>(x: &'p [Piece], y: &'p [Piece]) -> Vec<(&'p Piece, bool)> {
        let kept = keep(x, &[Class::Outside, Class::Opposite], false);
        kept.chain(keep(y, &[Class::Inside], true)).collect()
    }
}

fn keep<'p>(
    pieces: &'p [Piece],
    classes: &'p [Class],
    flip: bool,
) -> impl Iterator<Item = (&'p Piece, bool)> + 'p {
    pieces
        .iter()
        .filter(move |p| classes.contains(&p.class))
        .map(move |p| (p, flip))
}

/// The input's faces in output space, painted with the instance material and oriented
/// outward.
fn prepare(input: SolidInput, which: usize) -> Result<Entities, SolidError> {
    if !is_solid(input.ents) {
        return Err(SolidError::NotSolid(which));
    }
    let mut e = Entities::new();
    let faces: Vec<FaceId> = input.ents.faces.keys().collect();
    crate::group::copy_raw(&mut e, input.ents, &[], &faces, input.transform);
    if let Some(m) = input.material {
        for f in e.faces.values_mut() {
            f.attrs.material.get_or_insert(m);
            f.attrs.back_material.get_or_insert(m);
        }
    }
    e.ensure_index();
    orient_outward(&mut e);
    Ok(e)
}

/// Pseudo-random ray directions, none along an axis or diagonal.
const DIRS: [DVec3; 7] = [
    DVec3::new(0.314_159, 0.726_201, 0.611_347),
    DVec3::new(-0.642_713, 0.215_381, 0.735_291),
    DVec3::new(0.811_723, -0.456_721, 0.364_219),
    DVec3::new(-0.225_133, -0.873_422, 0.431_917),
    DVec3::new(0.153_241, 0.361_747, -0.919_517),
    DVec3::new(-0.517_381, -0.583_137, -0.625_731),
    DVec3::new(0.913_377, 0.274_161, -0.300_517),
];

/// Ray parity tests against a set of faces.
struct Classifier {
    bvh: Bvh,
    owner: Vec<FaceId>,
}

enum RayHit {
    Miss,
    Cross,
    /// Within rounding of a triangle edge, or starting on the triangle.
    Unsure,
}

impl Classifier {
    fn new(e: &Entities) -> Self {
        let faces: Vec<FaceId> = e.faces.keys().collect();
        Self::of_faces(e, &faces)
    }

    fn of_faces(e: &Entities, faces: &[FaceId]) -> Self {
        let mut tris = Vec::new();
        let mut owner = Vec::new();
        for &f in faces {
            for t in crate::tessellate::triangulate(e, f) {
                tris.push(t);
                owner.push(f);
            }
        }
        Self {
            bvh: Bvh::build(tris),
            owner,
        }
    }

    /// Number of triangles the ray crosses (skipping `skip`'s), or `None` when a hit is
    /// unsure.
    fn crossings(&self, p: DVec3, dir: DVec3, skip: Option<FaceId>) -> Option<usize> {
        let mut n = 0;
        let mut unsure = false;
        self.bvh.ray_candidates(p, dir, |i| {
            if unsure || skip == Some(self.owner[i]) {
                return;
            }
            match ray_hit(p, dir, self.bvh.triangle(i)) {
                RayHit::Miss => {}
                RayHit::Cross => n += 1,
                RayHit::Unsure => unsure = true,
            }
        });
        (!unsure).then_some(n)
    }

    /// Whether `p` (off the surface) is inside.
    fn inside(&self, p: DVec3) -> bool {
        let mut votes = 0i32;
        for d in DIRS {
            if let Some(n) = self.crossings(p, d.normalize(), None) {
                return n % 2 == 1;
            }
        }
        // Every direction grazed an edge: count anyway and take the majority.
        for d in DIRS.map(DVec3::normalize) {
            let mut n = 0;
            self.bvh.ray_candidates(p, d, |i| {
                if !matches!(ray_hit(p, d, self.bvh.triangle(i)), RayHit::Miss) {
                    n += 1;
                }
            });
            votes += if n % 2 == 1 { 1 } else { -1 };
        }
        votes > 0
    }

    /// Whether face `f` (whose sample is `p`) faces outward: a ray leaving the front
    /// crosses the rest of the surface an even number of times.
    fn faces_out(&self, p: DVec3, normal: DVec3, f: FaceId) -> bool {
        for d in DIRS {
            let d = d.normalize();
            let d = if d.dot(normal) < 0.0 { -d } else { d };
            if d.dot(normal) < 0.2 {
                continue;
            }
            if let Some(n) = self.crossings(p, d, Some(f)) {
                return n % 2 == 0;
            }
        }
        // Rays leaving close to the face or all unsure: go straight out.
        self.crossings(p, normal, Some(f))
            .is_none_or(|n| n % 2 == 0)
    }
}

/// Möller-Trumbore with an "unsure" band around the triangle's edges.
fn ray_hit(o: DVec3, d: DVec3, [a, b, c]: [DVec3; 3]) -> RayHit {
    const BARY: f64 = 1e-9;
    const T_EPS: f64 = 1e-9;
    let e1 = b - a;
    let e2 = c - a;
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    if det.abs() <= 1e-12 * e1.length() * e2.length() {
        // Parallel: crossing the triangle's plane edge-on is caught by its neighbors.
        return RayHit::Miss;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(pv) * inv;
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    let w = 1.0 - u - v;
    let t = e2.dot(q) * inv;
    if t < -T_EPS || u < -BARY || v < -BARY || w < -BARY {
        return RayHit::Miss;
    }
    if t <= T_EPS || u <= BARY || v <= BARY || w <= BARY {
        return RayHit::Unsure;
    }
    RayHit::Cross
}

/// Faces grouped into connected shells (through shared edges), with each shell made
/// consistently oriented (a shared edge runs opposite ways in its two faces).
fn consistent_shells(e: &mut Entities) -> Vec<Vec<FaceId>> {
    let mut seen: HashSet<FaceId> = HashSet::new();
    let mut shells = Vec::new();
    let keys: Vec<FaceId> = e.faces.keys().collect();
    for start in keys {
        if !seen.insert(start) {
            continue;
        }
        let mut shell = vec![start];
        let mut queue = VecDeque::from([start]);
        while let Some(f) = queue.pop_front() {
            let coedges: Vec<CoEdge> = e.faces[f]
                .loops()
                .flat_map(|l| l.coedges.iter().copied())
                .collect();
            for c in coedges {
                for g in e.edges[c.edge].faces.clone() {
                    if g == f || !seen.insert(g) {
                        continue;
                    }
                    let theirs = e.faces[g]
                        .loops()
                        .flat_map(|l| &l.coedges)
                        .find(|x| x.edge == c.edge)
                        .copied();
                    if theirs.is_some_and(|t| t.reversed == c.reversed) {
                        flip_face(e, g);
                    }
                    shell.push(g);
                    queue.push_back(g);
                }
            }
        }
        shells.push(shell);
    }
    shells
}

/// Orients every shell so its faces point out of the solid (inner shells of a hollow
/// solid point into the void).
fn orient_outward(e: &mut Entities) {
    let shells = consistent_shells(e);
    let cls = Classifier::new(e);
    for shell in shells {
        let f = shell[0];
        let p = face_sample(e, f);
        if !cls.faces_out(p, e.faces[f].normal, f) {
            for g in shell {
                flip_face(e, g);
            }
        }
    }
}

/// The segments along which both solids are split, computed from either side and with
/// their ends snapped once for both: to a vertex of either solid within TOL, else to an
/// earlier end within TOL. Splitting each solid with its own unsnapped segments lets
/// the two disagree by up to TOL where a cut passes near a vertex, leaving a face of one
/// uncut.
fn snapped_cuts(ea: &Entities, eb: &Entities) -> Vec<(DVec3, DVec3)> {
    let mut segs = cut_segments(ea, eb);
    segs.extend(cut_segments(eb, ea));
    let mut points = Entities::new();
    points.ensure_index();
    let mut snap = |p: DVec3| {
        let v = points
            .find_vertex(p)
            .unwrap_or_else(|| points.add_vertex(p));
        points.pos(v)
    };
    for v in ea.vertices.values().chain(eb.vertices.values()) {
        snap(v.pos);
    }
    let key = |p: DVec3| p.to_array().map(f64::to_bits);
    let mut seen: HashSet<([u64; 3], [u64; 3])> = HashSet::new();
    let mut out: Vec<(DVec3, DVec3)> = Vec::with_capacity(segs.len());
    for (p, q) in segs {
        let (p, q) = (snap(p), snap(q));
        let (kp, kq) = (key(p), key(q));
        if p.distance(q) > TOL && seen.insert((kp.min(kq), kp.max(kq))) {
            out.push((p, q));
        }
    }
    out
}

/// Segments along which faces of `ea` must be split for `eb`: where `eb`'s faces cross
/// them, and the parts of `eb`'s edges lying on them within TOL (edges of coplanar faces,
/// or of a corner grazing the face).
fn cut_segments(ea: &Entities, eb: &Entities) -> Vec<(DVec3, DVec3)> {
    let mut out = Vec::new();
    for f in ea.faces.keys() {
        let plane = ea.face_plane(f);
        let bounds = ea.face_bounds(f).expand(TOL);
        for g in eb.faces_in(&bounds) {
            if !eb.face_plane(g).coincides(&plane) {
                out.extend(face_face(ea, f, eb, g));
            }
        }
        for x in eb.edges_in(&bounds) {
            let edge = &eb.edges[x];
            edge_on_face(ea, f, eb.pos(edge.start), eb.pos(edge.end), &mut out);
        }
    }
    out
}

/// The parts of segment `p-q` lying on face `f` within TOL, as parts of `p-q`: all of it
/// when both ends are within TOL of the face's plane, or from the end that is to where
/// the segment crosses the plane (a corner grazing the face). Edges crossing the plane
/// with both ends off it are left to `face_face`.
fn edge_on_face(e: &Entities, f: FaceId, p: DVec3, q: DVec3, out: &mut Vec<(DVec3, DVec3)>) {
    let plane = e.face_plane(f);
    let (dp, dq) = (plane.distance(p), plane.distance(q));
    let (a, b) = match (dp.abs() <= TOL, dq.abs() <= TOL) {
        (true, true) => (p, q),
        (true, false) if dp * dq <= 0.0 => (p, p.lerp(q, dp / (dp - dq))),
        (false, true) if dp * dq <= 0.0 => (p.lerp(q, dp / (dp - dq)), q),
        _ => return,
    };
    let (pa, pb) = (
        a - plane.normal * plane.distance(a),
        b - plane.normal * plane.distance(b),
    );
    let len = pa.distance(pb);
    if len <= TOL {
        return;
    }
    let dir = (pb - pa) / len;
    for (s0, s1) in clip(e, f, pa, dir) {
        let (s0, s1) = (s0.max(0.0), s1.min(len));
        if s1 - s0 > TOL {
            out.push((a.lerp(b, s0 / len), a.lerp(b, s1 / len)));
        }
    }
}

/// Every face of `w` (a split solid) with its class relative to `other`.
fn classify(w: &Entities, other: &Entities, cls: &Classifier) -> Vec<Piece> {
    w.faces
        .iter()
        .map(|(f, face)| {
            let p = face_sample(w, f);
            let loops: Vec<Vec<DVec3>> = face.loops().map(|l| w.loop_points(l)).collect();
            // On the other solid: the whole piece within TOL of one of its faces (not
            // only coplanar faces: a sliver of a corner grazing a face is tilted).
            let on = other.faces_near(p, TOL).into_iter().find(|&g| {
                let plane = other.face_plane(g);
                loops.iter().flatten().all(|&x| plane.contains(x)) && other.face_contains(g, p)
            });
            let class = match on {
                Some(g) if other.faces[g].normal.dot(face.normal) > 0.0 => Class::Same,
                Some(_) => Class::Opposite,
                None if cls.inside(p) => Class::Inside,
                None => Class::Outside,
            };
            Piece {
                loops,
                normal: face.normal,
                attrs: face.attrs.clone(),
                class,
            }
        })
        .collect()
}

/// Welds the pieces into one `Entities`, merges coplanar neighbors and checks the
/// result is a solid (or empty).
fn finish(pieces: &[(&Piece, bool)]) -> Result<Entities, SolidError> {
    let mut out = assemble(pieces);
    merge_coplanar(&mut out);
    let vs: Vec<VertexId> = out.vertices.keys().collect();
    for v in vs {
        heal_vertex(&mut out, v);
        out.remove_vertex_if_unused(v);
    }
    split_near_crossings(&mut out);
    if out.faces.is_empty() {
        return Ok(Entities::new());
    }
    // Near-degenerate input (features of one solid within about TOL of the other's)
    // can weld vertices into a face that is no longer planar: report failure rather
    // than return invalid geometry.
    if !is_solid(&out) || out.faces.keys().any(|f| !face_is_sound(&out, f)) {
        return Err(SolidError::Failed);
    }
    Ok(out)
}

/// Builds faces from pieces (reversed where flagged), sharing vertices within TOL and
/// splitting loop edges at vertices of other pieces lying on them. Two pieces on the
/// same vertices facing opposite ways (a sliver thinner than TOL, from a solid grazing
/// the other within tolerance) cancel out.
fn assemble(pieces: &[(&Piece, bool)]) -> Entities {
    let mut out = Entities::new();
    out.ensure_index();
    let mut faces: Vec<(Vec<Vec<VertexId>>, DVec3, FaceAttrs)> = Vec::new();
    for (piece, flip) in pieces {
        let loops = piece
            .loops
            .iter()
            .map(|l| {
                let mut vs: Vec<VertexId> = l
                    .iter()
                    .map(|&p| out.find_vertex(p).unwrap_or_else(|| out.add_vertex(p)))
                    .collect();
                if *flip {
                    vs.reverse();
                }
                vs
            })
            .collect();
        let normal = if *flip { -piece.normal } else { piece.normal };
        faces.push((loops, normal, piece.attrs.clone()));
    }
    let mut faces: Vec<(Vec<Vec<VertexId>>, DVec3, FaceAttrs)> = faces
        .into_iter()
        .filter_map(|(loops, normal, attrs)| {
            let mut full: Vec<Vec<VertexId>> = Vec::new();
            for (i, vs) in loops.iter().enumerate() {
                let l = with_t_junctions(&out, vs);
                if l.len() >= 3 {
                    full.push(l);
                } else if i == 0 {
                    return None;
                }
            }
            Some((full, normal, attrs))
        })
        .collect();
    let mut by_key: HashMap<Vec<VertexId>, Vec<usize>> = HashMap::new();
    for (i, (loops, _, _)) in faces.iter().enumerate() {
        let mut key: Vec<VertexId> = loops.concat();
        key.sort();
        by_key.entry(key).or_default().push(i);
    }
    let mut cancelled = vec![false; faces.len()];
    for same in by_key.values() {
        for (k, &i) in same.iter().enumerate() {
            for &j in &same[k + 1..] {
                if !cancelled[i] && !cancelled[j] && faces[i].1.dot(faces[j].1) < 0.0 {
                    cancelled[i] = true;
                    cancelled[j] = true;
                }
            }
        }
    }
    let mut i = 0;
    faces.retain(|_| {
        i += 1;
        !cancelled[i - 1]
    });
    for (loops, normal, attrs) in faces {
        let mut built: Vec<Loop> = loops
            .iter()
            .map(|full| Loop {
                coedges: (0..full.len())
                    .map(|k| {
                        let (p, q) = (full[k], full[(k + 1) % full.len()]);
                        let e = out
                            .edge_between(p, q)
                            .unwrap_or_else(|| out.add_raw_edge(p, q, EdgeAttrs::default()));
                        CoEdge {
                            edge: e,
                            reversed: out.edges[e].start != p,
                        }
                    })
                    .collect(),
            })
            .collect();
        let outer = built.remove(0);
        out.add_raw_face(outer, built, normal, attrs);
    }
    out.vertices
        .keys()
        .collect::<Vec<_>>()
        .into_iter()
        .for_each(|v| out.remove_vertex_if_unused(v));
    out
}

/// A loop with the vertices lying on its edges inserted, and repeats removed.
fn with_t_junctions(out: &Entities, vs: &[VertexId]) -> Vec<VertexId> {
    let mut full: Vec<VertexId> = Vec::with_capacity(vs.len());
    for k in 0..vs.len() {
        let (a, b) = (vs[k], vs[(k + 1) % vs.len()]);
        full.push(a);
        if a == b {
            continue;
        }
        let (pa, pb) = (out.pos(a), out.pos(b));
        let mut mids: Vec<(f64, VertexId)> = out
            .vertices_near_segment(pa, pb, TOL)
            .into_iter()
            .filter(|&v| v != a && v != b)
            .filter_map(|v| {
                let (d, t) = point_segment(out.pos(v), pa, pb);
                (d <= TOL && t > 0.0 && t < 1.0).then_some((t, v))
            })
            .collect();
        mids.sort_by(|x, y| x.0.total_cmp(&y.0));
        full.extend(mids.into_iter().map(|(_, v)| v));
    }
    full.dedup();
    while full.len() > 1 && full.first() == full.last() {
        full.pop();
    }
    full
}

/// Splits edges that another edge or a vertex comes within TOL / 2 of (pieces passing
/// within tolerance of each other, as a near-tangent cut gives), so they share a vertex
/// as everywhere else in the kernel.
fn split_near_crossings(e: &mut Entities) {
    e.ensure_index();
    for _ in 0..e.edges.len() + 16 {
        let Some((edges, m)) = near_crossing(e) else {
            return;
        };
        let ends: Vec<VertexId> = edges
            .iter()
            .flat_map(|&x| [e.edges[x].start, e.edges[x].end])
            .collect();
        let w = ends
            .iter()
            .copied()
            .find(|&v| e.pos(v).distance(m) <= TOL)
            .or_else(|| e.find_vertex(m))
            .unwrap_or_else(|| e.add_vertex(m));
        for x in edges {
            if e.edges[x].start != w && e.edges[x].end != w {
                e.split_edge(x, w);
            }
        }
    }
}

/// Two edges passing within TOL / 2 without a shared vertex (or an edge and a vertex,
/// as one edge), and the point where they meet.
fn near_crossing(e: &Entities) -> Option<(Vec<EdgeId>, DVec3)> {
    for (a, edge) in &e.edges {
        let (p0, p1) = (e.pos(edge.start), e.pos(edge.end));
        for v in e.vertices_near_segment(p0, p1, TOL) {
            if v != edge.start && v != edge.end && point_segment(e.pos(v), p0, p1).0 <= TOL / 2.0 {
                return Some((vec![a], e.pos(v)));
            }
        }
        for b in e.edges_near_segment(p0, p1, TOL) {
            let other = &e.edges[b];
            if b <= a
                || [other.start, other.end]
                    .iter()
                    .any(|v| *v == edge.start || *v == edge.end)
            {
                continue;
            }
            let (q0, q1) = (e.pos(other.start), e.pos(other.end));
            if let Some((d, s, t)) = segment_segment(p0, p1, q0, q1)
                && d <= TOL / 2.0
            {
                let m = (p0.lerp(p1, s) + q0.lerp(q1, t)) / 2.0;
                return Some((vec![a, b], m));
            }
        }
    }
    None
}

fn mergeable(e: &Entities, f: FaceId, g: FaceId) -> bool {
    let (x, y) = (&e.faces[f], &e.faces[g]);
    f != g
        && x.normal.dot(y.normal) > 0.0
        && x.attrs == y.attrs
        && e.face_plane(f).coincides(&e.face_plane(g))
}

/// Replaces each group of edge-connected coplanar faces facing the same way with the
/// same attributes by the faces of their combined region.
fn merge_coplanar(e: &mut Entities) {
    let faces: Vec<FaceId> = e.faces.keys().collect();
    let index: HashMap<FaceId, usize> = faces.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn root(parent: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        parent[i] = r;
        r
    }
    for edge in e.edges.values() {
        if let [f, g] = edge.faces[..]
            && mergeable(e, f, g)
        {
            let (a, b) = (root(&mut parent, index[&f]), root(&mut parent, index[&g]));
            parent[a] = b;
        }
    }
    let mut groups: HashMap<usize, Vec<FaceId>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        groups.entry(root(&mut parent, i)).or_default().push(*f);
    }
    let mut groups: Vec<Vec<FaceId>> = groups.into_values().filter(|g| g.len() > 1).collect();
    groups.sort();
    for group in groups {
        let first = group[0];
        let plane = e.face_plane(first);
        let normal = e.faces[first].normal;
        let attrs = e.faces[first].attrs.clone();
        let regions: Vec<(Vec<DVec2>, Vec<Vec<DVec2>>)> =
            group.iter().map(|&f| e.face_polygons(f, &plane)).collect();
        let mut uses: HashMap<EdgeId, usize> = HashMap::new();
        for &f in &group {
            for edge in crate::transform::face_edges(e, f) {
                *uses.entry(edge).or_default() += 1;
            }
        }
        let mut boundary: Vec<EdgeId> = uses
            .iter()
            .filter(|(_, n)| **n == 1)
            .map(|(x, _)| *x)
            .collect();
        boundary.sort();
        for &f in &group {
            e.remove_face(f);
        }
        for (&edge, &n) in &uses {
            if n > 1 && e.edges[edge].faces.is_empty() {
                let (s, t) = (e.edges[edge].start, e.edges[edge].end);
                e.unlink_edge(edge);
                e.remove_vertex_if_unused(s);
                e.remove_vertex_if_unused(t);
            }
        }
        for cell in arrangement::cells(e, &plane, &boundary) {
            let q = plane.project(cell.sample);
            let inside = regions.iter().any(|(outer, holes)| {
                point_in_polygon(q, outer) && !holes.iter().any(|h| point_in_polygon(q, h))
            });
            if inside {
                face_from_cell(e, &plane, &cell, normal, attrs.clone());
            }
        }
    }
}

/// Removes shells lying inside another shell (Outer Shell's interior geometry).
fn drop_enclosed_shells(e: &mut Entities) {
    let shells = consistent_shells(e);
    if shells.len() < 2 {
        return;
    }
    let classifiers: Vec<Classifier> = shells.iter().map(|s| Classifier::of_faces(e, s)).collect();
    let enclosed: Vec<usize> = (0..shells.len())
        .filter(|&i| {
            let p = face_sample(e, shells[i][0]);
            (0..shells.len()).any(|j| j != i && classifiers[j].inside(p))
        })
        .collect();
    for i in enclosed {
        for &f in &shells[i] {
            let edges = crate::transform::face_edges(e, f);
            e.remove_face(f);
            for edge in edges {
                if e.edges.get(edge).is_some_and(|x| x.faces.is_empty()) {
                    let (s, t) = (e.edges[edge].start, e.edges[edge].end);
                    e.unlink_edge(edge);
                    e.remove_vertex_if_unused(s);
                    e.remove_vertex_if_unused(t);
                }
            }
        }
    }
}
