//! SketchUp modeling behavior, one scenario per test. The checker runs after every step.

use glam::DVec3;
use mt_kernel::{Checks, EdgeId, Entities, add_line, erase_edge, validate};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn check(ents: &Entities) {
    let errs = validate(ents, Checks::ALL);
    assert!(errs.is_empty(), "invalid: {errs:#?}");
}

fn line(ents: &mut Entities, a: DVec3, b: DVec3) -> Vec<EdgeId> {
    let e = add_line(ents, a, b);
    check(ents);
    e
}

fn polyline(ents: &mut Entities, pts: &[DVec3]) {
    for w in pts.windows(2) {
        line(ents, w[0], w[1]);
    }
}

fn rect(ents: &mut Entities, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) {
    polyline(
        ents,
        &[
            p(x0, y0, z),
            p(x1, y0, z),
            p(x1, y1, z),
            p(x0, y1, z),
            p(x0, y0, z),
        ],
    );
}

fn face_areas(ents: &Entities) -> Vec<f64> {
    let mut v: Vec<f64> = ents
        .faces
        .keys()
        .map(|f| {
            let plane = ents.face_plane(f);
            let (outer, holes) = ents.face_polygons(f, &plane);
            let a = mt_kernel::geom::signed_area2(&outer) / 2.0;
            a + holes
                .iter()
                .map(|h| mt_kernel::geom::signed_area2(h) / 2.0)
                .sum::<f64>()
        })
        .collect();
    v.sort_by(f64::total_cmp);
    v
}

#[test]
fn rectangle_on_ground_makes_downward_face() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 5.0, 0.0);
    assert_eq!(e.faces.len(), 1);
    let f = e.faces.values().next().unwrap();
    assert_eq!(
        f.normal,
        DVec3::NEG_Z,
        "SketchUp faces on the ground plane face down"
    );
    assert_eq!(face_areas(&e), [50.0]);
}

#[test]
fn open_chain_makes_no_face() {
    let mut e = Entities::new();
    polyline(
        &mut e,
        &[p(0.0, 0.0, 0.0), p(10.0, 0.0, 0.0), p(10.0, 10.0, 0.0)],
    );
    assert_eq!(e.faces.len(), 0);
}

#[test]
fn non_planar_loop_makes_no_face() {
    let mut e = Entities::new();
    polyline(
        &mut e,
        &[
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 10.0, 5.0),
            p(0.0, 10.0, 0.0),
            p(0.0, 0.0, 0.0),
        ],
    );
    assert_eq!(e.faces.len(), 0);
    assert_eq!(e.edges.len(), 4);
}

#[test]
fn crossing_lines_split_each_other() {
    let mut e = Entities::new();
    line(&mut e, p(-5.0, 0.0, 0.0), p(5.0, 0.0, 0.0));
    let new = line(&mut e, p(0.0, -5.0, 0.0), p(0.0, 5.0, 0.0));
    assert_eq!(new.len(), 2);
    assert_eq!(e.edges.len(), 4);
    assert_eq!(e.vertices.len(), 5);
}

#[test]
fn line_ending_on_edge_splits_it() {
    let mut e = Entities::new();
    line(&mut e, p(0.0, 0.0, 0.0), p(10.0, 0.0, 0.0));
    line(&mut e, p(4.0, 0.0, 0.0), p(4.0, 6.0, 0.0));
    assert_eq!(e.edges.len(), 3);
}

#[test]
fn collinear_overlap_reuses_edges() {
    let mut e = Entities::new();
    line(&mut e, p(0.0, 0.0, 0.0), p(10.0, 0.0, 0.0));
    let new = line(&mut e, p(5.0, 0.0, 0.0), p(15.0, 0.0, 0.0));
    assert_eq!(new.len(), 2, "5..10 reused, 10..15 new");
    assert_eq!(e.edges.len(), 3);
}

#[test]
fn endpoints_snap_within_tolerance() {
    let mut e = Entities::new();
    line(&mut e, p(0.0, 0.0, 0.0), p(10.0, 0.0, 0.0));
    line(&mut e, p(10.0004, 0.0, 0.0), p(10.0, 10.0, 0.0));
    assert_eq!(e.vertices.len(), 3);
}

#[test]
fn line_across_face_splits_it() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    line(&mut e, p(4.0, 0.0, 0.0), p(4.0, 10.0, 0.0));
    assert_eq!(face_areas(&e), [40.0, 60.0]);
    assert!(
        e.faces.values().all(|f| f.normal == DVec3::NEG_Z),
        "halves keep the orientation"
    );
}

#[test]
fn erasing_the_dividing_line_merges_and_heals() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    let div = line(&mut e, p(4.0, 0.0, 0.0), p(4.0, 10.0, 0.0));
    erase_edge(&mut e, div[0]);
    check(&e);
    assert_eq!(face_areas(&e), [100.0]);
    assert_eq!(
        e.edges.len(),
        4,
        "the split sides heal back into single edges"
    );
    assert_eq!(e.vertices.len(), 4);
}

#[test]
fn erasing_a_boundary_edge_deletes_the_face() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    let edge = e.edges.keys().next().unwrap();
    erase_edge(&mut e, edge);
    check(&e);
    assert_eq!(e.faces.len(), 0);
    assert_eq!(e.edges.len(), 3);
}

#[test]
fn rectangle_inside_face_makes_hole_and_inner_face() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    rect(&mut e, 3.0, 3.0, 6.0, 6.0, 0.0);
    assert_eq!(face_areas(&e), [9.0, 91.0]);
    let with_hole = e.faces.values().filter(|f| f.holes.len() == 1).count();
    assert_eq!(with_hole, 1);
}

#[test]
fn erasing_inner_rectangle_edge_fills_the_hole() {
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    rect(&mut e, 3.0, 3.0, 6.0, 6.0, 0.0);
    let inner = e
        .edges
        .iter()
        .find(|(_, x)| e.pos(x.start).x == 3.0 && e.pos(x.end).x == 3.0)
        .map(|(id, _)| id)
        .unwrap();
    erase_edge(&mut e, inner);
    check(&e);
    assert_eq!(face_areas(&e), [100.0]);
    assert_eq!(
        e.edges.len(),
        7,
        "three inner edges stay, lying on the face"
    );
}

#[test]
fn rectangle_around_face_gets_hole() {
    let mut e = Entities::new();
    rect(&mut e, 3.0, 3.0, 6.0, 6.0, 0.0);
    rect(&mut e, 0.0, 0.0, 10.0, 10.0, 0.0);
    assert_eq!(face_areas(&e), [9.0, 91.0]);
}

#[test]
fn vertical_rectangle_makes_face() {
    let mut e = Entities::new();
    polyline(
        &mut e,
        &[
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 0.0, 5.0),
            p(0.0, 0.0, 5.0),
            p(0.0, 0.0, 0.0),
        ],
    );
    assert_eq!(e.faces.len(), 1);
    assert_eq!(e.faces.values().next().unwrap().normal.y.abs(), 1.0);
}

/// A closed box from lines: six faces, consistently oriented, all facing out.
#[test]
fn box_from_lines_is_closed_and_outward() {
    let mut e = Entities::new();
    let (s, h) = (10.0, 7.0);
    rect(&mut e, 0.0, 0.0, s, s, 0.0);
    rect(&mut e, 0.0, 0.0, s, s, h);
    for (x, y) in [(0.0, 0.0), (s, 0.0), (s, s), (0.0, s)] {
        line(&mut e, p(x, y, 0.0), p(x, y, h));
    }
    assert_eq!(e.faces.len(), 6);
    assert_eq!(e.edges.len(), 12);
    for edge in e.edges.values() {
        assert_eq!(edge.faces.len(), 2, "manifold box");
        let dirs: Vec<bool> = edge
            .faces
            .iter()
            .map(|&f| {
                let face = &e.faces[f];
                face.outer
                    .coedges
                    .iter()
                    .find(|c| {
                        e.edges[c.edge].start == edge.start && e.edges[c.edge].end == edge.end
                    })
                    .unwrap()
                    .reversed
            })
            .collect();
        assert_ne!(
            dirs[0], dirs[1],
            "shared edge runs opposite ways in its two faces"
        );
    }
    let center = p(s / 2.0, s / 2.0, h / 2.0);
    for f in e.faces.keys() {
        let plane = e.face_plane(f);
        assert!(
            plane.distance(center) < 0.0,
            "face normal points out of the box"
        );
    }
}

/// A line from a box edge across the top face splits only the top face.
#[test]
fn splitting_a_box_face_keeps_neighbors() {
    let mut e = Entities::new();
    let s = 10.0;
    rect(&mut e, 0.0, 0.0, s, s, 0.0);
    rect(&mut e, 0.0, 0.0, s, s, s);
    for (x, y) in [(0.0, 0.0), (s, 0.0), (s, s), (0.0, s)] {
        line(&mut e, p(x, y, 0.0), p(x, y, s));
    }
    line(&mut e, p(5.0, 0.0, s), p(5.0, s, s));
    assert_eq!(e.faces.len(), 7);
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
fn circle_is_one_curve_with_a_face() {
    let mut e = Entities::new();
    let c = circle(DVec3::ZERO, 10.0);
    let edges = mt_kernel::add_curve(&mut e, &c.points(), Some(c));
    check(&e);
    assert_eq!(edges.len(), 24);
    assert_eq!(e.faces.len(), 1);
    assert_eq!(e.curves.len(), 1);
    let curve = e.curves.keys().next().unwrap();
    assert_eq!(e.curve_edges(curve).len(), 24);
    mt_kernel::erase_curve(&mut e, curve);
    check(&e);
    assert_eq!((e.edges.len(), e.faces.len(), e.curves.len()), (0, 0, 0));
}

#[test]
fn line_across_circle_keeps_curve_membership() {
    let mut e = Entities::new();
    let c = circle(DVec3::ZERO, 10.0);
    mt_kernel::add_curve(&mut e, &c.points(), Some(c));
    let across = line(&mut e, p(-20.0, 0.3, 0.0), p(20.0, 0.3, 0.0));
    assert!(
        across.iter().all(|x| e.edges[*x].attrs.curve.is_none()),
        "the line is not part of the circle"
    );
    let curve = e.curves.keys().next().unwrap();
    assert_eq!(
        e.curve_edges(curve).len(),
        26,
        "two circle segments split in two"
    );
    assert_eq!(e.faces.len(), 2);
}
