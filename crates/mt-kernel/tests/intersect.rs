//! Intersect Faces scenarios. The checker runs after every step.

use glam::DVec3;
use mt_kernel::{Checks, Entities, FaceId, add_line, intersect_faces, validate};

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

/// An axis-aligned box from lines.
fn boxed(ents: &mut Entities, lo: DVec3, hi: DVec3) {
    for z in [lo.z, hi.z] {
        polygon(
            ents,
            &[
                p(lo.x, lo.y, z),
                p(hi.x, lo.y, z),
                p(hi.x, hi.y, z),
                p(lo.x, hi.y, z),
            ],
        );
    }
    for (x, y) in [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)] {
        add_line(ents, p(x, y, lo.z), p(x, y, hi.z));
        check(ents);
    }
}

fn all_faces(ents: &Entities) -> Vec<FaceId> {
    ents.faces.keys().collect()
}

#[test]
fn crossing_boxes_get_the_intersection_loop() {
    let mut e = Entities::new();
    boxed(&mut e, p(0.0, 0.0, 0.0), p(10.0, 10.0, 10.0));
    boxed(&mut e, p(5.0, 5.0, 5.0), p(15.0, 15.0, 15.0));
    assert_eq!(e.faces.len(), 12);
    let faces = all_faces(&e);
    let new = intersect_faces(&mut e, &faces, &faces);
    check(&e);
    // Six segments, each 5 long, forming a closed loop through the two boxes.
    assert_eq!(new.len(), 6);
    let total: f64 = new
        .iter()
        .map(|&x| e.pos(e.edges[x].start).distance(e.pos(e.edges[x].end)))
        .sum();
    assert!((total - 30.0).abs() < 1e-9);
    for &x in &new {
        for v in [e.edges[x].start, e.edges[x].end] {
            let q = e.pos(v);
            let on = |c: f64| c == 5.0 || c == 10.0;
            assert!(on(q.x) && on(q.y) && on(q.z), "loop vertex {q}");
            assert_eq!(
                e.vertices[v]
                    .edges
                    .iter()
                    .filter(|x| new.contains(x))
                    .count(),
                2,
                "closed loop"
            );
        }
    }
    // The three faces of each box inside the other are split in two.
    assert_eq!(e.faces.len(), 18);
    for &x in &new {
        assert_eq!(
            e.edges[x].faces.len(),
            4,
            "borders both halves of a face of each box"
        );
    }

    // Intersecting again adds nothing.
    let faces = all_faces(&e);
    assert!(intersect_faces(&mut e, &faces, &faces).is_empty());
    check(&e);
    assert_eq!(e.faces.len(), 18);
}

#[test]
fn a_single_box_intersects_with_nothing_new() {
    let mut e = Entities::new();
    boxed(&mut e, p(0.0, 0.0, 0.0), p(10.0, 10.0, 10.0));
    let faces = all_faces(&e);
    assert!(intersect_faces(&mut e, &faces, &faces).is_empty());
    check(&e);
    assert_eq!((e.edges.len(), e.faces.len()), (12, 6));
}

/// A horizontal 10x10 face with a 4x4 hole, and a vertical face through it.
fn plate_with_hole(e: &mut Entities) -> FaceId {
    polygon(
        e,
        &[
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 10.0, 0.0),
            p(0.0, 10.0, 0.0),
        ],
    );
    polygon(
        e,
        &[
            p(3.0, 3.0, 0.0),
            p(7.0, 3.0, 0.0),
            p(7.0, 7.0, 0.0),
            p(3.0, 7.0, 0.0),
        ],
    );
    // Erase the hole's face so the hole is open.
    let inner = e.faces.iter().find(|(_, f)| f.holes.is_empty()).unwrap().0;
    e.remove_face(inner);
    check(e);
    e.faces.keys().next().unwrap()
}

#[test]
fn face_through_a_hole_adds_nothing_inside_it() {
    let mut e = Entities::new();
    let plate = plate_with_hole(&mut e);
    polygon(
        &mut e,
        &[
            p(5.0, 4.0, -2.0),
            p(5.0, 6.0, -2.0),
            p(5.0, 6.0, 2.0),
            p(5.0, 4.0, 2.0),
        ],
    );
    let wall = e.faces.keys().find(|&f| f != plate).unwrap();
    assert!(intersect_faces(&mut e, &[wall], &[plate]).is_empty());
    check(&e);
    assert_eq!(e.faces.len(), 2);
}

#[test]
fn face_across_a_hole_is_cut_on_both_sides() {
    let mut e = Entities::new();
    let plate = plate_with_hole(&mut e);
    polygon(
        &mut e,
        &[
            p(5.0, 1.0, -2.0),
            p(5.0, 9.0, -2.0),
            p(5.0, 9.0, 2.0),
            p(5.0, 1.0, 2.0),
        ],
    );
    let wall = e.faces.keys().find(|&f| f != plate).unwrap();
    let new = intersect_faces(&mut e, &[wall], &[plate]);
    check(&e);
    assert_eq!(new.len(), 2);
    let mut ys: Vec<f64> = new
        .iter()
        .flat_map(|&x| [e.edges[x].start, e.edges[x].end])
        .map(|v| e.pos(v).y)
        .collect();
    ys.sort_by(f64::total_cmp);
    assert_eq!(ys, [1.0, 3.0, 7.0, 9.0], "nothing inside the hole");
    assert!(new.iter().all(|&x| {
        let (a, b) = (e.pos(e.edges[x].start), e.pos(e.edges[x].end));
        a.x == 5.0 && b.x == 5.0 && a.z == 0.0 && b.z == 0.0
    }));
}

#[test]
fn wall_through_a_floor_splits_it() {
    let mut e = Entities::new();
    polygon(
        &mut e,
        &[
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 10.0, 0.0),
            p(0.0, 10.0, 0.0),
        ],
    );
    // A slanted wall crossing the whole floor and beyond it.
    polygon(
        &mut e,
        &[
            p(-5.0, 3.0, -2.0),
            p(15.0, 3.0, -2.0),
            p(15.0, 7.0, 2.0),
            p(-5.0, 7.0, 2.0),
        ],
    );
    assert_eq!(e.faces.len(), 2);
    let faces = all_faces(&e);
    let new = intersect_faces(&mut e, &faces, &faces);
    check(&e);
    assert_eq!(new.len(), 1);
    let (a, b) = (e.pos(e.edges[new[0]].start), e.pos(e.edges[new[0]].end));
    assert!((a.distance(b) - 10.0).abs() < 1e-9 && a.y == 5.0 && b.y == 5.0);
    assert_eq!(e.faces.len(), 3, "the floor splits; the wall does not");
}
