//! CPU vs GPU (and LBVH vs the kernel's BVH) on the sample files in tests/corpus/openskp.
//!
//! GPU tests run only with `--features cuda`; without a CUDA device they print a note and
//! pass.

use glam::DVec3;
use mt_cuda::{Lbvh, Ray, scene};
use mt_kernel::bvh::{Bvh, ray_triangle};

fn corpus(name: &str) -> Vec<[DVec3; 3]> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/openskp")
        .join(name);
    scene::read_triangles(path).unwrap()
}

/// Rays from around the model toward random points inside it, so most of them hit.
fn random_rays(lo: DVec3, hi: DVec3, n: usize, seed: u64) -> Vec<Ray> {
    let mut s = seed;
    let mut rnd = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let size = hi - lo;
    let mut point = |margin: f64| {
        lo - size * margin + DVec3::new(rnd(), rnd(), rnd()) * size * (1.0 + 2.0 * margin)
    };
    (0..n)
        .map(|_| {
            let o = point(0.5);
            (o, point(0.0) - o)
        })
        .collect()
}

#[test]
fn lbvh_answers_like_the_kernel_bvh() {
    let tris = corpus("coedge_orientation_regression.skp");
    assert!(tris.len() > 1000, "{}", tris.len());
    let bvh = Bvh::build(tris.clone());
    let lbvh = Lbvh::build(tris.clone());
    assert_eq!(lbvh.len(), bvh.len());
    let (lo, hi) = lbvh.bounds().unwrap();
    assert_eq!(Some((lo, hi)), bvh.bounds());
    let mut hits = 0;
    for (o, d) in random_rays(lo, hi, 5000, 1) {
        let (a, b) = (bvh.ray(o, d), lbvh.ray(o, d));
        match (a, b) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                hits += 1;
                assert_eq!(a.t, b.t, "ray {o} {d}");
                // Equal distances may pick different triangles; both must be hit there.
                if a.triangle != b.triangle {
                    assert_eq!(ray_triangle(o, d, tris[b.triangle]), Some(b.t));
                    assert!(b.triangle < a.triangle);
                }
            }
            _ => panic!("ray {o} {d}: {a:?} vs {b:?}"),
        }
    }
    assert!(hits > 2500, "only {hits} hits");
}

#[cfg(feature = "cuda")]
mod gpu {
    use super::*;
    use mt_cuda::{Accel, batch, gpu::Gpu};
    use mt_kernel::geom::Plane;

    fn device() -> Option<Gpu> {
        match Gpu::new() {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("no CUDA device, GPU test skipped: {e}");
                None
            }
        }
    }

    const FILES: [&str; 5] = [
        "coedge_orientation_regression.skp",
        "capilla_quiroz_v17.skp",
        "gondola_v20.skp",
        "chair_and_table.skp",
        "create_holes.skp",
    ];

    #[test]
    fn gpu_build_matches_cpu_build() {
        let Some(gpu) = device() else { return };
        for name in FILES {
            let tris = corpus(name);
            let cpu = Lbvh::build(tris.clone());
            let (nodes, root) = gpu.build_bvh(&tris).unwrap();
            assert_eq!(root, cpu.root(), "{name}");
            assert_eq!(nodes.len(), cpu.nodes().len(), "{name}");
            // Bit-identical nodes, bounds included.
            if let Some(i) = (0..nodes.len()).find(|&i| nodes[i] != cpu.nodes()[i]) {
                panic!("{name}: node {i}: {:?} vs {:?}", nodes[i], cpu.nodes()[i]);
            }
        }
    }

    #[test]
    fn gpu_build_handles_small_and_degenerate_inputs() {
        let Some(gpu) = device() else { return };
        let t = [DVec3::ZERO, DVec3::X, DVec3::Y];
        for tris in [
            vec![],
            vec![t],
            vec![t; 2],
            vec![t; 1000],
            (0..777)
                .map(|i| t.map(|p| p + DVec3::X * f64::from(i)))
                .collect(),
        ] {
            let cpu = Lbvh::build(tris.clone());
            let (nodes, root) = gpu.build_bvh(&tris).unwrap();
            assert_eq!((nodes.as_slice(), root), (cpu.nodes(), cpu.root()));
        }
    }

    #[test]
    fn gpu_rays_match_cpu() {
        let Some(gpu) = device() else { return };
        let tris = corpus("coedge_orientation_regression.skp");
        let bvh = Lbvh::build(tris);
        let (lo, hi) = bvh.bounds().unwrap();
        let rays = random_rays(lo, hi, 20_000, 2);
        let cpu = bvh.rays(&rays);
        let got = gpu.rays(&bvh, &rays).unwrap();
        assert_eq!(got, cpu);
        assert!(cpu.iter().filter(|h| h.is_some()).count() > 10_000);
        // Also through `Accel`, with a GPU-built tree.
        let accel = Accel::new();
        assert!(accel.is_gpu());
        let gpu_bvh = accel.build_bvh(bvh.triangles().to_vec());
        assert_eq!(gpu_bvh, bvh);
        assert_eq!(accel.rays(&gpu_bvh, &rays[..1000]), cpu[..1000]);
    }

    #[test]
    fn gpu_projection_and_normals_match_cpu() {
        let Some(gpu) = device() else { return };
        let tris = corpus("gondola_v20.skp");
        let normals = batch::triangle_normals(&tris);
        assert_eq!(gpu.triangle_normals(&tris).unwrap(), normals);

        // Project each triangle's corners into the plane of its (non-degenerate) normal.
        let faces: Vec<DVec3> = normals
            .iter()
            .copied()
            .filter(|n| *n != DVec3::ZERO)
            .collect();
        let points: Vec<(u32, DVec3)> = tris
            .iter()
            .zip(&normals)
            .filter(|(_, n)| **n != DVec3::ZERO)
            .enumerate()
            .flat_map(|(f, (t, _))| t.map(|p| (f as u32, p)))
            .collect();
        let cpu = batch::project_points(&faces, &points);
        assert_eq!(gpu.project_points(&faces, &points).unwrap(), cpu);
        // And the CPU batch is the kernel's projection.
        for (&(f, p), q) in points.iter().zip(&cpu).step_by(97) {
            assert_eq!(Plane::new(faces[f as usize], p).project(p), *q);
        }
    }
}
