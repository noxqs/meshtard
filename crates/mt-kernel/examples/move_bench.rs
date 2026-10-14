//! Times generating a big flat grid (slot map inserts and writes) and moving a tenth of
//! it (`transform_vertices` on a huge single context), as the app benchmark's "grid 1M
//! edges" model does, and prints a checksum of the result.
//!
//! `cargo run --release -p mt-kernel --example move_bench [-- N]` (N: grid side, 577 by
//! default: a million edges).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

use glam::{DMat4, DVec3};
use mt_kernel::{CoEdge, Entities, FaceAttrs, FaceId, Loop, VertexId, transform};

/// A face from vertex ids (edges reused when present), normal by Newell's method.
fn polygon(ents: &mut Entities, vs: &[VertexId]) -> FaceId {
    let mut coedges = Vec::new();
    let mut normal = DVec3::ZERO;
    for k in 0..vs.len() {
        let (a, b) = (vs[k], vs[(k + 1) % vs.len()]);
        let (pa, pb) = (ents.pos(a), ents.pos(b));
        normal += DVec3::new(
            (pa.y - pb.y) * (pa.z + pb.z),
            (pa.z - pb.z) * (pa.x + pb.x),
            (pa.x - pb.x) * (pa.y + pb.y),
        );
        let (edge, reversed) = match ents.edge_between(a, b) {
            Some(e) => (e, ents.edges[e].start != a),
            None => (ents.add_raw_edge(a, b, Default::default()), false),
        };
        coedges.push(CoEdge { edge, reversed });
    }
    ents.add_raw_face(
        Loop { coedges },
        Vec::new(),
        normal.normalize(),
        FaceAttrs::default(),
    )
}

/// The app benchmark's grid: `n` x `n` waved quads, two triangles each.
fn grid(n: usize) -> Entities {
    let mut ents = Entities::default();
    let mut v = Vec::with_capacity((n + 1) * (n + 1));
    for j in 0..=n {
        for i in 0..=n {
            let z = ((i as f64) * 0.3).sin() * ((j as f64) * 0.2).cos() * 2.0;
            v.push(ents.add_vertex(DVec3::new(i as f64 * 10.0, j as f64 * 10.0, z)));
        }
    }
    let at = |i: usize, j: usize| v[j * (n + 1) + i];
    for j in 0..n {
        for i in 0..n {
            polygon(&mut ents, &[at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            polygon(&mut ents, &[at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    ents
}

fn checksum(ents: &Entities) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{:?}{:?}{:?}", ents.vertices, ents.edges, ents.faces).hash(&mut h);
    h.finish()
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(577);
    let mut best = f64::MAX;
    let mut ents = Entities::default();
    for _ in 0..3 {
        let t = Instant::now();
        ents = grid(n);
        best = best.min(t.elapsed().as_secs_f64());
    }
    println!(
        "grid {n}: {} edges, {} faces, generate {:.1} ms (best of 3)",
        ents.edges.len(),
        ents.faces.len(),
        best * 1e3
    );
    // A tenth of everything, faces first (as the app's Select All lists them).
    let faces: Vec<FaceId> = ents.faces.keys().collect();
    let count = (ents.faces.len() + ents.edges.len()).div_ceil(10);
    let faces = &faces[..count.min(faces.len())];
    let verts = transform::vertices_of(&ents, &[], faces);
    // MOVE_BENCH_WELD: a move by one row, so the moved edge row lands on the next
    // (welding, through the rebuild path).
    let by = if std::env::var_os("MOVE_BENCH_WELD").is_some() {
        DVec3::new(0.0, 10.0, 0.0)
    } else {
        DVec3::new(5.0, 3.0, 1.0)
    };
    let xf = DMat4::from_translation(by);
    let t = Instant::now();
    ents.ensure_index();
    let index = t.elapsed();
    let t = Instant::now();
    transform::transform_vertices(&mut ents, &verts, xf);
    println!(
        "move {} vertices: index {:.0} ms, transform_vertices {:.0} ms",
        verts.len(),
        index.as_secs_f64() * 1e3,
        t.elapsed().as_secs_f64() * 1e3
    );
    println!(
        "after: {} edges, {} faces, checksum {:016x}",
        ents.edges.len(),
        ents.faces.len(),
        checksum(&ents)
    );
    if std::env::var_os("MOVE_BENCH_VALIDATE").is_some() {
        let errors = mt_kernel::validate(&ents, mt_kernel::Checks::TOPOLOGY);
        println!("validate: {} problems", errors.len());
    }
}
