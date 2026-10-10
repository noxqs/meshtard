//! Push/Pull, Move/Rotate/Scale and their copies, one SketchUp scenario per test. The
//! checker runs after every step.

use glam::{DMat4, DVec3};
use mt_kernel::{
    Checks, Entities, FaceAttrs, FaceId, add_curve, add_line, copy_geometry, push_pull,
    transform_vertices, validate, vertices_of,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn check(ents: &Entities) {
    let errs = validate(ents, Checks::ALL);
    assert!(errs.is_empty(), "invalid: {errs:#?}");
}

fn rect(ents: &mut Entities, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) {
    let c = [p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)];
    for i in 0..4 {
        add_line(ents, c[i], c[(i + 1) % 4]);
        check(ents);
    }
}

/// The face whose plane holds `pt` and that contains it.
fn face_at(ents: &Entities, pt: DVec3) -> FaceId {
    ents.faces
        .keys()
        .find(|&f| ents.face_contains(f, pt))
        .unwrap_or_else(|| panic!("no face at {pt}"))
}

fn pp(ents: &mut Entities, f: FaceId, d: f64, new_faces: bool) -> Option<FaceId> {
    let r = push_pull(ents, f, d, new_faces);
    check(ents);
    r
}

/// A 10 x 10 x `h` box from a ground rectangle, as SketchUp users make one.
fn make_box(ents: &mut Entities, x0: f64, h: f64) -> FaceId {
    rect(ents, x0, 0.0, x0 + 10.0, 10.0, 0.0);
    let base = face_at(ents, p(x0 + 5.0, 5.0, 0.0));
    assert_eq!(ents.faces[base].normal, DVec3::NEG_Z);
    // The ground face looks down: pulling it up is a negative distance along its normal.
    pp(ents, base, -h, false).expect("top face")
}

/// Every edge borders exactly two faces that use it in opposite directions, and every
/// face looks away from `center`.
fn assert_closed_outward(ents: &Entities, center: DVec3) {
    for edge in ents.edges.values() {
        assert_eq!(edge.faces.len(), 2, "manifold solid");
        let dirs: Vec<bool> = edge
            .faces
            .iter()
            .map(|&f| {
                ents.faces[f]
                    .loops()
                    .flat_map(|l| &l.coedges)
                    .find(|c| {
                        let e = &ents.edges[c.edge];
                        e.start == edge.start && e.end == edge.end
                    })
                    .unwrap()
                    .reversed
            })
            .collect();
        assert_ne!(dirs[0], dirs[1], "shared edge runs opposite ways");
    }
    for f in ents.faces.keys() {
        assert!(
            ents.face_plane(f).distance(center) < 0.0,
            "face normal points out of the solid"
        );
    }
}

fn zmax(ents: &Entities) -> f64 {
    ents.vertices
        .values()
        .map(|v| v.pos.z)
        .fold(f64::MIN, f64::max)
}

#[test]
fn rectangle_pulled_up_is_a_closed_outward_box() {
    let mut e = Entities::new();
    let top = make_box(&mut e, 0.0, 10.0);
    assert_eq!(e.faces.len(), 6);
    assert_eq!(e.edges.len(), 12);
    assert_eq!(e.vertices.len(), 8);
    assert_eq!(e.faces[top].normal, DVec3::Z);
    assert!((e.face_plane(top).d - 10.0).abs() < 1e-9);
    assert_closed_outward(&e, p(5.0, 5.0, 5.0));
}

#[test]
fn free_face_pushed_along_its_normal_keeps_outward_normals() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    let base = face_at(&e, p(5.0, 5.0, 0.0));
    let top = pp(&mut e, base, 4.0, false).unwrap();
    assert_eq!(e.faces[top].normal, DVec3::NEG_Z);
    assert_eq!(e.faces.len(), 6);
    assert_closed_outward(&e, p(5.0, 5.0, -2.0));
}

#[test]
fn box_top_pulled_again_stretches() {
    let mut e = Entities::new();
    let top = make_box(&mut e, 0.0, 10.0);
    let again = pp(&mut e, top, 5.0, false).unwrap();
    assert_eq!(again, top, "the face moves, keeping its id");
    assert_eq!(e.faces.len(), 6);
    assert_eq!(e.edges.len(), 12);
    assert!((zmax(&e) - 15.0).abs() < 1e-9);
    assert_closed_outward(&e, p(5.0, 5.0, 7.0));
}

#[test]
fn box_top_pulled_with_new_faces_adds_a_layer() {
    let mut e = Entities::new();
    let top = make_box(&mut e, 0.0, 10.0);
    let new_top = pp(&mut e, top, 5.0, true).unwrap();
    assert!(!e.faces.contains_key(top), "old top is not free-standing");
    assert_eq!(e.faces.len(), 10);
    assert_eq!(e.faces[new_top].normal, DVec3::Z);
    assert!((e.face_plane(new_top).d - 15.0).abs() < 1e-9);
}

#[test]
fn box_top_pushed_in_with_new_faces_stays_valid() {
    let mut e = Entities::new();
    let top = make_box(&mut e, 0.0, 10.0);
    let new_top = pp(&mut e, top, -4.0, true).unwrap();
    assert!((e.face_plane(new_top).d - 6.0).abs() < 1e-9);
    assert_closed_outward(&e, p(5.0, 5.0, 3.0));
}

#[test]
fn rectangle_on_box_top_pushed_through_makes_a_hole() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    rect(&mut e, 3.0, 3.0, 6.0, 6.0, 10.0);
    assert_eq!(e.faces.len(), 7);
    let inner = face_at(&e, p(4.5, 4.5, 10.0));
    assert_eq!(pp(&mut e, inner, -10.0, false), None, "cut through");
    // Top and bottom with holes, four outer sides, four hole walls.
    assert_eq!(e.faces.len(), 10);
    assert_eq!(e.faces.values().filter(|f| f.holes.len() == 1).count(), 2);
    assert!(
        !e.faces.keys().any(|f| e.face_contains(f, p(4.5, 4.5, 0.0))),
        "bottom is open under the hole"
    );
    assert!(
        !e.faces
            .keys()
            .any(|f| e.face_contains(f, p(4.5, 4.5, 10.0)))
    );
    assert_closed_outward_hole(&e);
}

/// Every edge of a box with a hole through it borders two faces.
fn assert_closed_outward_hole(e: &Entities) {
    for edge in e.edges.values() {
        assert_eq!(edge.faces.len(), 2);
    }
}

#[test]
fn rectangle_on_box_top_pushed_part_way_makes_a_pocket() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    rect(&mut e, 3.0, 3.0, 6.0, 6.0, 10.0);
    let inner = face_at(&e, p(4.5, 4.5, 10.0));
    let bottom = pp(&mut e, inner, -4.0, false).unwrap();
    assert_eq!(e.faces[bottom].normal, DVec3::Z, "pocket floor looks up");
    assert!((e.face_plane(bottom).d - 6.0).abs() < 1e-9);
    // Six box faces plus four pocket walls and the floor.
    assert_eq!(e.faces.len(), 11);
    for edge in e.edges.values() {
        assert_eq!(edge.faces.len(), 2);
    }
    // Walls look into the pocket.
    let wall = face_at(&e, p(3.0, 4.5, 8.0));
    assert!(e.faces[wall].normal.x > 0.99, "wall faces the pocket");
}

#[test]
fn rectangle_on_box_top_pulled_up_makes_a_bump() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    rect(&mut e, 3.0, 3.0, 6.0, 6.0, 10.0);
    let inner = face_at(&e, p(4.5, 4.5, 10.0));
    let top = pp(&mut e, inner, 3.0, false).unwrap();
    assert!((e.face_plane(top).d - 13.0).abs() < 1e-9);
    assert_eq!(e.faces.len(), 11);
}

#[test]
fn face_with_hole_pulled_gets_hole_walls() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    rect(&mut e, 3.0, 3.0, 6.0, 6.0, 0.0);
    let inner = face_at(&e, p(4.5, 4.5, 0.0));
    let outer = face_at(&e, p(1.0, 1.0, 0.0));
    // Erase the inner face so the outer one is free-standing with a hole.
    e.remove_face(inner);
    check(&e);
    let top = pp(&mut e, outer, -5.0, false).unwrap();
    assert_eq!(e.faces[top].holes.len(), 1);
    assert_eq!(e.faces.len(), 10, "top, base, 4 sides, 4 hole walls");
    for edge in e.edges.values() {
        assert_eq!(edge.faces.len(), 2);
    }
}

#[test]
fn zero_distance_does_nothing() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    let f = face_at(&e, p(5.0, 5.0, 0.0));
    assert_eq!(pp(&mut e, f, 0.0, false), None);
    assert_eq!(e.faces.len(), 1);
}

#[test]
fn moving_a_box_top_stretches_the_box() {
    let mut e = Entities::new();
    let top = make_box(&mut e, 0.0, 10.0);
    let vs = vertices_of(&e, &[], &[top]);
    assert_eq!(vs.len(), 4);
    transform_vertices(&mut e, &vs, DMat4::from_translation(p(0.0, 0.0, 6.0)));
    check(&e);
    assert!(e.faces.contains_key(top));
    assert_eq!(e.faces.len(), 6);
    assert!((zmax(&e) - 16.0).abs() < 1e-9);
    assert_closed_outward(&e, p(5.0, 5.0, 8.0));
}

#[test]
fn moving_a_box_top_sideways_shears_the_box() {
    let mut e = Entities::new();
    let top = make_box(&mut e, 0.0, 10.0);
    let vs = vertices_of(&e, &[], &[top]);
    transform_vertices(&mut e, &vs, DMat4::from_translation(p(4.0, 0.0, 0.0)));
    check(&e);
    assert_eq!(e.faces.len(), 6);
}

#[test]
fn moving_one_corner_out_folds_faces() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    let corner = e.find_vertex(p(10.0, 10.0, 10.0)).unwrap();
    transform_vertices(&mut e, &[corner], DMat4::from_translation(p(1.0, 2.0, 3.0)));
    check(&e);
    assert_eq!(e.faces.len(), 9, "top and two sides fold in two");
    assert_eq!(e.edges.len(), 15);
    let folds = e
        .edges
        .values()
        .filter(|x| x.attrs.soft && x.attrs.smooth)
        .count();
    assert_eq!(folds, 3, "fold edges are soft and smooth");
    for edge in e.edges.values() {
        assert_eq!(edge.faces.len(), 2, "still closed");
    }
    assert!(e.find_vertex(p(11.0, 12.0, 13.0)).is_some());
}

#[test]
fn moving_one_corner_straight_up_folds_only_the_top() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    let corner = e.find_vertex(p(10.0, 10.0, 10.0)).unwrap();
    transform_vertices(&mut e, &[corner], DMat4::from_translation(p(0.0, 0.0, 3.0)));
    check(&e);
    assert_eq!(e.faces.len(), 7, "the sides stay planar");
    assert_eq!(e.edges.len(), 13);
}

#[test]
fn moving_a_rectangle_onto_another_welds_the_corner() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    rect(&mut e, 20.0, 0.0, 30.0, 10.0, 0.0);
    assert_eq!(e.vertices.len(), 8);
    let f = face_at(&e, p(25.0, 5.0, 0.0));
    let vs = vertices_of(&e, &[], &[f]);
    // Corner (20, 0) lands on (10, 10), within tolerance.
    transform_vertices(&mut e, &vs, DMat4::from_translation(p(-10.0004, 10.0, 0.0)));
    check(&e);
    assert_eq!(e.vertices.len(), 7);
    assert_eq!(e.faces.len(), 2);
}

#[test]
fn moving_a_rectangle_onto_an_edge_merges_edges() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    rect(&mut e, 20.0, 0.0, 30.0, 10.0, 0.0);
    let f = face_at(&e, p(25.0, 5.0, 0.0));
    let vs = vertices_of(&e, &[], &[f]);
    transform_vertices(&mut e, &vs, DMat4::from_translation(p(-10.0, 0.0, 0.0)));
    check(&e);
    assert_eq!(e.vertices.len(), 6);
    assert_eq!(e.edges.len(), 7, "the shared side is one edge");
    assert_eq!(e.faces.len(), 2);
}

#[test]
fn copying_a_box_makes_a_second_box() {
    let mut e = Entities::new();
    let mat = FaceAttrs {
        material: Some(3),
        ..Default::default()
    };
    let top = make_box(&mut e, 0.0, 10.0);
    e.faces[top].attrs = mat.clone();
    let edges: Vec<_> = e.edges.keys().collect();
    let faces: Vec<_> = e.faces.keys().collect();
    let (ne, nf) = copy_geometry(
        &mut e,
        &edges,
        &faces,
        DMat4::from_translation(p(20.0, 0.0, 0.0)),
    );
    check(&e);
    assert_eq!(e.faces.len(), 12);
    assert_eq!(e.edges.len(), 24);
    assert_eq!((ne.len(), nf.len()), (12, 6));
    let copied_top = face_at(&e, p(25.0, 5.0, 10.0));
    assert_eq!(e.faces[copied_top].attrs, mat);
    assert_eq!(e.faces[copied_top].normal, DVec3::Z);
    for f in &nf {
        let pl = e.face_plane(*f);
        assert!(
            pl.distance(p(25.0, 5.0, 5.0)) < 0.0,
            "copied face looks out"
        );
    }
}

#[test]
fn copying_a_box_against_itself_merges_the_shared_side() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    let edges: Vec<_> = e.edges.keys().collect();
    let faces: Vec<_> = e.faces.keys().collect();
    copy_geometry(
        &mut e,
        &edges,
        &faces,
        DMat4::from_translation(p(10.0, 0.0, 0.0)),
    );
    check(&e);
    assert_eq!(e.vertices.len(), 12);
    assert_eq!(e.edges.len(), 20);
}

fn circle(center: DVec3, radius: f64) -> mt_kernel::Arc {
    mt_kernel::Arc {
        center,
        normal: DVec3::Z,
        x_axis: DVec3::X,
        radius,
        start_angle: 0.0,
        end_angle: std::f64::consts::TAU,
        segments: 24,
    }
}

#[test]
fn copying_a_circle_keeps_it_a_curve() {
    let mut e = Entities::new();
    let c = circle(DVec3::ZERO, 5.0);
    let edges = add_curve(&mut e, &c.points(), Some(c));
    let faces: Vec<_> = e.faces.keys().collect();
    let old = e.curves.keys().next().unwrap();
    let (ne, nf) = copy_geometry(
        &mut e,
        &edges,
        &faces,
        DMat4::from_translation(p(20.0, 0.0, 0.0)),
    );
    check(&e);
    assert_eq!((ne.len(), nf.len()), (24, 1));
    assert_eq!(e.curves.len(), 2);
    let new = e.edges[ne[0]]
        .attrs
        .curve
        .expect("copied edges are on a curve");
    assert_ne!(new, old);
    assert_eq!(e.curve_edges(new).len(), 24);
    let arc = e.curves[new].arc.expect("still an arc");
    assert!((arc.center - p(20.0, 0.0, 0.0)).length() < 1e-9);
    assert!((arc.radius - 5.0).abs() < 1e-9);
}

#[test]
fn moving_a_circle_moves_its_arc() {
    let mut e = Entities::new();
    let c = circle(DVec3::ZERO, 5.0);
    let edges = add_curve(&mut e, &c.points(), Some(c));
    let vs = vertices_of(&e, &edges, &[]);
    transform_vertices(&mut e, &vs, DMat4::from_scale(DVec3::splat(2.0)));
    check(&e);
    let arc = e.curves.values().next().unwrap().arc.unwrap();
    assert!((arc.radius - 10.0).abs() < 1e-9);
}

#[test]
fn rotating_and_scaling_a_box_keeps_it_valid() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    let all: Vec<_> = e.vertices.keys().collect();
    transform_vertices(&mut e, &all, DMat4::from_rotation_z(0.7));
    check(&e);
    transform_vertices(&mut e, &all, DMat4::from_scale(p(2.0, 0.5, 3.0)));
    check(&e);
    transform_vertices(&mut e, &all, DMat4::from_rotation_x(0.3));
    check(&e);
    // Mirror: faces keep looking out.
    transform_vertices(&mut e, &all, DMat4::from_scale(p(-1.0, 1.0, 1.0)));
    check(&e);
    assert_eq!(e.faces.len(), 6);
    let c = e.vertices.values().map(|v| v.pos).sum::<DVec3>() / 8.0;
    assert_closed_outward(&e, c);
}

#[test]
fn rotating_part_of_a_box_folds_or_stays_valid() {
    let mut e = Entities::new();
    let top = make_box(&mut e, 0.0, 10.0);
    let vs = vertices_of(&e, &[], &[top]);
    let about = DMat4::from_translation(p(5.0, 5.0, 10.0))
        * DMat4::from_rotation_z(0.5)
        * DMat4::from_translation(p(-5.0, -5.0, -10.0));
    transform_vertices(&mut e, &vs, about);
    check(&e);
    for edge in e.edges.values() {
        assert_eq!(edge.faces.len(), 2, "twisted box stays closed");
    }
}

#[test]
fn copy_rotated_and_scaled_stays_valid() {
    let mut e = Entities::new();
    make_box(&mut e, 0.0, 10.0);
    let edges: Vec<_> = e.edges.keys().collect();
    let faces: Vec<_> = e.faces.keys().collect();
    let xf = DMat4::from_translation(p(3.0, 3.0, 0.0)) * DMat4::from_rotation_z(0.4);
    copy_geometry(&mut e, &edges, &faces, xf);
    check(&e);
    copy_geometry(&mut e, &edges, &faces, DMat4::from_scale(DVec3::splat(0.5)));
    check(&e);
}
