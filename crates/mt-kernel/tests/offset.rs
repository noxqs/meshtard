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

/// An offset that collapses entirely (inward past the inradius) leaves nothing to keep.
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

    // Growing the hole past the shrinking outer loop on every side leaves no region
    // between them: nothing is added. (A hole only partly reaching the outer offset
    // merges with it: `hole_merges_with_outer_offset`.)
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
    // The inner L's arms and corner are all 2 wide: offsetting it by 1.5 collapses the
    // whole of it, so nothing is kept. (A wider part would be kept: see
    // `l_shape_keeps_the_wide_part`.)
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

fn polygon(ents: &mut Entities, pts: &[(f64, f64)]) -> FaceId {
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        add_line(ents, p(a.0, a.1, 0.0), p(b.0, b.1, 0.0));
    }
    check(ents);
    only_face(ents)
}

#[test]
fn l_shape_keeps_the_wide_part() {
    // A 10x6 block with a 2-wide arm on top: offsetting inward by 1.5 makes the arm
    // vanish; the block's offset is kept (SketchUp trims rather than cancels).
    let mut e = Entities::new();
    let f = polygon(
        &mut e,
        &[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 6.0),
            (2.0, 6.0),
            (2.0, 10.0),
            (0.0, 10.0),
        ],
    );
    let new = offset_face(&mut e, f, 1.5);
    check(&e);
    assert_eq!(new.len(), 4);
    assert_eq!(
        areas(&e),
        [21.0, 47.0],
        "inner 7x3 block and the rest of the L"
    );
    for &x in &new {
        for v in [e.edges[x].start, e.edges[x].end] {
            let q = e.pos(v);
            assert!(q.x == 1.5 || q.x == 8.5, "{q}");
            assert!(q.y == 1.5 || q.y == 4.5, "{q}");
        }
    }
}

#[test]
fn u_shape_arms_vanish() {
    let mut e = Entities::new();
    let f = polygon(
        &mut e,
        &[
            (0.0, 0.0),
            (12.0, 0.0),
            (12.0, 12.0),
            (10.0, 12.0),
            (10.0, 6.0),
            (2.0, 6.0),
            (2.0, 12.0),
            (0.0, 12.0),
        ],
    );
    let new = offset_face(&mut e, f, 1.5);
    check(&e);
    assert_eq!(new.len(), 4);
    // The base's offset [1.5, 10.5] x [1.5, 4.5]; the U has area 96.
    assert_eq!(areas(&e), [27.0, 69.0]);
}

#[test]
fn narrow_neck_splits_the_offset_in_two() {
    // Two 6x6 squares joined by a 2-wide bridge: the bridge vanishes and each square
    // keeps its own offset loop.
    let mut e = Entities::new();
    let f = polygon(
        &mut e,
        &[
            (0.0, 0.0),
            (6.0, 0.0),
            (6.0, 2.0),
            (12.0, 2.0),
            (12.0, 0.0),
            (18.0, 0.0),
            (18.0, 6.0),
            (12.0, 6.0),
            (12.0, 4.0),
            (6.0, 4.0),
            (6.0, 6.0),
            (0.0, 6.0),
        ],
    );
    let new = offset_face(&mut e, f, 1.5);
    check(&e);
    assert_eq!(new.len(), 8);
    assert_eq!(areas(&e), [9.0, 9.0, 66.0]);
}

#[test]
fn hole_merges_with_outer_offset() {
    // A hole near the left side: growing it by 1.5 runs into the outer loop's offset,
    // and the two offsets merge into one C-shaped loop.
    let mut e = Entities::new();
    rect(&mut e, 0.0, 0.0, 10.0, 10.0);
    rect(&mut e, 1.0, 4.0, 3.0, 6.0);
    let f = e
        .faces
        .iter()
        .find(|(_, f)| f.holes.len() == 1)
        .map(|(id, _)| id)
        .unwrap();
    let new = offset_face(&mut e, f, 1.5);
    check(&e);
    assert_eq!(new.len(), 8);
    // [1.5, 8.5]^2 minus the grown hole [-0.5, 4.5] x [2.5, 7.5]: 49 - 15. The rest of
    // the face (96 - 34) keeps both the hole and the new loop as holes.
    assert_eq!(areas(&e), [4.0, 34.0, 62.0]);
    let rest = e
        .faces
        .keys()
        .find(|&g| (area(&e, g) - 62.0).abs() < 1e-6)
        .unwrap();
    assert_eq!(e.faces[rest].holes.len(), 2);
}

#[test]
fn outward_offset_closes_a_narrow_slot() {
    // A block with a 2-wide slot cut into its top: offsetting outward by 1.5 fills the
    // slot; the offset is the block's outline grown, with nothing of the slot.
    let mut e = Entities::new();
    let f = polygon(
        &mut e,
        &[
            (0.0, 0.0),
            (12.0, 0.0),
            (12.0, 12.0),
            (7.0, 12.0),
            (7.0, 6.0),
            (5.0, 6.0),
            (5.0, 12.0),
            (0.0, 12.0),
        ],
    );
    let new = offset_face(&mut e, f, -1.5);
    check(&e);
    assert_eq!(new.len(), 4);
    // The grown outline is 15x15; the block has area 132.
    assert_eq!(areas(&e), [93.0, 132.0]);
}

#[test]
fn circle_offset_past_its_radius_adds_nothing() {
    let mut e = Entities::new();
    let c = circle(10.0);
    add_curve(&mut e, &c.points(), Some(c));
    let f = only_face(&e);
    for d in [10.0, 12.0] {
        assert!(offset_face(&mut e, f, d).is_empty(), "offset {d}");
        check(&e);
        assert_eq!((e.edges.len(), e.faces.len()), (24, 1));
    }
}

#[test]
fn rounded_ends_stay_arcs_when_trimmed_elsewhere() {
    // A slot (two half circles of radius 3 joined by straight sides) with a 2-wide tab
    // on one side: offsetting inward by 1.5 removes the tab and keeps both ends as
    // concentric arcs of radius 1.5.
    let mut e = Entities::new();
    let half = |cx: f64, start: f64| mt_kernel::Arc {
        center: p(cx, 0.0, 0.0),
        start_angle: start,
        end_angle: start + std::f64::consts::PI,
        segments: 8,
        ..circle(3.0)
    };
    let right = half(10.0, -std::f64::consts::FRAC_PI_2);
    let left = half(0.0, std::f64::consts::FRAC_PI_2);
    add_curve(&mut e, &right.points(), Some(right));
    add_curve(&mut e, &left.points(), Some(left));
    // Top side with a tab x in [4, 6], y in [3, 6]; bottom side straight.
    let top = [
        (10.0, 3.0),
        (6.0, 3.0),
        (6.0, 6.0),
        (4.0, 6.0),
        (4.0, 3.0),
        (0.0, 3.0),
    ];
    for w in top.windows(2) {
        add_line(&mut e, p(w[0].0, w[0].1, 0.0), p(w[1].0, w[1].1, 0.0));
    }
    add_line(&mut e, p(0.0, -3.0, 0.0), p(10.0, -3.0, 0.0));
    check(&e);
    let f = only_face(&e);
    let new = offset_face(&mut e, f, 1.5);
    check(&e);
    assert_eq!(e.faces.len(), 2);
    // 8 + 8 arc segments and the two straight sides: the tab left no trace.
    assert_eq!(new.len(), 18);
    let mut radii: Vec<f64> = new
        .iter()
        .filter_map(|&x| e.edges[x].attrs.curve)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .map(|c| e.curves[c].arc.expect("arc curve").radius)
        .collect();
    radii.sort_by(f64::total_cmp);
    assert_eq!(radii.len(), 2, "one curve per end");
    assert!(radii.iter().all(|r| (r - 1.5).abs() < 1e-9));
    for &x in &new {
        for v in [e.edges[x].start, e.edges[x].end] {
            assert!(
                e.pos(v).y.abs() <= 1.5 + 1e-9,
                "nothing of the tab: {}",
                e.pos(v)
            );
        }
    }
}

mod random {
    use super::*;
    use proptest::prelude::*;

    /// Overlapping rectangles (making L, U and holed faces), then offsets of faces by
    /// distances that trim them in every way; the model stays valid.
    fn run(rects: &[(i8, i8, i8, i8)], offsets: &[(usize, i8)]) -> Result<(), TestCaseError> {
        let mut e = Entities::new();
        for &(x0, y0, w, h) in rects {
            let (x0, y0) = (f64::from(x0), f64::from(y0));
            rect(&mut e, x0, y0, x0 + f64::from(w), y0 + f64::from(h));
        }
        for &(n, d) in offsets {
            let Some(f) = e.faces.keys().nth(n % e.faces.len().max(1)) else {
                break;
            };
            let d = f64::from(d) * 0.75;
            let new = offset_face(&mut e, f, d);
            let errs = validate(&e, Checks::ALL);
            prop_assert!(errs.is_empty(), "offset {d} of {f:?}: {errs:#?}");
            for x in new {
                prop_assert!(e.edges.contains_key(x));
            }
        }
        Ok(())
    }

    proptest! {
        #[test]
        fn offsets_stay_valid(
            rects in prop::collection::vec((0i8..12, 0i8..12, 1i8..8, 1i8..8), 1..5),
            offsets in prop::collection::vec((any::<usize>(), -4i8..=4), 1..4),
        ) {
            run(&rects, &offsets)?;
        }
    }
}
