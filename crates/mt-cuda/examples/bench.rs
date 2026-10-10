//! CPU vs GPU BVH build and ray batch on the largest sample file.
//!
//! cargo run --release -p mt-cuda --example bench [--features cuda] [file.skp]
//! cargo run --release -p mt-cuda --example bench [--features cuda] -- --sweep
//!
//! `--sweep` times the kernel's `Bvh::build` against `Accel::build_bvh` on synthetic
//! scenes of growing size, to choose the triangle count above which the app builds its
//! pick index on the GPU.

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

/// Median wall time in ms of `runs` calls; `setup` runs outside the timed part.
fn median_ms<S, T>(runs: usize, mut setup: impl FnMut() -> S, mut f: impl FnMut(S) -> T) -> f64 {
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let input = setup();
            let start = Instant::now();
            std::hint::black_box(f(input));
            start.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[runs / 2]
}

/// Boxes (12 triangles) and 32-sided cylinders (124 triangles) of random size scattered
/// over a square grid, cut to exactly `n` triangles.
fn synthetic_scene(n: usize) -> Vec<[DVec3; 3]> {
    let mut s = n as u64 + 17;
    let mut rnd = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let quad = |out: &mut Vec<[DVec3; 3]>, a, b, c, d| {
        out.push([a, b, c]);
        out.push([a, c, d]);
    };
    // About 40 triangles per object on average; spacing 100 inches.
    let side = ((n / 40).max(1) as f64).sqrt().ceil() as usize;
    let mut out = Vec::with_capacity(n + 124);
    let mut k = 0;
    while out.len() < n {
        let base = DVec3::new((k % side) as f64, (k / side) as f64, 0.0) * 100.0
            + DVec3::new(rnd(), rnd(), 0.0) * 30.0;
        let size = DVec3::new(
            10.0 + rnd() * 50.0,
            10.0 + rnd() * 50.0,
            10.0 + rnd() * 100.0,
        );
        k += 1;
        if rnd() < 0.75 {
            let p = |x: f64, y: f64, z: f64| base + size * DVec3::new(x, y, z);
            let c: Vec<DVec3> = (0..8)
                .map(|i| p(f64::from(i & 1), f64::from(i >> 1 & 1), f64::from(i >> 2)))
                .collect();
            for [a, b, cc, d] in [
                [0, 2, 3, 1],
                [4, 5, 7, 6],
                [0, 1, 5, 4],
                [2, 6, 7, 3],
                [0, 4, 6, 2],
                [1, 3, 7, 5],
            ] {
                quad(&mut out, c[a], c[b], c[cc], c[d]);
            }
        } else {
            let r = size.x / 2.0;
            let center = base + DVec3::new(r, r, 0.0);
            let rim = |i: usize, z: f64| {
                let a = i as f64 / 32.0 * std::f64::consts::TAU;
                center + DVec3::new(r * a.cos(), r * a.sin(), z)
            };
            for i in 0..32 {
                let j = (i + 1) % 32;
                quad(
                    &mut out,
                    rim(i, 0.0),
                    rim(j, 0.0),
                    rim(j, size.z),
                    rim(i, size.z),
                );
            }
            for i in 1..31 {
                out.push([rim(0, 0.0), rim(i + 1, 0.0), rim(i, 0.0)]);
                out.push([rim(0, size.z), rim(i, size.z), rim(i + 1, size.z)]);
            }
        }
    }
    out.truncate(n);
    out
}

fn sweep() {
    let accel = Accel::new();
    if !accel.is_gpu() {
        println!("No GPU (build with --features cuda and a CUDA device); nothing to compare.");
        return;
    }
    // Warm up the context and kernels.
    accel.build_bvh(synthetic_scene(1000));
    println!("| triangles | CPU Bvh::build (ms) | GPU Accel::build_bvh (ms) | CPU / GPU |");
    println!("|---:|---:|---:|---:|");
    for n in [
        1_000, 5_000, 10_000, 20_000, 50_000, 100_000, 500_000, 2_000_000,
    ] {
        let tris = synthetic_scene(n);
        let cpu = median_ms(5, || tris.clone(), Bvh::build);
        let gpu = median_ms(5, || tris.clone(), |t| accel.build_bvh(t));
        println!("| {n} | {cpu:.2} | {gpu:.2} | {:.2} |", cpu / gpu);
    }
}

fn main() {
    if std::env::args().any(|a| a == "--sweep") {
        return sweep();
    }
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
