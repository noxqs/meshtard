//! Offset tool scenarios. The checker runs after every step.

use glam::DVec3;
use mt_kernel::{Checks, Entities, FaceId, add_curve, add_line, offset_face, validate};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn check(ents: &Entities) {
    let errs = validate(ents, Checks::ALL);
    assert!(errs.is_empty(), "invalid: {errs:#?}");
}

fn rect(ents: &mut Entities, x0: f64, y0: f64, x1: f64, y1: f64) {
    let c = [
        p(x0, y0, 0.0),
        p(x1, y0, 0.0),
        p(x1, y1, 0.0),
        p(x0, y1, 0.0),
    ];
    for i in 0..4 {
        add_line(ents, c[i], c[(i + 1) % 4]);
        check(ents);
    }
}

fn area(ents: &Entities, f: FaceId) -> f64 {
    let plane = ents.face_plane(f);
    let (outer, holes) = ents.face_polygons(f, &plane);
    mt_kernel::geom::signed_area2(&outer) / 2.0
        + holes
            .iter()
            .map(|h| mt_kernel::geom::signed_area2(h) / 2.0)
            .sum::<f64>()
}

fn areas(ents: &Entities) -> Vec<f64> {
    let mut v: Vec<f64> = ents
        .faces
        .keys()
        .map(|f| (area(ents, f) * 1e6).round() / 1e6)
        .collect();
    v.sort_by(f64::total_cmp);
    v
}

fn only_face(ents: &Entities) -> FaceId {
    assert_eq!(ents.faces.len(), 1);
    ents.faces.keys().next().unwrap()
}

#[test]
fn inward_offset_makes_ring_and_inner_face() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0);
    let f = only_face(&e);
    let new = offset_face(&mut e, f, 2.0);
    check(&e);
    assert_eq!(new.len(), 4);
    assert_eq!(areas(&e), [36.0, 64.0]);
    for &x in &new {
        for v in [e.edges[x].start, e.edges[x].end] {
            let q = e.pos(v);
            assert!(q.x == 2.0 || q.x == 8.0);
            assert!(q.y == 2.0 || q.y == 8.0);
        }
    }
    assert!(
        e.faces.values().all(|f| f.normal == DVec3::NEG_Z),
        "both pieces keep the face's orientation"
    );
}

#[test]
fn outward_offset_surrounds_a_free_face() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0);
    let f = only_face(&e);
    let new = offset_face(&mut e, f, -2.0);
    check(&e);
    assert_eq!(new.len(), 4);
    let xs: Vec<f64> = new
        .iter()
        .flat_map(|&x| [e.edges[x].start, e.edges[x].end])
        .map(|v| e.pos(v).x)
        .collect();
    assert!(xs.iter().all(|&x| x == -2.0 || x == 12.0), "14x14 loop");
    assert_eq!(areas(&e), [96.0, 100.0], "ring around the face");
}

#[test]
fn collapsing_offset_adds_nothing() {
    for d in [5.0, 6.0, 50.0] {
        let mut e = Entities::new();
        rect(&mut e, 0.0, 0.0, 10.0, 10.0);
        let f = only_face(&e);
        assert!(offset_face(&mut e, f, d).is_empty(), "offset {d}");
        check(&e);
        assert_eq!((e.edges.len(), e.faces.len()), (4, 1));
    }
    // Zero distance is a no-op too.
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0);
    let f = only_face(&e);
    assert!(offset_face(&mut e, f, 0.0).is_empty());
    assert_eq!(e.edges.len(), 4);
}

#[test]
fn offset_of_face_with_hole_grows_the_hole() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0);
    rect(&mut e, 3.0, 3.0, 7.0, 7.0);
    let f = e
        .faces
        .iter()
        .find(|(_, f)| f.holes.len() == 1)
        .map(|(id, _)| id)
        .unwrap();
    let new = offset_face(&mut e, f, 1.0);
    check(&e);
    assert_eq!(new.len(), 8);
    // Hole face 16, outer ring 100-64, ring around the hole 36-16, middle ring 64-36.
    assert_eq!(areas(&e), [16.0, 20.0, 28.0, 36.0]);

    // Growing the hole into the outer loop collapses: nothing happens.
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0);
    rect(&mut e, 3.0, 3.0, 7.0, 7.0);
    let f = e
        .faces
        .iter()
        .find(|(_, f)| f.holes.len() == 1)
        .map(|(id, _)| id)
        .unwrap();
    assert!(offset_face(&mut e, f, 2.0).is_empty());
    check(&e);
    assert_eq!(e.faces.len(), 2);
}

#[test]
fn concave_face_offsets_with_miters() {
    let mut e = Entities::new();
    let pts = [
        p(0.0, 0.0, 0.0),
        p(10.0, 0.0, 0.0),
        p(10.0, 4.0, 0.0),
        p(4.0, 4.0, 0.0),
        p(4.0, 10.0, 0.0),
        p(0.0, 10.0, 0.0),
    ];
    for i in 0..pts.len() {
        add_line(&mut e, pts[i], pts[(i + 1) % pts.len()]);
    }
    check(&e);
    let f = only_face(&e);
    let new = offset_face(&mut e, f, 1.0);
    check(&e);
    assert_eq!(new.len(), 6);
    // The L has area 64; the inner L is [1,9]x[1,3] + [1,3]x[3,9].
    assert_eq!(areas(&e), [28.0, 36.0]);
    // The inner L's arms are 2 wide: offsetting it by 1.5 collapses them.
    let inner = e
        .faces
        .keys()
        .find(|&f| (area(&e, f) - 28.0).abs() < 1e-6)
        .unwrap();
    assert!(offset_face(&mut e, inner, 1.5).is_empty());
    check(&e);
}

fn circle(radius: f64) -> mt_kernel::Arc {
    mt_kernel::Arc {
        center: DVec3::ZERO,
        normal: DVec3::Z,
        x_axis: DVec3::X,
        radius,
        start_angle: 0.0,
        end_angle: std::f64::consts::TAU,
        segments: 24,
    }
}

#[test]
fn circle_offsets_to_concentric_circle() {
    let mut e = Entities::new();
    let c = circle(10.0);
    add_curve(&mut e, &c.points(), Some(c));
    check(&e);
    let f = only_face(&e);
    let new = offset_face(&mut e, f, 2.0);
    check(&e);
    assert_eq!(new.len(), 24);
    assert_eq!(e.curves.len(), 2);
    let curve = e.edges[new[0]].attrs.curve.expect("offset is a curve");
    assert!(new.iter().all(|&x| e.edges[x].attrs.curve == Some(curve)));
    let arc = e.curves[curve].arc.expect("an arc curve");
    assert!((arc.radius - 8.0).abs() < 1e-9);
    assert!(arc.is_circle());
    assert_eq!(arc.segments, 24);
    assert!(arc.center.length() < 1e-9);
    for &x in &new {
        let r = e.pos(e.edges[x].start).length();
        assert!((r - 8.0).abs() < 1e-9, "vertex on the new circle: {r}");
    }
    assert_eq!(e.faces.len(), 2);

    // Outward: a bigger concentric circle.
    let inner = f_with_radius(&e, 8.0);
    let new = offset_face(&mut e, inner, -1.0);
    check(&e);
    let arc = e.curves[e.edges[new[0]].attrs.curve.unwrap()].arc.unwrap();
    assert!((arc.radius - 9.0).abs() < 1e-9);
}

/// The face whose outer loop is the circle of radius `r` (no holes).
fn f_with_radius(e: &Entities, r: f64) -> FaceId {
    e.faces
        .iter()
        .find(|(_, f)| {
            f.holes.is_empty() && {
                let v = e.coedge_ends(f.outer.coedges[0]).0;
                (e.pos(v).length() - r).abs() < 1e-6
            }
        })
        .map(|(id, _)| id)
        .unwrap()
}

#[test]
fn arc_and_lines_keep_the_arc() {
    // A "D" shape: a half circle closed by its diameter.
    let mut e = Entities::new();
    let arc = mt_kernel::Arc {
        end_angle: std::f64::consts::PI,
        segments: 12,
        ..circle(10.0)
    };
    add_curve(&mut e, &arc.points(), Some(arc));
    add_line(&mut e, p(-10.0, 0.0, 0.0), p(10.0, 0.0, 0.0));
    check(&e);
    let f = only_face(&e);
    let new = offset_face(&mut e, f, 1.0);
    check(&e);
    assert_eq!(new.len(), 13);
    assert_eq!(e.faces.len(), 2);
    let curves: Vec<_> = new.iter().filter_map(|&x| e.edges[x].attrs.curve).collect();
    assert_eq!(curves.len(), 12, "the arc part stays one curve");
    let a = e.curves[curves[0]].arc.unwrap();
    assert!((a.radius - 9.0).abs() < 1e-9);
    // Its ends lie on the offset diameter, y = 1.
    let ends: Vec<f64> = new
        .iter()
        .filter(|&&x| e.edges[x].attrs.curve.is_none())
        .flat_map(|&x| [e.edges[x].start, e.edges[x].end])
        .map(|v| e.pos(v).y)
        .collect();
    assert!(ends.iter().all(|y| (y - 1.0).abs() < 1e-9));
}
