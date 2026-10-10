//! Follow Me scenarios. The checker runs after every step.

use glam::DVec3;
use mt_kernel::{Checks, EdgeId, Entities, FaceId, add_curve, add_line, follow_me, validate};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn check(ents: &Entities) {
    let errs = validate(ents, Checks::ALL);
    assert!(errs.is_empty(), "invalid: {errs:#?}");
}

fn polygon(ents: &mut Entities, pts: &[DVec3]) {
    for i in 0..pts.len() {
        add_line(ents, pts[i], pts[(i + 1) % pts.len()]);
        check(ents);
    }
}

fn path(ents: &mut Entities, pts: &[DVec3]) -> Vec<EdgeId> {
    let mut out = Vec::new();
    for w in pts.windows(2) {
        out.extend(add_line(ents, w[0], w[1]));
        check(ents);
    }
    out
}

/// The face whose vertices all satisfy `pred`.
fn face_where(ents: &Entities, pred: impl Fn(DVec3) -> bool) -> FaceId {
    ents.faces
        .iter()
        .find(|(_, f)| ents.loop_points(&f.outer).into_iter().all(&pred))
        .map(|(id, _)| id)
        .expect("face exists")
}

/// A 2x2 rectangle profile in the XZ plane (y = 0), floating 1 above the ground.
fn xz_profile(ents: &mut Entities) -> FaceId {
    polygon(
        ents,
        &[
            p(-1.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 0.0, 3.0),
            p(-1.0, 0.0, 3.0),
        ],
    );
    face_where(ents, |q| q.y == 0.0)
}

/// Every edge of the sweep (all but the path's) borders exactly two faces.
/// Also, the two faces run it in opposite directions (consistent orientation).
fn assert_closed(ents: &Entities, path_edges: &[EdgeId]) {
    for (id, e) in &ents.edges {
        if path_edges.contains(&id) {
            continue;
        }
        assert_eq!(e.faces.len(), 2, "edge {id:?} borders {:?}", e.faces);
        let dirs: Vec<bool> = e
            .faces
            .iter()
            .map(|&f| {
                ents.faces[f]
                    .loops()
                    .flat_map(|l| &l.coedges)
                    .find(|c| c.edge == id)
                    .unwrap()
                    .reversed
            })
            .collect();
        assert_ne!(
            dirs[0], dirs[1],
            "edge {id:?}: faces disagree on orientation"
        );
    }
}

#[test]
fn rectangle_along_l_path_is_mitered() {
    let mut e = Entities::new();
    let prof = xz_profile(&mut e);
    let l = path(
        &mut e,
        &[p(0.0, 0.0, 0.0), p(0.0, 10.0, 0.0), p(10.0, 10.0, 0.0)],
    );
    let new = follow_me(&mut e, prof, &l);
    check(&e);
    // Four sides per segment and the end cap; the profile stays as the start cap.
    assert_eq!(new.len(), 9);
    assert_eq!(e.faces.len(), 10);
    assert!(e.faces.contains_key(prof));
    // Edges: 3 sections of 4, 2 x 4 along the path, plus the path.
    assert_eq!(e.edges.len(), 12 + 8 + 2);
    assert_closed(&e, &l);
    // The corner section lies on the 45 degree miter plane x - y = -10 + ... through
    // the joint (0, 10): outer corner (-1, 11), inner corner (1, 9).
    for (x, y) in [(-1.0, 11.0), (1.0, 9.0)] {
        for z in [1.0, 3.0] {
            assert!(
                e.find_vertex(p(x, y, z)).is_some(),
                "miter vertex {x},{y},{z}"
            );
        }
    }
    // The end cap lies in the plane x = 10.
    let cap = face_where(&e, |q| q.x == 10.0);
    assert_eq!(e.faces[cap].outer.coedges.len(), 4);
    // Faces point out; the volume is the profile's area times the path length.
    assert!((volume(&e) - 80.0).abs() < 1e-9, "volume {}", volume(&e));
    // A 90 degree joint stays hard.
    assert!(e.edges.values().all(|x| !x.attrs.soft));
}

#[test]
fn profile_on_the_path_keeps_the_path_on_the_bottom() {
    // SketchUp's usual setup: the path starts on the profile's bottom edge.
    let mut e = Entities::new();
    polygon(
        &mut e,
        &[
            p(-1.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 0.0, 2.0),
            p(-1.0, 0.0, 2.0),
        ],
    );
    let l = path(
        &mut e,
        &[p(0.0, 0.0, 0.0), p(0.0, 10.0, 0.0), p(10.0, 10.0, 0.0)],
    );
    let prof = face_where(&e, |q| q.y == 0.0);
    let new = follow_me(&mut e, prof, &l);
    check(&e);
    // The path divides each bottom face in two.
    assert_eq!(new.len(), 11);
    assert_closed(&e, &[]);
}

#[test]
fn circle_along_closed_square_makes_a_ring() {
    let mut e = Entities::new();
    let c = mt_kernel::Arc {
        center: p(0.0, 0.0, 3.0),
        normal: DVec3::X,
        x_axis: DVec3::Y,
        radius: 1.0,
        start_angle: 0.0,
        end_angle: std::f64::consts::TAU,
        segments: 12,
    };
    add_curve(&mut e, &c.points(), Some(c));
    check(&e);
    let prof = e.faces.keys().next().unwrap();
    let sq = path(
        &mut e,
        &[
            p(0.0, 0.0, 0.0),
            p(20.0, 0.0, 0.0),
            p(20.0, 20.0, 0.0),
            p(0.0, 20.0, 0.0),
            p(0.0, 0.0, 0.0),
        ],
    );
    let ground = face_where(&e, |q| q.z == 0.0);
    let new = follow_me(&mut e, prof, &sq);
    check(&e);
    assert_eq!(new.len(), 4 * 12, "only side faces: no caps");
    assert_eq!(e.faces.len(), 4 * 12 + 1, "plus the square's own face");
    assert!(!e.faces.contains_key(prof), "the profile is used up");
    assert!(e.faces.contains_key(ground));
    // 4 corner sections of 12 edges and 4 x 12 edges along the path, plus the path.
    assert_eq!(e.edges.len(), 48 + 48 + 4);
    assert_closed(
        &e,
        &e.faces[ground]
            .outer
            .coedges
            .iter()
            .map(|c| c.edge)
            .collect::<Vec<_>>(),
    );
    // Sharp corners stay hard; the tube's sides are smooth (swept from a curve).
    let soft = e
        .edges
        .values()
        .filter(|x| x.attrs.soft && x.attrs.smooth)
        .count();
    assert_eq!(soft, 48);
    // (The ground face at z = 0 adds no volume.)
    assert!(volume(&e) > 0.0, "faces point out");
    // The profile curve's record is gone with its edges.
    assert!(e.curves.is_empty());
}

#[test]
fn profile_along_circle_is_a_smooth_lathe() {
    let mut e = Entities::new();
    let prof = {
        polygon(
            &mut e,
            &[
                p(9.0, 0.0, 1.0),
                p(11.0, 0.0, 1.0),
                p(11.0, 0.0, 3.0),
                p(9.0, 0.0, 3.0),
            ],
        );
        face_where(&e, |q| q.y == 0.0)
    };
    let c = mt_kernel::Arc {
        center: DVec3::ZERO,
        normal: DVec3::Z,
        x_axis: DVec3::X,
        radius: 10.0,
        start_angle: 0.0,
        end_angle: std::f64::consts::TAU,
        segments: 24,
    };
    let circle = add_curve(&mut e, &c.points(), Some(c));
    check(&e);
    let new = follow_me(&mut e, prof, &circle);
    check(&e);
    assert_eq!(new.len(), 24 * 4, "a closed ring: no caps");
    assert!(!e.faces.contains_key(prof));
    // Section edges between consecutive segments (15 degree joints) are soft and
    // smooth; the rectangle's corners are swept as hard edges.
    let mut section_edges = 0;
    for x in e.edges.values() {
        let (a, b) = (e.pos(x.start), e.pos(x.end));
        if x.attrs.curve.is_some() {
            continue; // the path
        }
        let radial = {
            let (ra, rb) = (a.truncate().normalize(), b.truncate().normalize());
            ra.dot(rb) > 1.0 - 1e-9
        };
        if radial {
            section_edges += 1;
            assert!(x.attrs.soft && x.attrs.smooth, "joint edge {a} {b}");
        } else {
            assert!(!x.attrs.soft, "swept edge {a} {b}");
        }
    }
    assert_eq!(section_edges, 24 * 4);
    assert!(volume(&e) > 0.0, "faces point out");
    let path_edges: Vec<EdgeId> = e
        .edges
        .iter()
        .filter(|(_, x)| x.attrs.curve.is_some())
        .map(|(id, _)| id)
        .collect();
    assert_closed(&e, &path_edges);
}

#[test]
fn profile_with_hole_makes_a_tube() {
    let mut e = Entities::new();
    polygon(
        &mut e,
        &[
            p(-2.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 0.0, 4.0),
            p(-2.0, 0.0, 4.0),
        ],
    );
    polygon(
        &mut e,
        &[
            p(-1.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 0.0, 3.0),
            p(-1.0, 0.0, 3.0),
        ],
    );
    let prof = e
        .faces
        .iter()
        .find(|(_, f)| f.holes.len() == 1)
        .map(|(id, _)| id)
        .unwrap();
    let inner = e.faces.keys().find(|&f| f != prof).unwrap();
    e.remove_face(inner);
    let l = path(&mut e, &[p(0.0, 0.0, -5.0), p(0.0, 10.0, -5.0)]);
    let new = follow_me(&mut e, prof, &l);
    check(&e);
    assert_eq!(new.len(), 8 + 1, "outer and inner sides, and the end cap");
    let cap = face_where(&e, |q| q.y == 10.0);
    assert_eq!(
        e.faces[cap].holes.len(),
        1,
        "the end cap keeps the hole open"
    );
    assert_closed(&e, &l);
    assert!((volume(&e) - 120.0).abs() < 1e-9, "(16 - 4) x 10");
}

#[test]
fn collinear_path_edges_sweep_as_one() {
    let mut a = Entities::new();
    let prof = xz_profile(&mut a);
    let l = path(
        &mut a,
        &[p(0.0, 0.0, 0.0), p(0.0, 4.0, 0.0), p(0.0, 10.0, 0.0)],
    );
    assert_eq!(l.len(), 2);
    let new = follow_me(&mut a, prof, &l);
    check(&a);
    assert_eq!(new.len(), 5, "4 sides and the end cap, no joint in between");
    assert_eq!(a.edges.len(), 12 + 2);
}

#[test]
fn degenerate_inputs_change_nothing() {
    let mut e = Entities::new();
    let prof = xz_profile(&mut e);
    let (edges, faces) = (e.edges.len(), e.faces.len());
    // Empty path.
    assert!(follow_me(&mut e, prof, &[]).is_empty());
    // A path whose first segment runs in the profile's plane (along x, at y = 0).
    let flat = path(
        &mut e,
        &[p(5.0, 0.0, 0.0), p(8.0, 0.0, 0.0), p(8.0, 5.0, 0.0)],
    );
    let (edges, faces) = (edges + 2, faces);
    assert!(follow_me(&mut e, prof, &flat).is_empty());
    // Edges that are not a chain.
    let other = path(&mut e, &[p(20.0, 0.0, 0.0), p(20.0, 5.0, 0.0)]);
    assert!(follow_me(&mut e, prof, &[flat[0], other[0]]).is_empty());
    // A path that doubles back on itself.
    let back = path(&mut e, &[p(30.0, 0.0, 0.0), p(30.0, 5.0, 0.0)]);
    assert!(follow_me(&mut e, prof, &[back[0], back[0]]).is_empty());
    check(&e);
    assert_eq!((e.edges.len(), e.faces.len()), (edges + 2, faces));
}

#[test]
fn profile_away_from_the_path_follows_its_shape() {
    // The profile sits 5 to the side of the path's start; the sweep is the path's
    // shape carried along from there.
    let mut e = Entities::new();
    let prof = xz_profile(&mut e);
    let l = path(
        &mut e,
        &[p(5.0, -3.0, 0.0), p(5.0, 10.0, 0.0), p(15.0, 10.0, 0.0)],
    );
    let new = follow_me(&mut e, prof, &l);
    check(&e);
    assert_eq!(new.len(), 9);
    // Miter through the joint (5, 10): the profile at x = -1..1 turns at y = 14..16.
    for (x, y) in [(-1.0, 16.0), (1.0, 14.0)] {
        assert!(
            e.find_vertex(p(x, y, 1.0)).is_some(),
            "miter vertex {x},{y}"
        );
    }
    // The end cap is at the path's end, x = 15.
    face_where(&e, |q| q.x == 15.0);
}

/// Volume enclosed by the faces, positive when they face outward.
fn volume(ents: &Entities) -> f64 {
    ents.faces
        .keys()
        .map(|f| {
            let plane = ents.face_plane(f);
            let (outer, holes) = ents.face_polygons(f, &plane);
            let area = mt_kernel::geom::signed_area2(&outer)
                + holes
                    .iter()
                    .map(|h| mt_kernel::geom::signed_area2(h))
                    .sum::<f64>();
            area / 2.0 * plane.d / 3.0
        })
        .sum()
}
