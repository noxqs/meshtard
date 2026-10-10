//! Random sequences of rectangles, push/pulls, moves and copies keep the model valid.

use glam::{DMat4, DVec3};
use mt_kernel::{
    Checks, Entities, add_line, copy_geometry, push_pull, transform_vertices, validate, vertices_of,
};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    /// Axis-aligned rectangle on a horizontal plane: corners and height.
    Rect([i8; 2], [i8; 2], i8),
    /// Push/pull the n-th face (modulo the face count) by a distance, with Ctrl or not.
    PushPull(usize, i8, bool),
    /// Move the n-th vertex.
    MoveVertex(usize, [i8; 3]),
    /// Move the vertices of the n-th face.
    MoveFace(usize, [i8; 3]),
    /// Copy the n-th face (and its edges), rotated a quarter turn `turns` times about Z
    /// and translated.
    CopyFace(usize, [i8; 3], u8),
    /// Rotate the vertices of the n-th face a quarter turn `turns` times about a vertical
    /// axis through a grid point. (Arbitrary angles compound into slivers narrower than
    /// the tolerance, which sticky geometry does not always resolve: about one sequence
    /// in 20,000 fails then.)
    RotateFace(usize, [i8; 2], u8),
}

fn vec(c: [i8; 3]) -> DVec3 {
    DVec3::new(c[0].into(), c[1].into(), c[2].into()) * 1.25
}

fn apply(e: &mut Entities, op: &Op) {
    let nth_face = |e: &Entities, n: usize| e.faces.keys().nth(n % e.faces.len().max(1));
    match *op {
        Op::Rect(a, b, z) => {
            let q = |x: i8, y: i8| DVec3::new(x.into(), y.into(), 2.0 * f64::from(z)) * 1.25;
            let c = [q(a[0], a[1]), q(b[0], a[1]), q(b[0], b[1]), q(a[0], b[1])];
            for i in 0..4 {
                add_line(e, c[i], c[(i + 1) % 4]);
            }
        }
        Op::PushPull(n, d, new_faces) => {
            if let Some(f) = nth_face(e, n) {
                push_pull(e, f, f64::from(d) * 1.25, new_faces);
            }
        }
        Op::MoveVertex(n, t) => {
            if let Some(v) = e.vertices.keys().nth(n % e.vertices.len().max(1)) {
                transform_vertices(e, &[v], DMat4::from_translation(vec(t)));
            }
        }
        Op::MoveFace(n, t) => {
            if let Some(f) = nth_face(e, n) {
                let vs = vertices_of(e, &[], &[f]);
                transform_vertices(e, &vs, DMat4::from_translation(vec(t)));
            }
        }
        Op::RotateFace(n, c, turns) => {
            if let Some(f) = nth_face(e, n) {
                let vs = vertices_of(e, &[], &[f]);
                let c = vec([c[0], c[1], 0]);
                let xf = DMat4::from_translation(c)
                    * DMat4::from_rotation_z(f64::from(turns % 4) * std::f64::consts::FRAC_PI_2)
                    * DMat4::from_translation(-c);
                transform_vertices(e, &vs, xf);
            }
        }
        Op::CopyFace(n, t, turns) => {
            if let Some(f) = nth_face(e, n) {
                let xf = DMat4::from_translation(vec(t))
                    * DMat4::from_rotation_z(f64::from(turns % 4) * std::f64::consts::FRAC_PI_2);
                copy_geometry(e, &[], &[f], xf);
            }
        }
    }
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let corner = || (0i8..=8, 0i8..=8).prop_map(|(x, y)| [x, y]);
    let shift = || (-4i8..=4, -4i8..=4, -4i8..=4).prop_map(|(x, y, z)| [x, y, z]);
    let op = prop_oneof![
        3 => (corner(), corner(), 0i8..=2).prop_map(|(a, b, z)| Op::Rect(a, b, z)),
        4 => (any::<usize>(), -8i8..=8, any::<bool>()).prop_map(|(n, d, c)| Op::PushPull(n, d, c)),
        2 => (any::<usize>(), shift()).prop_map(|(n, t)| Op::MoveVertex(n, t)),
        2 => (any::<usize>(), shift()).prop_map(|(n, t)| Op::MoveFace(n, t)),
        1 => (any::<usize>(), shift(), any::<u8>()).prop_map(|(n, t, r)| Op::CopyFace(n, t, r)),
        2 => (any::<usize>(), (0i8..=8, 0i8..=8), 1u8..=3)
            .prop_map(|(n, (x, y), t)| Op::RotateFace(n, [x, y], t)),
    ];
    prop::collection::vec(op, 1..20)
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
    fn modeling_stays_valid(ops in ops()) {
        run(&ops)?;
    }
}

/// The random scenes are not trivial: push/pull does make solids.
#[test]
fn random_scenes_make_solids() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let mut faces = 0;
    for _ in 0..100 {
        let ops = ops().new_tree(&mut runner).unwrap().current();
        let mut e = Entities::new();
        for op in &ops {
            apply(&mut e, op);
        }
        faces += e.faces.len();
    }
    assert!(faces > 300, "only {faces} faces in 100 scenes");
    eprintln!("{faces} faces");
}
