//! Solid Tools: the solid test and booleans on boxes, including coincident faces and
//! rotated operands. Every result must validate and be a solid (or empty).

use glam::{DMat4, DQuat, DVec3};
use mt_kernel::solid::{self, SolidInput, volume};
use mt_kernel::{Checks, Entities, add_line, is_solid, validate};
use proptest::prelude::*;

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// An axis-aligned box drawn with lines (faces from auto-face, normals as SketchUp
/// would make them: the solid code orients them).
fn cube(lo: DVec3, hi: DVec3) -> Entities {
    let mut e = Entities::new();
    for z in [lo.z, hi.z] {
        let pts = [
            p(lo.x, lo.y, z),
            p(hi.x, lo.y, z),
            p(hi.x, hi.y, z),
            p(lo.x, hi.y, z),
        ];
        for i in 0..4 {
            add_line(&mut e, pts[i], pts[(i + 1) % 4]);
        }
    }
    for (x, y) in [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)] {
        add_line(&mut e, p(x, y, lo.z), p(x, y, hi.z));
    }
    assert!(is_solid(&e), "test cube is not solid");
    e
}

fn unit() -> Entities {
    cube(DVec3::ZERO, DVec3::ONE)
}

fn at(e: &Entities, offset: DVec3) -> SolidInput<'_> {
    SolidInput::new(e, DMat4::from_translation(offset))
}

fn check(e: &Entities) {
    let errs = validate(e, Checks::ALL);
    assert!(errs.is_empty(), "invalid: {errs:#?}");
    assert!(e.faces.is_empty() || is_solid(e), "result is not a solid");
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn solid_test() {
    let c = unit();
    assert!(is_solid(&c));
    close(volume(&c), 1.0);
    let mut open = Entities::new();
    for (a, b) in [
        (p(0., 0., 0.), p(1., 0., 0.)),
        (p(1., 0., 0.), p(1., 1., 0.)),
    ] {
        add_line(&mut open, a, b);
    }
    assert!(!is_solid(&open));
    assert!(!is_solid(&Entities::new()));
    // A cube with a stray edge is not a solid.
    let mut stray = unit();
    add_line(&mut stray, p(1., 1., 1.), p(2., 2., 2.));
    assert!(!is_solid(&stray));
    // Not a solid: the boolean says which input.
    let r = solid::union(SolidInput::new(&c, DMat4::IDENTITY), at(&open, DVec3::ZERO));
    assert_eq!(r.unwrap_err(), solid::SolidError::NotSolid(1));
}

#[test]
fn union_of_overlapping_cubes() {
    let c = unit();
    let r = solid::union(at(&c, DVec3::ZERO), at(&c, DVec3::splat(0.5))).unwrap();
    check(&r);
    close(volume(&r), 2.0 - 0.125);
    // Three full faces and three L-shaped faces per cube.
    assert_eq!(r.faces.len(), 12);
}

#[test]
fn intersect_subtract_and_split_of_overlapping_cubes() {
    let c = unit();
    let (a, b) = (at(&c, DVec3::ZERO), at(&c, DVec3::splat(0.5)));
    let i = solid::intersect(a, b).unwrap();
    check(&i);
    close(volume(&i), 0.125);
    assert_eq!(i.faces.len(), 6);

    // Subtract removes the first from the second.
    let s = solid::subtract(a, b).unwrap();
    check(&s);
    close(volume(&s), 1.0 - 0.125);
    assert!(
        s.vertices
            .values()
            .all(|v| v.pos.cmpge(DVec3::splat(0.5 - 1e-9)).all())
    );
    assert_eq!(s.faces.len(), 9);
    let t = solid::trim(a, b).unwrap();
    close(volume(&t), 1.0 - 0.125);

    let sp = solid::split(a, b).unwrap();
    for piece in [&sp.first_only, &sp.both, &sp.second_only] {
        check(piece);
    }
    close(volume(&sp.first_only), 0.875);
    close(volume(&sp.both), 0.125);
    close(volume(&sp.second_only), 0.875);
    assert!(
        sp.first_only
            .vertices
            .values()
            .all(|v| v.pos.cmple(DVec3::ONE + 1e-9).all())
    );
}

#[test]
fn cubes_sharing_a_face() {
    let c = unit();
    let (a, b) = (at(&c, DVec3::ZERO), at(&c, DVec3::X));
    let u = solid::union(a, b).unwrap();
    check(&u);
    close(volume(&u), 2.0);
    assert_eq!(u.faces.len(), 6, "coplanar faces merge into one box");

    let i = solid::intersect(a, b).unwrap();
    assert!(i.faces.is_empty());
    let s = solid::subtract(a, b).unwrap();
    check(&s);
    close(volume(&s), 1.0);
    assert_eq!(s.faces.len(), 6);
}

#[test]
fn cubes_offset_by_half_along_an_axis() {
    // Four pairs of coplanar faces facing the same way.
    let c = unit();
    let (a, b) = (at(&c, DVec3::ZERO), at(&c, DVec3::X * 0.5));
    let u = solid::union(a, b).unwrap();
    check(&u);
    close(volume(&u), 1.5);
    assert_eq!(u.faces.len(), 6);
    let i = solid::intersect(a, b).unwrap();
    check(&i);
    close(volume(&i), 0.5);
    assert_eq!(i.faces.len(), 6);
    let s = solid::subtract(a, b).unwrap();
    check(&s);
    close(volume(&s), 0.5);
    assert_eq!(s.faces.len(), 6);
    let sp = solid::split(a, b).unwrap();
    let total: f64 = [&sp.first_only, &sp.both, &sp.second_only]
        .iter()
        .map(|e| {
            check(e);
            volume(e)
        })
        .sum();
    close(total, 1.5);
}

#[test]
fn identical_cubes() {
    let c = unit();
    let (a, b) = (at(&c, DVec3::ZERO), at(&c, DVec3::ZERO));
    let u = solid::union(a, b).unwrap();
    check(&u);
    close(volume(&u), 1.0);
    assert_eq!(u.faces.len(), 6);
    let i = solid::intersect(a, b).unwrap();
    check(&i);
    close(volume(&i), 1.0);
    assert!(solid::subtract(a, b).unwrap().faces.is_empty());
}

#[test]
fn cube_through_cube_makes_a_tunnel() {
    let big = cube(DVec3::ZERO, DVec3::splat(3.0));
    let bar = cube(p(1., 1., -1.), p(2., 2., 4.));
    let id = DMat4::IDENTITY;
    let s = solid::subtract(SolidInput::new(&bar, id), SolidInput::new(&big, id)).unwrap();
    check(&s);
    close(volume(&s), 27.0 - 3.0);
    // Top and bottom with a hole each, four outer sides, four tunnel walls.
    assert_eq!(s.faces.len(), 10);
    assert_eq!(s.faces.values().filter(|f| f.holes.len() == 1).count(), 2);
    let u = solid::union(SolidInput::new(&bar, id), SolidInput::new(&big, id)).unwrap();
    check(&u);
    close(volume(&u), 27.0 + 2.0);
}

#[test]
fn rotated_cube() {
    let c = unit();
    let centered = DMat4::from_translation(DVec3::splat(-0.5));
    let rot = DMat4::from_quat(DQuat::from_euler(glam::EulerRot::XYZ, 0.3, 0.5, 0.7));
    let a = SolidInput::new(&c, centered);
    let b = SolidInput::new(
        &c,
        DMat4::from_translation(DVec3::splat(0.4)) * rot * centered,
    );
    let sp = solid::split(a, b).unwrap();
    let (x, y, z) = (
        volume(&sp.first_only),
        volume(&sp.both),
        volume(&sp.second_only),
    );
    for piece in [&sp.first_only, &sp.both, &sp.second_only] {
        check(piece);
    }
    assert!(y > 0.01 && x > 0.01 && z > 0.01);
    close(x + y, 1.0);
    close(z + y, 1.0);
    let u = solid::union(a, b).unwrap();
    check(&u);
    close(volume(&u), x + y + z);
}

#[test]
fn mirrored_and_painted_operand() {
    let c = unit();
    let mut a = SolidInput::new(&c, DMat4::from_scale(p(-1.0, 1.0, 1.0)));
    a.material = Some(3);
    let b = at(&c, p(-0.5, 0.5, 0.0));
    let u = solid::union(a, b).unwrap();
    check(&u);
    close(volume(&u), 1.75);
    assert!(u.faces.values().any(|f| f.attrs.material == Some(3)));
    assert!(u.faces.values().any(|f| f.attrs.material.is_none()));
}

#[test]
fn outer_shell_of_three_cubes() {
    let c = unit();
    let r = solid::outer_shell(&[
        at(&c, DVec3::ZERO),
        at(&c, DVec3::splat(0.5)),
        at(&c, p(1.0, 0.0, 0.0)),
    ])
    .unwrap();
    check(&r);
    // Cubes at 0 and x=1 share a face: a 2x1x1 box; the third adds 1 - overlap, where
    // its overlap with the box is [0.5,1.5]x[0.5,1]x[0.5,1] = 0.25.
    close(volume(&r), 2.0 + 1.0 - 0.25);
}

#[test]
fn outer_shell_drops_voids() {
    // A hollow box (inner shell facing in) and a cube outside it.
    let mut hollow = cube(DVec3::ZERO, DVec3::splat(4.0));
    let inner = cube(DVec3::ONE, DVec3::splat(3.0));
    mt_kernel::group::copy_raw(
        &mut hollow,
        &inner,
        &[],
        &inner.faces.keys().collect::<Vec<_>>(),
        DMat4::IDENTITY,
    );
    assert!(is_solid(&hollow));
    let c = unit();
    let id = DMat4::IDENTITY;
    let u = solid::union(SolidInput::new(&hollow, id), at(&c, p(4.0, 0.0, 0.0))).unwrap();
    check(&u);
    close(volume(&u), 64.0 - 8.0 + 1.0);
    let o = solid::outer_shell(&[SolidInput::new(&hollow, id), at(&c, p(4.0, 0.0, 0.0))]).unwrap();
    check(&o);
    close(volume(&o), 65.0);
}

/// Exact overlap volume of two axis-aligned boxes.
fn overlap(a: (DVec3, DVec3), b: (DVec3, DVec3)) -> f64 {
    let lo = a.0.max(b.0);
    let hi = a.1.min(b.1);
    let d = (hi - lo).max(DVec3::ZERO);
    d.x * d.y * d.z
}

fn corner() -> impl Strategy<Value = DVec3> {
    // Half-unit steps: many coincident faces and edges.
    (0i8..4, 0i8..4, 0i8..4).prop_map(|(x, y, z)| p(x.into(), y.into(), z.into()) * 0.5)
}

fn size() -> impl Strategy<Value = DVec3> {
    (1i8..4, 1i8..4, 1i8..4).prop_map(|(x, y, z)| p(x.into(), y.into(), z.into()) * 0.5)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn random_box_pairs(a0 in corner(), sa in size(), b0 in corner(), sb in size()) {
        let (ba, bb) = ((a0, a0 + sa), (b0, b0 + sb));
        let (ca, cb) = (cube(ba.0, ba.1), cube(bb.0, bb.1));
        let id = DMat4::IDENTITY;
        let (a, b) = (SolidInput::new(&ca, id), SolidInput::new(&cb, id));
        let (va, vb, vi) = (sa.x * sa.y * sa.z, sb.x * sb.y * sb.z, overlap(ba, bb));
        prop_assert!((volume(&ca) - va).abs() < 1e-9);
        // Boxes touching along an edge or at a corner give non-manifold unions.
        let gap = ba.1.min(bb.1) - ba.0.max(bb.0);
        let touching_only = gap.min_element() >= 0.0
            && [gap.x, gap.y, gap.z].iter().filter(|g| **g == 0.0).count() >= 2;
        if !touching_only {
            let u = solid::union(a, b).unwrap();
            check(&u);
            prop_assert!((volume(&u) - (va + vb - vi)).abs() < 1e-6);
        }
        let sp = solid::split(a, b);
        if let Ok(sp) = sp {
            for e in [&sp.first_only, &sp.both, &sp.second_only] {
                check(e);
            }
            prop_assert!((volume(&sp.both) - vi).abs() < 1e-6);
            prop_assert!((volume(&sp.first_only) - (va - vi)).abs() < 1e-6);
            prop_assert!((volume(&sp.second_only) - (vb - vi)).abs() < 1e-6);
        } else {
            prop_assert!(vi == 0.0, "split failed with overlap {}", vi);
        }
    }
}

/// A unit cube against randomly rotated and scaled boxes (seeded, so failures
/// reproduce). Every result returned must be valid with the right volume; inputs with
/// features within about TOL of each other may fail with `SolidError::Failed`, which
/// must stay rare.
#[test]
fn random_rotated_pairs() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = |lo: f64, hi: f64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        lo + (hi - lo) * ((seed >> 11) as f64 / (1u64 << 53) as f64)
    };
    let c = unit();
    let a = SolidInput::new(&c, DMat4::IDENTITY);
    let (cases, mut failed) = (250, 0);
    for _ in 0..cases {
        let rot = DQuat::from_euler(
            glam::EulerRot::XYZ,
            rnd(0.0, 6.3),
            rnd(0.0, 6.3),
            rnd(0.0, 6.3),
        );
        let offset = p(rnd(-0.8, 0.8), rnd(-0.8, 0.8), rnd(-0.8, 0.8));
        let scale = p(rnd(0.3, 1.5), rnd(0.3, 1.5), rnd(0.3, 1.5));
        let xf = DMat4::from_translation(DVec3::splat(0.5) + offset)
            * DMat4::from_quat(rot)
            * DMat4::from_scale(scale)
            * DMat4::from_translation(DVec3::splat(-0.5));
        let b = SolidInput::new(&c, xf);
        let vb = scale.x * scale.y * scale.z;
        let (Ok(sp), Ok(u)) = (solid::split(a, b), solid::union(a, b)) else {
            failed += 1;
            continue;
        };
        for e in [&sp.first_only, &sp.both, &sp.second_only, &u] {
            check(e);
        }
        let (x, y, z) = (
            volume(&sp.first_only),
            volume(&sp.both),
            volume(&sp.second_only),
        );
        // Intersection points within TOL of a vertex snap to it, moving the cut
        // surface by up to TOL: allow TOL times the surface area.
        let slack = 10.0 * mt_kernel::TOL;
        assert!((x + y - 1.0).abs() < slack, "{x} + {y} != 1");
        assert!((z + y - vb).abs() < slack, "{z} + {y} != {vb}");
        assert!((volume(&u) - (x + y + z)).abs() < slack);
    }
    assert!(failed * 50 <= cases, "{failed} of {cases} failed");
}
