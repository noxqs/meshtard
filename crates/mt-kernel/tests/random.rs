//! Random sequences of lines and erasures keep the model valid.

use glam::DVec3;
use mt_kernel::{Checks, Entities, add_line, erase_edge, validate};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Line([i8; 3], [i8; 3]),
    /// Axis-aligned rectangle on a horizontal plane: corners and height.
    Rect([i8; 2], [i8; 2], i8),
    /// Erase the n-th edge (modulo the edge count).
    Erase(usize),
}

fn point(c: [i8; 3]) -> DVec3 {
    DVec3::new(c[0].into(), c[1].into(), c[2].into()) * 2.5
}

fn apply(e: &mut Entities, op: &Op) {
    match *op {
        Op::Line(a, b) => {
            add_line(e, point(a), point(b));
        }
        Op::Rect(a, b, z) => {
            let q = |x: i8, y: i8| DVec3::new(x.into(), y.into(), 2.0 * f64::from(z)) * 1.25;
            let c = [q(a[0], a[1]), q(b[0], a[1]), q(b[0], b[1]), q(a[0], b[1])];
            for i in 0..4 {
                add_line(e, c[i], c[(i + 1) % 4]);
            }
        }
        Op::Erase(n) => {
            if let Some(id) = e.edges.keys().nth(n % e.edges.len().max(1)) {
                erase_edge(e, id);
            }
        }
    }
}

fn ops(z_range: std::ops::RangeInclusive<i8>) -> impl Strategy<Value = Vec<Op>> {
    let z = z_range.clone();
    let coord = move || (0i8..=4, 0i8..=4, z_range.clone()).prop_map(|(x, y, z)| [x, y, z]);
    let corner = || (0i8..=8, 0i8..=8).prop_map(|(x, y)| [x, y]);
    let op = prop_oneof![
        4 => (coord(), coord()).prop_map(|(a, b)| Op::Line(a, b)),
        2 => (corner(), corner(), z).prop_map(|(a, b, z)| Op::Rect(a, b, z)),
        1 => any::<usize>().prop_map(Op::Erase),
    ];
    prop::collection::vec(op, 1..25)
}

fn run(ops: &[Op]) -> Result<(), TestCaseError> {
    let mut e = Entities::new();
    for (i, op) in ops.iter().enumerate() {
        apply(&mut e, op);
        let errs = validate(&e, Checks::ALL);
        prop_assert!(errs.is_empty(), "after step {i} ({op:?}): {errs:#?}");
    }
    Ok(())
}

proptest! {
    // 256 cases by default; PROPTEST_CASES=5000 for a longer run.
    #[test]
    fn flat_drawings_stay_valid(ops in ops(0..=0)) {
        run(&ops)?;
    }

    #[test]
    fn solid_drawings_stay_valid(ops in ops(0..=2)) {
        run(&ops)?;
    }
}

/// The random scenes are not trivial: they do create faces and holes.
#[test]
fn random_scenes_create_faces() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut faces, mut holes) = (0, 0);
    for _ in 0..200 {
        let ops = ops(0..=0).new_tree(&mut runner).unwrap().current();
        let mut e = Entities::new();
        for op in &ops {
            apply(&mut e, op);
        }
        faces += e.faces.len();
        holes += e.faces.values().map(|f| f.holes.len()).sum::<usize>();
    }
    assert!(faces > 200, "only {faces} faces in 200 scenes");
    eprintln!("{faces} faces, {holes} holes");
}

proptest! {
    /// Only rectangles: nested ones make faces with holes.
    #[test]
    fn nested_rectangles_stay_valid(rects in prop::collection::vec(((0i8..=8, 0i8..=8), (0i8..=8, 0i8..=8), 0i8..=1), 1..12)) {
        let ops: Vec<Op> = rects
            .into_iter()
            .map(|((x0, y0), (x1, y1), z)| Op::Rect([x0, y0], [x1, y1], z))
            .chain([Op::Erase(3), Op::Erase(7)])
            .collect();
        run(&ops)?;
    }
}

#[test]
fn nested_rectangles_make_holes() {
    let mut e = Entities::new();
    for (a, b) in [
        ([0, 0], [8, 8]),
        ([2, 2], [6, 6]),
        ([3, 3], [4, 4]),
        ([7, 1], [7, 7]),
    ] {
        apply(&mut e, &Op::Rect(a, b, 0));
    }
    assert!(validate(&e, Checks::ALL).is_empty());
    assert_eq!(e.faces.values().map(|f| f.holes.len()).sum::<usize>(), 2);
}

/// Two faces joined by a bridge edge, then a rectangle around them: the hole loop runs
/// along the bridge twice, and ear clipping of such a loop can put a triangle over a
/// block. The sample point of the big face must still be inside it (it was not, for
/// some loop start vertices: the big face then counted as overlapping a block, and
/// drawing it rebuilt the block's face).
#[test]
fn bridged_blocks_inside_a_rectangle() {
    let mut e = Entities::new();
    for (a, b) in [
        ([4, 5], [7, 3]),
        ([1, 6], [3, 3]),
        ([4, 3], [1, 3]),
        ([0, 7], [8, 0]),
    ] {
        apply(&mut e, &Op::Rect(a, b, 0));
        let errs = validate(&e, Checks::ALL);
        assert!(errs.is_empty(), "{errs:#?}");
    }
    assert_eq!(e.faces.len(), 3);
    assert_eq!(e.faces.values().map(|f| f.holes.len()).sum::<usize>(), 1);
}
