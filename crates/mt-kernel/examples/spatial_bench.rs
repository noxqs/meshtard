//! Times kernel operations on a large model (the biggest corpus file, its definitions
//! copied side by side into one context until it has ~50k edges).
//!
//! `cargo run --release -p mt-kernel --example spatial_bench`

use std::time::Instant;

use glam::{DMat4, DVec3};
use mt_kernel::{Checks, Entities, FaceId, VertexId, add_line, group, import, validate};

fn model() -> Entities {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/openskp/coedge_orientation_regression.skp");
    let src = mt_skp::read(&path).expect("corpus file");
    let model = import::from_skp(&src);
    let mut defs: Vec<&Entities> = model.definitions.values().map(|d| &d.entities).collect();
    defs.sort_by_key(|e| std::cmp::Reverse(e.edges.len()));
    let mut ents = Entities::new();
    let mut x = 0.0;
    for d in defs {
        if ents.edges.len() >= 50_000 || d.edges.is_empty() {
            break;
        }
        let (lo, hi) = bounds(d);
        let xf = DMat4::from_translation(DVec3::new(x - lo.x, 0.0, 0.0));
        let edges: Vec<_> = d.edges.keys().collect();
        let faces: Vec<_> = d.faces.keys().collect();
        group::copy_raw(&mut ents, d, &edges, &faces, xf);
        x += hi.x - lo.x + 10.0;
    }
    ents
}

fn bounds(e: &Entities) -> (DVec3, DVec3) {
    e.vertices.values().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), v| (lo.min(v.pos), hi.max(v.pos)),
    )
}

struct Rng(u64);

impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

fn faces(e: &Entities) -> Vec<FaceId> {
    e.faces.keys().collect()
}

fn verts(e: &Entities) -> Vec<VertexId> {
    e.vertices.keys().collect()
}

fn time(label: &str, n: usize, f: impl FnOnce()) {
    let t = Instant::now();
    f();
    let dt = t.elapsed().as_secs_f64();
    println!(
        "{label:<34} {n:>5} ops  {:>9.1} ms total  {:>8.3} ms/op",
        dt * 1e3,
        dt * 1e3 / n as f64
    );
}

fn main() {
    let t = Instant::now();
    let mut ents = model();
    println!(
        "model: {} vertices, {} edges, {} faces (built in {:.0} ms)",
        ents.vertices.len(),
        ents.edges.len(),
        ents.faces.len(),
        t.elapsed().as_secs_f64() * 1e3
    );
    let mut rng = Rng(7);

    let vs = verts(&ents);
    let probes: Vec<DVec3> = (0..20_000)
        .map(|_| ents.pos(vs[rng.next(vs.len())]) + DVec3::splat(1e-4))
        .collect();
    let mut e0 = ents.clone();
    time("build_index", 1, || e0.ensure_index());
    time("find_vertex", probes.len(), || {
        let found = probes
            .iter()
            .filter(|p| e0.find_vertex(**p).is_some())
            .count();
        assert_eq!(found, probes.len());
    });

    // Lines across faces (between two vertices of a face's outer loop): split faces.
    let n = 200;
    let fs = faces(&ents);
    let chords: Vec<(DVec3, DVec3)> = (0..n)
        .filter_map(|_| {
            let f = fs[rng.next(fs.len())];
            let pts = ents.loop_points(&ents.faces[f].outer);
            (pts.len() >= 4).then(|| (pts[0], pts[pts.len() / 2]))
        })
        .collect();
    time("add_line (chords of faces)", chords.len(), || {
        for &(a, b) in &chords {
            add_line(&mut ents, a, b);
        }
    });

    // Lines between nearby vertices (new edges, often closing loops).
    let vs = verts(&ents);
    let lines: Vec<(DVec3, DVec3)> = (0..n)
        .map(|_| {
            let i = rng.next(vs.len() - 1);
            (ents.pos(vs[i]), ents.pos(vs[i + 1]))
        })
        .collect();
    time("add_line (vertex to vertex)", lines.len(), || {
        for &(a, b) in &lines {
            add_line(&mut ents, a, b);
        }
    });

    // Free lines in empty space above the model.
    let (lo, hi) = bounds(&ents);
    let free: Vec<(DVec3, DVec3)> = (0..n)
        .map(|i| {
            let z = hi.z + 10.0 + i as f64;
            (DVec3::new(lo.x, lo.y, z), DVec3::new(hi.x, hi.y, z))
        })
        .collect();
    time("add_line (free, above model)", free.len(), || {
        for &(a, b) in &free {
            add_line(&mut ents, a, b);
        }
    });

    let n = 50;
    let fs = faces(&ents);
    let picks: Vec<FaceId> = (0..n).map(|_| fs[rng.next(fs.len())]).collect();
    time("push_pull", n, || {
        for &f in &picks {
            if ents.faces.contains_key(f) {
                mt_kernel::push_pull(&mut ents, f, 0.5, false);
            }
        }
    });

    let fs = faces(&ents);
    let picks: Vec<FaceId> = (0..n).map(|_| fs[rng.next(fs.len())]).collect();
    time("move face (transform_vertices)", n, || {
        for &f in &picks {
            if ents.faces.contains_key(f) {
                let vs = mt_kernel::vertices_of(&ents, &[], &[f]);
                let t = ents.faces[f].normal * 0.25;
                mt_kernel::transform_vertices(&mut ents, &vs, DMat4::from_translation(t));
            }
        }
    });

    let fs = faces(&ents);
    let picks: Vec<FaceId> = (0..n).map(|_| fs[rng.next(fs.len())]).collect();
    time("offset_face", n, || {
        for &f in &picks {
            if ents.faces.contains_key(f) {
                mt_kernel::offset_face(&mut ents, f, 0.1);
            }
        }
    });

    time("clone (undo snapshot)", 1, || {
        std::hint::black_box(ents.clone());
    });
    println!(
        "result: {} vertices, {} edges, {} faces, {} curves",
        ents.vertices.len(),
        ents.edges.len(),
        ents.faces.len(),
        ents.curves.len()
    );
    let errs = validate(&ents, Checks::TOPOLOGY);
    println!("topology errors: {}", errs.len());
}
