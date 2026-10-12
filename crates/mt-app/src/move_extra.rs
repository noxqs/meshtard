//! Move tool extras (FEATURES.md 57): auto-fold, live stretching of connected
//! geometry, and reshaping an arc by its midpoint or an end.
//!
//! - Auto-fold: moving faces in a direction that would bend a neighboring face out of
//!   its plane is held to the directions that keep every neighbor planar; Alt toggles
//!   auto-fold, which allows any direction and folds the bent faces (the kernel's
//!   `transform_vertices` folds them). SketchUp's help lists Alt as "toggle
//!   Auto-fold"; that the restriction applies only when faces are moved (edges and
//!   vertices fold freely) is from memory, unverified.
//! - Live stretching: while Move, Rotate or Scale runs, the faces and edges connected
//!   to the moved vertices are drawn stretched ([`Stretch`]), as the click will leave
//!   them (faces that will fold are drawn unfolded).
//! - Arcs [doc ARC]: Move on an arc's midpoint drags its bulge (the ends stay); on an
//!   arc's end it moves that end, the other end and the arc's tangent there staying
//!   (which end and tangent stay is unverified), "adjusting the arc's length and
//!   radius".

use glam::{DMat4, DVec3};
use mt_kernel::{Arc, CurveId, Entities, FaceId, TOL, VertexId};

use crate::document::Owner;

// ------------------------------------------------------------------ Auto-fold

/// The directions a move may take without folding faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Freedom {
    Free,
    /// In the plane with this unit normal.
    Plane(DVec3),
    /// Along this unit direction.
    Line(DVec3),
    /// Not at all.
    Fixed,
}

impl Freedom {
    /// `v` held to the allowed directions.
    pub fn project(self, v: DVec3) -> DVec3 {
        match self {
            Self::Free => v,
            Self::Plane(n) => v - n * v.dot(n),
            Self::Line(d) => d * v.dot(d),
            Self::Fixed => DVec3::ZERO,
        }
    }

    fn and_plane(self, n: DVec3) -> Self {
        let parallel = |a: DVec3, b: DVec3| a.cross(b).length() < 1e-9;
        match self {
            Self::Free => Self::Plane(n),
            Self::Plane(m) if parallel(m, n) => self,
            Self::Plane(m) => Self::Line(m.cross(n).normalize()),
            Self::Line(d) if d.dot(n).abs() < 1e-9 => self,
            Self::Line(_) | Self::Fixed => Self::Fixed,
        }
    }
}

/// Whether the points lie on one line (one point or none counts).
fn collinear(pts: &[DVec3]) -> Option<Option<DVec3>> {
    let Some(&a) = pts.first() else {
        return Some(None);
    };
    let far = pts
        .iter()
        .copied()
        .max_by(|p, q| p.distance(a).total_cmp(&q.distance(a)))?;
    if far.distance(a) <= TOL {
        return Some(None);
    }
    let d = (far - a).normalize();
    pts.iter()
        .all(|p| (*p - a - d * (*p - a).dot(d)).length() <= TOL)
        .then_some(Some(d))
}

/// What a translation of the targets may do without auto-fold: each face that has
/// some of its vertices moved and some not must stay planar. A face with fixed
/// vertices spanning its plane (or moved ones spanning it) holds the move to that
/// plane; a face made of two parallel lines (or a line and a point) stays planar
/// whatever the move. Only moves of faces are held (see the module notes).
pub fn fold_freedom(ents: &Entities, targets: &[Owner]) -> Freedom {
    let (mut edges, mut faces) = (Vec::new(), Vec::new());
    for t in targets {
        match *t {
            Owner::Edge(e) => edges.push(e),
            Owner::Face(f) => faces.push(f),
            _ => {}
        }
    }
    if faces.is_empty() {
        return Freedom::Free;
    }
    let moved: std::collections::HashSet<VertexId> = mt_kernel::vertices_of(ents, &edges, &faces)
        .into_iter()
        .collect();
    let mut around: Vec<FaceId> = moved
        .iter()
        .filter_map(|v| ents.vertices.get(*v))
        .flat_map(|v| v.edges.iter())
        .filter_map(|e| ents.edges.get(*e))
        .flat_map(|e| e.faces.iter().copied())
        .collect();
    around.sort();
    around.dedup();
    let mut out = Freedom::Free;
    for f in around {
        let face = &ents.faces[f];
        let (mut fixed, mut moving) = (Vec::new(), Vec::new());
        for l in face.loops() {
            for v in ents.loop_vertices(l) {
                if moved.contains(&v) {
                    moving.push(ents.pos(v));
                } else {
                    fixed.push(ents.pos(v));
                }
            }
        }
        if fixed.is_empty() || moving.is_empty() {
            continue;
        }
        let free = match (collinear(&fixed), collinear(&moving)) {
            (Some(None), Some(_)) | (Some(_), Some(None)) => true,
            (Some(Some(a)), Some(Some(b))) => a.cross(b).length() < 1e-9,
            _ => false,
        };
        if !free {
            out = out.and_plane(face.normal.normalize_or(DVec3::Z));
        }
    }
    out
}

// ------------------------------------------------------------------ Live stretching

/// The faces and edges stretched by a running move: for each, its moved vertices as
/// (world position now, world position after). The moved targets themselves are drawn
/// with the whole transform and are not listed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stretch {
    pub owners: mt_kernel::spatial::FxMap<Owner, Vec<(DVec3, DVec3)>>,
}

/// See [`Stretch`]. `xf` is in context space, `ctx_xf` the context's transform.
pub fn stretch(ents: &Entities, targets: &[Owner], xf: DMat4, ctx_xf: DMat4) -> Stretch {
    let (mut edges, mut faces) = (Vec::new(), Vec::new());
    for t in targets {
        match *t {
            Owner::Edge(e) => edges.push(e),
            Owner::Face(f) => faces.push(f),
            _ => {}
        }
    }
    let moved: mt_kernel::spatial::FxMap<VertexId, (DVec3, DVec3)> =
        mt_kernel::vertices_of(ents, &edges, &faces)
            .into_iter()
            .map(|v| {
                let p = ents.pos(v);
                (
                    v,
                    (
                        ctx_xf.transform_point3(p),
                        ctx_xf.transform_point3(xf.transform_point3(p)),
                    ),
                )
            })
            .collect();
    let mut out = Stretch::default();
    let targets: mt_kernel::spatial::FxSet<Owner> = targets.iter().copied().collect();
    let is_target = |o: &Owner| targets.contains(o);
    for &v in moved.keys() {
        for &e in &ents.vertices[v].edges {
            let edge = &ents.edges[e];
            let o = Owner::Edge(e);
            if !is_target(&o) && !out.owners.contains_key(&o) {
                let pairs = [edge.start, edge.end]
                    .iter()
                    .filter_map(|w| moved.get(w).copied())
                    .collect();
                out.owners.insert(o, pairs);
            }
            for &f in &edge.faces {
                let o = Owner::Face(f);
                if is_target(&o) || out.owners.contains_key(&o) {
                    continue;
                }
                let pairs = ents.faces[f]
                    .loops()
                    .flat_map(|l| ents.loop_vertices(l))
                    .filter_map(|w| moved.get(&w).copied())
                    .collect();
                out.owners.insert(o, pairs);
            }
        }
    }
    out
}

impl Stretch {
    /// Where the drawn point `p` (world) of `owner` goes; `None` when `owner` is not
    /// stretched.
    pub fn map(&self, owner: &Owner, p: [f32; 3]) -> Option<[f32; 3]> {
        let pairs = self.owners.get(owner)?;
        let q = glam::Vec3::from(p).as_dvec3();
        // Drawn positions are single precision.
        let eps = 1e-4 * (1.0 + q.abs().max_element());
        Some(
            pairs
                .iter()
                .find(|(from, _)| from.distance(q) <= eps)
                .map_or(p, |(_, to)| to.as_vec3().into()),
        )
    }

    pub fn contains(&self, owner: &Owner) -> bool {
        self.owners.contains_key(owner)
    }

    /// A drawn triangle of a stretched face, moved, with its flat normal (the
    /// face's smoothing is not kept while it stretches).
    pub fn triangle(
        &self,
        owner: &Owner,
        tri: [mt_render::FaceVertex; 3],
    ) -> [mt_render::FaceVertex; 3] {
        let mut tri = tri.map(|mut v| {
            v.position = self.map(owner, v.position).unwrap_or(v.position);
            v
        });
        let p = tri.map(|v| glam::Vec3::from(v.position));
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if n != glam::Vec3::ZERO {
            for v in &mut tri {
                // Keep the side the face showed.
                let old = glam::Vec3::from(v.normal);
                v.normal = if old.dot(n) < 0.0 { -n } else { n }.into();
            }
        }
        tri
    }
}

/// The stretch of the app's live operation (`(targets, transform, copy)` in context
/// space): empty for a copy or nothing running.
pub fn live_stretch(
    doc: Option<&crate::document::Document>,
    live: &Option<(Vec<Owner>, DMat4, bool)>,
    ctx_xf: DMat4,
) -> Stretch {
    match (doc, live) {
        (Some(doc), Some((targets, xf, false))) => stretch(doc.active(), targets, *xf, ctx_xf),
        _ => Stretch::default(),
    }
}

// ------------------------------------------------------------------ Arcs

/// Which part of an arc the Move tool holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcGrip {
    Mid,
    Start,
    End,
}

/// The arc point at the middle of its sweep.
pub fn arc_midpoint(arc: &Arc) -> DVec3 {
    let t = (arc.start_angle + arc.end_angle) / 2.0;
    arc.center + arc.radius * (t.cos() * arc.x_axis + t.sin() * arc.y_axis())
}

/// The direction the arc leaves its start in (toward its end).
pub(crate) fn start_tangent(arc: &Arc) -> DVec3 {
    let t = arc.start_angle;
    let d = -t.sin() * arc.x_axis + t.cos() * arc.y_axis();
    if arc.end_angle >= arc.start_angle {
        d
    } else {
        -d
    }
}

/// The direction the arc leaves its end in (back toward its start).
pub(crate) fn end_tangent(arc: &Arc) -> DVec3 {
    let t = arc.end_angle;
    let d = -t.sin() * arc.x_axis + t.cos() * arc.y_axis();
    if arc.end_angle >= arc.start_angle {
        -d
    } else {
        d
    }
}

/// The arc curve and grip under a Move base point: the midpoint of an arc ("Midpoint
/// of Arc" inference), or an end of the arc whose edge is `picked`. Circles have no
/// grips.
pub fn arc_grip_at(
    ents: &Entities,
    point: DVec3,
    kind: mt_infer::Kind,
    picked: Option<Owner>,
) -> Option<(CurveId, ArcGrip)> {
    let usable = |c: CurveId| {
        ents.curves
            .get(c)
            .and_then(|x| x.arc.filter(|a| !a.is_circle() && !x.polygon))
    };
    match kind {
        mt_infer::Kind::ArcMidpoint => ents.curves.keys().find_map(|c| {
            let a = usable(c)?;
            (arc_midpoint(&a).distance(point) <= TOL).then_some((c, ArcGrip::Mid))
        }),
        mt_infer::Kind::Endpoint => {
            let Some(Owner::Edge(e)) = picked else {
                return None;
            };
            let c = ents.edges.get(e)?.attrs.curve?;
            let a = usable(c)?;
            let pts = a.points();
            if pts.first()?.distance(point) <= TOL {
                Some((c, ArcGrip::Start))
            } else if pts.last()?.distance(point) <= TOL {
                Some((c, ArcGrip::End))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// `arc` with its `grip` dragged to `p` (projected into the arc's plane).
pub fn reshaped(arc: &Arc, grip: ArcGrip, p: DVec3) -> Option<Arc> {
    let pts = arc.points();
    let (a, b) = (*pts.first()?, *pts.last()?);
    let n = arc.normal.normalize_or_zero();
    let p = p - n * (p - arc.center).dot(n);
    let (from, to, tangent) = match grip {
        ArcGrip::Mid => {
            let dir = (b - a).normalize_or_zero();
            let bulge = (p - a).dot(n.cross(dir));
            return crate::tools::arc_from_bulge(a, b, bulge, n, arc.segments);
        }
        ArcGrip::End => (a, p, start_tangent(arc)),
        ArcGrip::Start => (b, p, end_tangent(arc)),
    };
    let bulge = mt_infer::hints::tangent_bulge(from, to, n, tangent)?;
    crate::tools::arc_from_bulge(from, to, bulge, n, arc.segments)
}

/// Moves the arc's `grip` to `p`: an end vertex moves first (edges meeting it
/// stretch), then the curve is redrawn. Returns whether the model changed.
pub fn reshape_arc(ents: &mut Entities, c: CurveId, grip: ArcGrip, p: DVec3) -> bool {
    let Some(arc) = ents.curves.get(c).and_then(|x| x.arc) else {
        return false;
    };
    let Some(new) = reshaped(&arc, grip, p) else {
        return false;
    };
    let pts = arc.points();
    let end = match grip {
        ArcGrip::Mid => None,
        ArcGrip::Start => pts.first().copied(),
        ArcGrip::End => pts.last().copied(),
    };
    let target = match grip {
        ArcGrip::Mid => None,
        _ => new.points().last().copied(),
    };
    if let (Some(from), Some(to)) = (end, target)
        && let Some(v) = ents.find_vertex(from)
    {
        // Only the edges that are not the arc's stretch; the arc is redrawn next.
        let others = ents.vertices[v]
            .edges
            .iter()
            .any(|e| ents.edges[*e].attrs.curve != Some(c));
        if others {
            mt_kernel::transform_vertices(ents, &[v], DMat4::from_translation(to - from));
        }
    }
    // The end move may have redrawn the curve under a new id; find it by an edge.
    let c = if ents.curves.contains_key(c) && !ents.curve_edges(c).is_empty() {
        c
    } else {
        return false;
    };
    mt_kernel::entity_edit::replace_arc(ents, c, new).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    /// A 3 x 3 grid of unit 10 squares on the ground; returns the middle face.
    fn grid() -> (Entities, FaceId) {
        let mut e = Entities::new();
        for i in 0..4 {
            let t = f64::from(i) * 10.0;
            mt_kernel::add_line(&mut e, p(0.0, t, 0.0), p(30.0, t, 0.0));
            mt_kernel::add_line(&mut e, p(t, 0.0, 0.0), p(t, 30.0, 0.0));
        }
        let mid = e
            .faces
            .keys()
            .find(|&f| {
                let pts = e.loop_points(&e.faces[f].outer);
                pts.iter()
                    .all(|q| (10.0..=20.0).contains(&q.x) && (10.0..=20.0).contains(&q.y))
            })
            .unwrap();
        (e, mid)
    }

    fn all_planar(e: &Entities) -> bool {
        e.faces.keys().all(|f| {
            let plane = e.face_plane(f);
            e.faces[f]
                .loops()
                .flat_map(|l| e.loop_points(l))
                .all(|q| plane.distance(q).abs() <= TOL)
        })
    }

    #[test]
    fn moving_a_grid_cell_up_needs_auto_fold() {
        let (mut e, mid) = grid();
        assert_eq!(e.faces.len(), 9);
        let targets = [Owner::Face(mid)];
        // Corner neighbors share one vertex: only level moves keep them planar.
        let free = fold_freedom(&e, &targets);
        assert!(matches!(free, Freedom::Plane(n) if n.z.abs() > 0.99));
        assert_eq!(free.project(p(1.0, 2.0, 5.0)), p(1.0, 2.0, 0.0));
        // Edges alone fold freely.
        let any_edge = e.edges.keys().next().unwrap();
        assert_eq!(fold_freedom(&e, &[Owner::Edge(any_edge)]), Freedom::Free);
        // With auto-fold the cell goes up and every face stays planar (folded).
        crate::modify::apply(
            &mut e,
            &targets,
            DMat4::from_translation(p(0.0, 0.0, 5.0)),
            false,
        );
        assert!(e.faces.len() > 9, "neighbors folded");
        assert!(all_planar(&e));
        assert!(e.vertices.values().filter(|v| v.pos.z > 4.99).count() == 4);
    }

    #[test]
    fn freedom_intersects_planes() {
        let f = Freedom::Free.and_plane(DVec3::Z).and_plane(DVec3::X);
        assert!(matches!(f, Freedom::Line(d) if d.y.abs() > 0.99));
        assert_eq!(f.and_plane(DVec3::Y), Freedom::Fixed);
        assert_eq!(f.and_plane(DVec3::Z), f);
    }

    #[test]
    fn live_stretch_matches_the_committed_move() {
        let mut e = Entities::new();
        let sq = [
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 10.0, 0.0),
            p(0.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            mt_kernel::add_line(&mut e, sq[i], sq[(i + 1) % 4]);
        }
        let base = e.faces.keys().next().unwrap();
        mt_kernel::push_pull(&mut e, base, -8.0, false);
        let top = e
            .faces
            .keys()
            .find(|&f| e.faces[f].normal.z > 0.99)
            .unwrap();
        let targets = [Owner::Face(top)];
        let xf = DMat4::from_translation(p(3.0, 1.0, 4.0));
        let ctx = DMat4::from_translation(p(100.0, 0.0, 0.0));
        let live = stretch(&e, &targets, xf, ctx);
        // The four sides and the top's four edges and four risers stretch.
        assert_eq!(
            live.owners
                .keys()
                .filter(|o| matches!(o, Owner::Face(_)))
                .count(),
            4
        );
        assert_eq!(
            live.owners
                .keys()
                .filter(|o| matches!(o, Owner::Edge(_)))
                .count(),
            8
        );
        let before = e.clone();
        crate::modify::apply(&mut e, &targets, xf, false);
        for o in live.owners.keys() {
            let (old, new): (Vec<DVec3>, Vec<DVec3>) = match *o {
                Owner::Face(f) => (
                    before.loop_points(&before.faces[f].outer),
                    e.loop_points(&e.faces[f].outer),
                ),
                Owner::Edge(x) => {
                    let (a, b) = (&before.edges[x], &e.edges[x]);
                    (
                        vec![before.pos(a.start), before.pos(a.end)],
                        vec![e.pos(b.start), e.pos(b.end)],
                    )
                }
                _ => continue,
            };
            for (q, r) in old.iter().zip(&new) {
                let drawn = ctx.transform_point3(*q).as_vec3().into();
                let shown = glam::Vec3::from(live.map(o, drawn).unwrap()).as_dvec3();
                assert!(shown.distance(ctx.transform_point3(*r)) < 1e-3, "{o:?}");
            }
        }
        assert!(live.map(&Owner::Face(top), [0.0; 3]).is_none());
    }

    fn quarter(segments: usize) -> Arc {
        Arc {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            x_axis: DVec3::X,
            radius: 10.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            segments,
        }
    }

    #[test]
    fn arc_midpoint_drag_changes_the_radius() {
        let mut e = Entities::new();
        let arc = quarter(6);
        mt_kernel::add_curve(&mut e, &arc.points(), Some(arc));
        let line_end = p(10.0, -20.0, 0.0);
        mt_kernel::add_line(&mut e, p(10.0, 0.0, 0.0), line_end);
        let c = e.curves.keys().next().unwrap();
        let mid = arc_midpoint(&arc);
        assert_eq!(
            arc_grip_at(&e, mid, mt_infer::Kind::ArcMidpoint, None),
            Some((c, ArcGrip::Mid))
        );
        // Flatter: the midpoint dragged toward the chord.
        let chord_mid = (p(10.0, 0.0, 0.0) + p(0.0, 10.0, 0.0)) / 2.0;
        assert!(reshape_arc(
            &mut e,
            c,
            ArcGrip::Mid,
            (mid + chord_mid) / 2.0
        ));
        let new = e.curves.values().find_map(|x| x.arc).unwrap();
        assert!(new.radius > 10.5);
        let pts = new.points();
        assert!(pts[0].distance(p(10.0, 0.0, 0.0)) < 1e-6);
        assert!(pts.last().unwrap().distance(p(0.0, 10.0, 0.0)) < 1e-6);
        assert_eq!(new.segments, 6);
    }

    #[test]
    fn arc_end_drag_keeps_the_other_end_and_its_tangent() {
        let mut e = Entities::new();
        let arc = quarter(6);
        mt_kernel::add_curve(&mut e, &arc.points(), Some(arc));
        // A line meets the arc's end and stretches with it.
        mt_kernel::add_line(&mut e, p(0.0, 10.0, 0.0), p(-20.0, 10.0, 0.0));
        let c = e.curves.keys().next().unwrap();
        let arc_edge = e.curve_edges(c)[0];
        let (c2, grip) = arc_grip_at(
            &e,
            p(0.0, 10.0, 0.0),
            mt_infer::Kind::Endpoint,
            Some(Owner::Edge(arc_edge)),
        )
        .or_else(|| {
            let last = *e.curve_edges(c).last().unwrap();
            arc_grip_at(
                &e,
                p(0.0, 10.0, 0.0),
                mt_infer::Kind::Endpoint,
                Some(Owner::Edge(last)),
            )
        })
        .unwrap();
        assert_eq!((c2, grip), (c, ArcGrip::End));
        let to = p(-5.0, 14.0, 0.0);
        assert!(reshape_arc(&mut e, c, ArcGrip::End, to));
        let new = e.curves.values().find_map(|x| x.arc).unwrap();
        let pts = new.points();
        assert!(pts[0].distance(p(10.0, 0.0, 0.0)) < 1e-6);
        assert!(pts.last().unwrap().distance(to) < 1e-6);
        // Still leaves (10, 0) straight up, as before.
        assert!(start_tangent(&new).distance(DVec3::Y) < 1e-6);
        assert!(new.radius > 10.0);
        // The line now starts at the moved end.
        assert!(e.find_vertex(to).is_some());
        assert!(e.find_vertex(p(-20.0, 10.0, 0.0)).is_some());
        assert!(e.find_vertex(p(0.0, 10.0, 0.0)).is_none());
    }

    fn at(point: DVec3, kind: mt_infer::Kind, segment: Option<[DVec3; 2]>) -> mt_infer::Inference {
        mt_infer::Inference {
            point,
            kind,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment,
            container: None,
        }
    }

    #[test]
    fn alt_locks_the_two_point_arc_tangent() {
        use crate::tools::{DrawHover, DrawTool, TwoPointArc};
        let mut e = Entities::new();
        mt_kernel::add_line(&mut e, p(-10.0, 0.0, 0.0), DVec3::ZERO);
        let mut t = TwoPointArc::default();
        let hover = |t: &mut TwoPointArc, alt: bool| {
            t.hover(&DrawHover {
                instance_name: None,
                units: mt_infer::units::LengthUnit::Inch,
                alt,
            })
        };
        let start = at(
            DVec3::ZERO,
            mt_infer::Kind::Endpoint,
            Some([p(-10.0, 0.0, 0.0), DVec3::ZERO]),
        );
        assert!(!t.click(&mut e, &start));
        hover(&mut t, true);
        hover(&mut t, false);
        assert!(t.status().contains("Tangent locked"));
        // The end click finishes a tangent arc.
        assert!(t.click(&mut e, &at(p(10.0, 10.0, 0.0), mt_infer::Kind::Free, None)));
        let arc = e.curves.values().find_map(|c| c.arc).unwrap();
        assert!(start_tangent(&arc).distance(DVec3::X) < 1e-9);
        assert!((arc.radius - 10.0).abs() < 1e-9);
    }

    #[test]
    fn clicking_outside_a_rounded_corner_unrounds_it() {
        let mut e = Entities::new();
        let arc = Arc {
            center: p(14.0, 14.0, 0.0),
            normal: DVec3::Z,
            x_axis: DVec3::X,
            radius: 6.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            segments: 6,
        };
        for (a, b) in [
            (p(0.0, 0.0, 0.0), p(20.0, 0.0, 0.0)),
            (p(20.0, 0.0, 0.0), p(20.0, 14.0, 0.0)),
            (p(14.0, 20.0, 0.0), p(0.0, 20.0, 0.0)),
            (p(0.0, 20.0, 0.0), p(0.0, 0.0, 0.0)),
        ] {
            mt_kernel::add_line(&mut e, a, b);
        }
        mt_kernel::add_curve(&mut e, &arc.points(), Some(arc));
        assert_eq!(e.faces.len(), 1);
        // Inside the face (the arc's concave side): nothing.
        assert!(!crate::tools::unround_at(&mut e, p(15.0, 15.0, 0.0)));
        assert!(crate::tools::unround_at(&mut e, p(19.5, 19.5, 0.0)));
        assert!(e.curves.is_empty());
        assert!(e.find_vertex(p(20.0, 20.0, 0.0)).is_some());
        assert_eq!(e.faces.len(), 1);
        assert_eq!(e.faces.values().next().unwrap().outer.coedges.len(), 4);
    }
}
