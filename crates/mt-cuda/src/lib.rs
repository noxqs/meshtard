//! GPU (CUDA) versions of the kernel's large, parallel work, as listed in PLAN.md.
//!
//! Every function has a CPU version giving bit-identical results; the GPU kernels are
//! compiled with `--fmad=false` and do their floating point in the same order as the CPU
//! code (which in turn matches `glam`), so results can be compared with `==`.
//!
//! What is here:
//! - [`Lbvh`]: a linear BVH (Morton codes, radix sort, Karras hierarchy). Built on the
//!   GPU by [`Accel::build_bvh`] or on the CPU by [`Lbvh::build`]; both give the same
//!   tree. It answers the same queries as [`mt_kernel::bvh::Bvh`].
//! - [`Accel::rays`]: nearest hit for many rays at once (picking, inference).
//! - [`Accel::project_points`], [`Accel::triangle_normals`]: per-face and per-triangle
//!   math.
//!
//! Not here: GPU triangulation. Ear clipping is sequential per face and its run time
//! varies a lot between faces (most faces are quads, a few have hundreds of vertices
//! and holes), so one GPU thread per face would mostly wait on the slowest face, and the
//! input (loops of vertex ids) must first be flattened on the CPU anyway. Only the part
//! of triangulation that is uniform per vertex, projecting face vertices into the face
//! plane, is offered as a GPU batch ([`Accel::project_points`]).
//!
//! Without the `cuda` feature, or without a usable CUDA device, [`Accel`] runs everything
//! on the CPU.

pub mod batch;
#[cfg(feature = "cuda")]
#[allow(unsafe_code)]
pub mod gpu;
pub mod lbvh;
pub mod scene;

use glam::{DVec2, DVec3};
pub use lbvh::Lbvh;
pub use mt_kernel::bvh::Hit;

/// A ray: origin and direction (need not be unit; hit distances are in units of it).
pub type Ray = (DVec3, DVec3);

/// Runs work on the GPU when one is available, otherwise on the CPU.
pub struct Accel {
    #[cfg(feature = "cuda")]
    gpu: Option<gpu::Gpu>,
}

impl Default for Accel {
    fn default() -> Self {
        Self::new()
    }
}

impl Accel {
    /// Uses CUDA device 0 if the `cuda` feature is on and a device is found.
    pub fn new() -> Self {
        #[cfg(feature = "cuda")]
        {
            let gpu = gpu::Gpu::new()
                .inspect_err(|e| log::info!("CUDA unavailable, using the CPU: {e}"))
                .ok();
            Self { gpu }
        }
        #[cfg(not(feature = "cuda"))]
        Self::cpu()
    }

    /// CPU only.
    pub fn cpu() -> Self {
        Self {
            #[cfg(feature = "cuda")]
            gpu: None,
        }
    }

    pub fn is_gpu(&self) -> bool {
        #[cfg(feature = "cuda")]
        return self.gpu.is_some();
        #[cfg(not(feature = "cuda"))]
        false
    }

    /// Builds a BVH. A GPU failure falls back to the CPU build, which gives the same tree.
    pub fn build_bvh(&self, triangles: Vec<[DVec3; 3]>) -> Lbvh {
        #[cfg(feature = "cuda")]
        if let Some(gpu) = &self.gpu {
            match gpu.build_bvh(&triangles) {
                Ok((nodes, root)) => return Lbvh::from_parts(triangles, nodes, root),
                Err(e) => log::warn!("GPU BVH build failed, using the CPU: {e}"),
            }
        }
        Lbvh::build(triangles)
    }

    /// Nearest hit of each ray, as [`Lbvh::ray`].
    pub fn rays(&self, bvh: &Lbvh, rays: &[Ray]) -> Vec<Option<Hit>> {
        #[cfg(feature = "cuda")]
        if let Some(gpu) = &self.gpu {
            match gpu.rays(bvh, rays) {
                Ok(hits) => return hits,
                Err(e) => log::warn!("GPU ray batch failed, using the CPU: {e}"),
            }
        }
        bvh.rays(rays)
    }

    /// As [`batch::project_points`].
    pub fn project_points(&self, normals: &[DVec3], points: &[(u32, DVec3)]) -> Vec<DVec2> {
        #[cfg(feature = "cuda")]
        if let Some(gpu) = &self.gpu {
            match gpu.project_points(normals, points) {
                Ok(out) => return out,
                Err(e) => log::warn!("GPU projection failed, using the CPU: {e}"),
            }
        }
        batch::project_points(normals, points)
    }

    /// As [`batch::triangle_normals`].
    pub fn triangle_normals(&self, triangles: &[[DVec3; 3]]) -> Vec<DVec3> {
        #[cfg(feature = "cuda")]
        if let Some(gpu) = &self.gpu {
            match gpu.triangle_normals(triangles) {
                Ok(out) => return out,
                Err(e) => log::warn!("GPU normals failed, using the CPU: {e}"),
            }
        }
        batch::triangle_normals(triangles)
    }
}
