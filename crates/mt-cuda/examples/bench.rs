//! CPU vs GPU BVH build and ray batch on the largest sample file.
//!
//! cargo run --release -p mt-cuda --example bench [--features cuda] [file.skp]

use std::time::Instant;

use glam::DVec3;
use mt_cuda::{Accel, Lbvh, scene};
use mt_kernel::bvh::Bvh;

fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut out = None;
    for _ in 0..runs {
        let start = Instant::now();
        let r = f();
        best = best.min(start.elapsed().as_secs_f64() * 1e3);
        out = Some(r);
    }
    (best, out.unwrap())
}

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/openskp");
    let path = std::env::args().nth(1).map_or_else(
        || {
            // Largest file by size.
            std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|x| x == "skp"))
                .max_by_key(|p| p.metadata().unwrap().len())
                .unwrap()
        },
        Into::into,
    );
    let tris = scene::read_triangles(&path).unwrap();
    println!("{}: {} triangles", path.display(), tris.len());

    let (ms, _) = best_of(3, || Bvh::build(tris.clone()));
    println!("CPU median-split Bvh build (mt-kernel): {ms:8.2} ms");
    let (ms, cpu) = best_of(3, || Lbvh::build(tris.clone()));
    println!("CPU LBVH build:                         {ms:8.2} ms");

    let start = Instant::now();
    let accel = Accel::new();
    let init = start.elapsed().as_secs_f64() * 1e3;
    if !accel.is_gpu() {
        println!("No GPU (build with --features cuda and a CUDA device); CPU only.");
        return;
    }
    println!("GPU init (context + NVRTC compile):     {init:8.2} ms");
    let (ms, gpu) = best_of(5, || accel.build_bvh(tris.clone()));
    assert_eq!(gpu, cpu, "GPU and CPU trees differ");
    println!("GPU LBVH build (incl. transfers):       {ms:8.2} ms  (same tree as CPU)");

    let (lo, hi) = cpu.bounds().unwrap();
    let mut s = 3u64;
    let mut rnd = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let size = hi - lo;
    let rays: Vec<_> = (0..100_000)
        .map(|_| {
            let o = lo - size * 0.5 + DVec3::new(rnd(), rnd(), rnd()) * size * 2.0;
            (o, lo + DVec3::new(rnd(), rnd(), rnd()) * size - o)
        })
        .collect();
    let (ms, cpu_hits) = best_of(3, || cpu.rays(&rays));
    println!(
        "CPU 100k rays (1 thread):               {ms:8.2} ms  ({} hits)",
        cpu_hits.iter().flatten().count()
    );
    let (ms, gpu_hits) = best_of(3, || accel.rays(&cpu, &rays));
    assert_eq!(gpu_hits, cpu_hits, "GPU and CPU hits differ");
    println!("GPU 100k rays (incl. transfers):        {ms:8.2} ms  (same hits as CPU)");
}
