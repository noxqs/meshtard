//! Importing every sample `.skp` file gives valid topology and complete triangulations.

use glam::DVec2;
use mt_kernel::geom::signed_area2;

/// Whether any two edges of the loops cross (bad input that no triangulation fixes).
/// Such faces, and faces whose loops touch themselves, are reported but not checked.
fn loops_cross(loops: &[&Vec<DVec2>]) -> bool {
    let segs: Vec<(DVec2, DVec2)> = loops
        .iter()
        .flat_map(|l| (0..l.len()).map(move |i| (l[i], l[(i + 1) % l.len()])))
        .collect();
    let orient = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    segs.iter().enumerate().any(|(i, &(a, b))| {
        segs[i + 1..].iter().any(|&(c, d)| {
            let touch = [a, b]
                .iter()
                .any(|p| p.distance(c) < 1e-9 || p.distance(d) < 1e-9);
            !touch
                && orient(a, b, c) * orient(a, b, d) < 0.0
                && orient(c, d, a) * orient(c, d, b) < 0.0
        })
    })
}
use mt_kernel::{Checks, import, tessellate, validate};

fn corpus() -> Vec<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/openskp");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "skp"))
        .collect();
    v.sort();
    v
}

#[test]
fn corpus_imports_with_valid_topology() {
    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let src = mt_skp::read(&path).unwrap();
        let model = import::from_skp(&src);
        let all = std::iter::once((&src.root, &model.root)).chain(
            src.definitions
                .values()
                .zip(model.definitions.values().map(|d| &d.entities)),
        );
        let (mut faces, mut src_faces, mut tris) = (0, 0, 0);
        let (mut off, mut worst, mut crossing) = (0, 0.0f64, 0);
        for (s, ents) in all {
            let errs = validate(ents, Checks::TOPOLOGY);
            assert!(
                errs.is_empty(),
                "{name}: {:#?}",
                &errs[..errs.len().min(10)]
            );
            faces += ents.faces.len();
            src_faces += s.faces.len();
            for f in ents.faces.keys() {
                let t = tessellate::triangulate(ents, f);
                let plane = ents.face_plane(f);
                let (outer, holes) = ents.face_polygons(f, &plane);
                let area = signed_area2(&outer).abs() / 2.0
                    - holes
                        .iter()
                        .map(|h| signed_area2(h).abs() / 2.0)
                        .sum::<f64>();
                // Compare flat areas: SketchUp faces may be off-plane by up to its tolerance.
                let tri_area: f64 = t
                    .iter()
                    .map(|[a, b, c]| {
                        let (a, b, c) = (plane.project(*a), plane.project(*b), plane.project(*c));
                        (b - a).perp_dot(c - a).abs() / 2.0
                    })
                    .sum();
                let rel = (area - tri_area).abs() / area.max(1e-9);
                let loops: Vec<&Vec<DVec2>> = std::iter::once(&outer).chain(&holes).collect();
                let pinched = ents.faces[f].loops().any(|l| {
                    let mut vs = ents.loop_vertices(l);
                    let n = vs.len();
                    vs.sort();
                    vs.dedup();
                    vs.len() != n
                });
                if pinched || loops_cross(&loops) {
                    crossing += 1;
                } else if rel > 1e-6 {
                    off += 1;
                    worst = worst.max(rel);
                }
                tris += t.len();
            }
        }
        assert_eq!(faces, src_faces, "{name}: faces dropped on import");
        eprintln!(
            "{name}: {faces} faces, {tris} triangles, {} definitions, {crossing} with self-touching or crossing loops",
            model.definitions.len()
        );
        assert_eq!(
            off, 0,
            "{name}: {off} well-formed faces mis-triangulated (worst {worst:.2e})"
        );
    }
}
