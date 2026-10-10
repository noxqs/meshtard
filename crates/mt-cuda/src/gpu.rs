//! CUDA kernels (CUDA C compiled at run time with NVRTC) and their host side.
//!
//! Each kernel mirrors a CPU function in [`crate::lbvh`] or [`crate::batch`] operation by
//! operation. Kernels are compiled with `--fmad=false` so that no multiply-add is fused,
//! and double division and square root are IEEE-rounded on both sides, which makes the
//! results bit-identical.

use std::sync::Arc;

use cudarc::driver::{
    CudaContext, CudaFunction, CudaSlice, CudaStream, LaunchConfig, PushKernelArg,
};
use cudarc::nvrtc::{CompileOptions, compile_ptx_with_opts};
use glam::{DVec2, DVec3};
use mt_kernel::bvh::Hit;

use crate::lbvh::{self, LEAF, Lbvh, Node};

const SOURCE: &str = r#"
typedef unsigned long long u64;
typedef unsigned int u32;
#define LEAF 0x80000000u
#define INF __longlong_as_double(0x7ff0000000000000ULL)
#define MORTON_MAX 2097151u

// Same as glam's min/max: `a < b ? a : b`.
__device__ __forceinline__ double gmin(double a, double b) { return a < b ? a : b; }
__device__ __forceinline__ double gmax(double a, double b) { return a > b ? a : b; }

// ---- BVH build -------------------------------------------------------------------

// Per-block min/max of triangle centroids (corner sums); the host combines the blocks.
extern "C" __global__ void centroid_bounds(const double* tris, u32 n, double* partial) {
    __shared__ double s[6][256];
    u32 tid = threadIdx.x;
    double lo[3] = {INF, INF, INF}, hi[3] = {-INF, -INF, -INF};
    for (u32 i = blockIdx.x * blockDim.x + tid; i < n; i += gridDim.x * blockDim.x) {
        const double* t = tris + 9 * (size_t)i;
        for (int k = 0; k < 3; ++k) {
            double c = (t[k] + t[3 + k]) + t[6 + k];
            lo[k] = gmin(lo[k], c);
            hi[k] = gmax(hi[k], c);
        }
    }
    for (int k = 0; k < 3; ++k) { s[k][tid] = lo[k]; s[3 + k][tid] = hi[k]; }
    __syncthreads();
    for (u32 step = blockDim.x / 2; step > 0; step >>= 1) {
        if (tid < step) {
            for (int k = 0; k < 3; ++k) {
                s[k][tid] = gmin(s[k][tid], s[k][tid + step]);
                s[3 + k][tid] = gmax(s[3 + k][tid], s[3 + k][tid + step]);
            }
        }
        __syncthreads();
    }
    if (tid < 6) partial[6 * blockIdx.x + tid] = s[tid][0];
}

__device__ u32 quantize(double x, double lo, double scale) {
    double t = (x - lo) * scale;
    if (!(t > 0.0)) return 0;
    if (t >= (double)MORTON_MAX) return MORTON_MAX;
    return (u32)t;
}

__device__ u64 spread(u32 v) {
    u64 x = (u64)v & 0x1fffffULL;
    x = (x | x << 32) & 0x001f00000000ffffULL;
    x = (x | x << 16) & 0x001f0000ff0000ffULL;
    x = (x | x << 8) & 0x100f00f00f00f00fULL;
    x = (x | x << 4) & 0x10c30c30c30c30c3ULL;
    x = (x | x << 2) & 0x1249249249249249ULL;
    return x;
}

extern "C" __global__ void morton_codes(const double* tris, u32 n,
        double lx, double ly, double lz, double sx, double sy, double sz,
        u64* keys, u32* vals) {
    u32 i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= n) return;
    const double* t = tris + 9 * (size_t)i;
    double cx = (t[0] + t[3]) + t[6];
    double cy = (t[1] + t[4]) + t[7];
    double cz = (t[2] + t[5]) + t[8];
    keys[i] = spread(quantize(cx, lx, sx)) << 2 | spread(quantize(cy, ly, sy)) << 1
        | spread(quantize(cz, lz, sz));
    vals[i] = i;
}

// Stable LSD radix sort, 4 bits per pass, tiles of 256 keys (one per thread).
// hist is digit-major (hist[digit * blocks + block]) so one exclusive scan over it gives
// every tile's output offset per digit.
extern "C" __global__ void radix_hist(const u64* keys, u32 n, u32 shift, u32* hist, u32 blocks) {
    __shared__ u32 c[16];
    if (threadIdx.x < 16) c[threadIdx.x] = 0;
    __syncthreads();
    u32 i = blockIdx.x * 256 + threadIdx.x;
    if (i < n) atomicAdd(&c[(u32)(keys[i] >> shift) & 15u], 1u);
    __syncthreads();
    if (threadIdx.x < 16) hist[threadIdx.x * blocks + blockIdx.x] = c[threadIdx.x];
}

// Exclusive prefix sum in place, by one block of 1024 threads.
extern "C" __global__ void scan_exclusive(u32* a, u32 m) {
    __shared__ u32 s[1024];
    __shared__ u32 carry;
    u32 tid = threadIdx.x;
    if (tid == 0) carry = 0;
    __syncthreads();
    for (u32 base = 0; base < m; base += 1024) {
        u32 i = base + tid;
        u32 v = i < m ? a[i] : 0;
        s[tid] = v;
        __syncthreads();
        for (u32 off = 1; off < 1024; off <<= 1) {
            u32 x = tid >= off ? s[tid - off] : 0;
            __syncthreads();
            s[tid] += x;
            __syncthreads();
        }
        if (i < m) a[i] = carry + s[tid] - v;
        __syncthreads();
        if (tid == 1023) carry += s[1023];
        __syncthreads();
    }
}

extern "C" __global__ void radix_scatter(const u64* kin, const u32* vin, u64* kout, u32* vout,
        u32 n, u32 shift, const u32* offsets, u32 blocks) {
    __shared__ u32 warp_count[16][8];
    u32 tid = threadIdx.x, lane = tid & 31, warp = tid >> 5;
    u32 i = blockIdx.x * 256 + tid;
    bool valid = i < n;
    u64 k = valid ? kin[i] : 0;
    u32 d = valid ? ((u32)(k >> shift) & 15u) : 16u;
    u32 rank = 0;
    for (u32 dd = 0; dd < 16; ++dd) {
        u32 m = __ballot_sync(0xffffffffu, d == dd);
        if (lane == 0) warp_count[dd][warp] = __popc(m);
        if (d == dd) rank = __popc(m & ((1u << lane) - 1u));
    }
    __syncthreads();
    if (!valid) return;
    u32 o = offsets[d * blocks + blockIdx.x] + rank;
    for (u32 w = 0; w < warp; ++w) o += warp_count[d][w];
    kout[o] = k;
    vout[o] = vin[i];
}

__device__ int delta(const u64* keys, u32 n, long long i, long long j) {
    if (j < 0 || j >= (long long)n) return -1;
    u64 a = keys[i], b = keys[j];
    if (a == b) return 64 + __clz((u32)i ^ (u32)j);
    return __clzll(a ^ b);
}

// Karras: the children of internal node i, from the sorted keys.
extern "C" __global__ void hierarchy(const u64* keys, const u32* order, u32 n,
        u32* children, u32* parent, u32* leaf_parent) {
    u32 ui = blockIdx.x * blockDim.x + threadIdx.x;
    if (ui + 1 >= n) return;
    long long i = ui;
    int dl = delta(keys, n, i, i + 1) - delta(keys, n, i, i - 1);
    long long d = dl > 0 ? 1 : (dl < 0 ? -1 : 0);
    int d_min = delta(keys, n, i, i - d);
    long long l_max = 2;
    while (delta(keys, n, i, i + l_max * d) > d_min) l_max *= 2;
    long long l = 0;
    for (long long t = l_max / 2; t >= 1; t /= 2)
        if (delta(keys, n, i, i + (l + t) * d) > d_min) l += t;
    long long j = i + l * d;
    int d_node = delta(keys, n, i, j);
    long long s = 0, t = l;
    for (;;) {
        t = (t + 1) / 2;
        if (delta(keys, n, i, i + (s + t) * d) > d_node) s += t;
        if (t <= 1) break;
    }
    long long gamma = i + s * d + (d < 0 ? d : 0);
    long long lo = i < j ? i : j, hi = i < j ? j : i;
    u32 left, right;
    if (lo == gamma) { left = LEAF | order[gamma]; leaf_parent[gamma] = ui; }
    else { left = (u32)gamma; parent[gamma] = ui; }
    if (hi == gamma + 1) { right = LEAF | order[gamma + 1]; leaf_parent[gamma + 1] = ui; }
    else { right = (u32)(gamma + 1); parent[gamma + 1] = ui; }
    children[2 * ui] = left;
    children[2 * ui + 1] = right;
}

__device__ void child_bounds(const double* tris, volatile double* bounds, u32 c, double* b) {
    if (c & LEAF) {
        const double* t = tris + 9 * (size_t)(c & ~LEAF);
        for (int k = 0; k < 3; ++k) {
            b[k] = gmin(gmin(t[k], t[3 + k]), t[6 + k]);
            b[3 + k] = gmax(gmax(t[k], t[3 + k]), t[6 + k]);
        }
    } else {
        for (int k = 0; k < 6; ++k) b[k] = bounds[6 * (size_t)c + k];
    }
}

// Bottom-up bounds: the second thread to reach a node computes it from both children.
extern "C" __global__ void bounds_up(const double* tris, const u32* children, const u32* parent,
        const u32* leaf_parent, u32* flags, double* bounds, u32 n) {
    u32 j = blockIdx.x * blockDim.x + threadIdx.x;
    if (j >= n) return;
    volatile double* vb = bounds;
    u32 node = leaf_parent[j];
    for (;;) {
        __threadfence();
        if (atomicAdd(&flags[node], 1u) == 0) return;
        __threadfence();
        double l[6], r[6];
        child_bounds(tris, vb, children[2 * node], l);
        child_bounds(tris, vb, children[2 * node + 1], r);
        for (int k = 0; k < 3; ++k) {
            vb[6 * (size_t)node + k] = gmin(l[k], r[k]);
            vb[6 * (size_t)node + 3 + k] = gmax(l[3 + k], r[3 + k]);
        }
        if (node == 0) return;
        node = parent[node];
    }
}

// ---- Queries ---------------------------------------------------------------------

// mt_kernel::bvh::ray_triangle (Moller-Trumbore, two-sided), operation for operation.
__device__ bool ray_triangle(const double* o, const double* d, const double* T, double* t_out) {
    double e1x = T[3] - T[0], e1y = T[4] - T[1], e1z = T[5] - T[2];
    double e2x = T[6] - T[0], e2y = T[7] - T[1], e2z = T[8] - T[2];
    double px = d[1] * e2z - e2y * d[2];
    double py = d[2] * e2x - e2z * d[0];
    double pz = d[0] * e2y - e2x * d[1];
    double det = (e1x * px + e1y * py) + e1z * pz;
    double l1 = sqrt((e1x * e1x + e1y * e1y) + e1z * e1z);
    double l2 = sqrt((e2x * e2x + e2y * e2y) + e2z * e2z);
    double l3 = sqrt((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]);
    if (fabs(det) < ((1e-14 * l1) * l2) * l3) return false;
    double inv = 1.0 / det;
    double sx = o[0] - T[0], sy = o[1] - T[1], sz = o[2] - T[2];
    double u = ((sx * px + sy * py) + sz * pz) * inv;
    if (!(0.0 <= u && u <= 1.0)) return false;
    double qx = sy * e1z - e1y * sz;
    double qy = sz * e1x - e1z * sx;
    double qz = sx * e1y - e1x * sy;
    double v = ((d[0] * qx + d[1] * qy) + d[2] * qz) * inv;
    if (v < 0.0 || u + v > 1.0) return false;
    double t = ((e2x * qx + e2y * qy) + e2z * qz) * inv;
    if (!(t > 0.0)) return false;
    *t_out = t;
    return true;
}

__device__ bool slab(const double* b, const double* o, const double* inv, double t_max) {
    double near[3], far[3];
    for (int k = 0; k < 3; ++k) {
        double t0 = (b[k] - o[k]) * inv[k];
        double t1 = (b[3 + k] - o[k]) * inv[k];
        near[k] = gmin(t0, t1);
        far[k] = gmax(t0, t1);
    }
    double ne = fmax(gmax(near[0], gmax(near[1], near[2])), 0.0);
    double fa = fmin(gmin(far[0], gmin(far[1], far[2])), t_max);
    return ne <= fa;
}

// Nearest hit per ray; ties go to the lowest triangle index. Depth is at most 96
// (63 key bits + 32 index bits + 1), so a stack of 128 suffices.
extern "C" __global__ void rays_nearest(const double* tris, u32 n_tris, const double* bounds,
        const u32* children, u32 root, const double* rays, u32 m, double* out_t, u32* out_i) {
    u32 r = blockIdx.x * blockDim.x + threadIdx.x;
    if (r >= m) return;
    double o[3] = {rays[6 * (size_t)r], rays[6 * (size_t)r + 1], rays[6 * (size_t)r + 2]};
    double d[3] = {rays[6 * (size_t)r + 3], rays[6 * (size_t)r + 4], rays[6 * (size_t)r + 5]};
    double inv[3] = {1.0 / d[0], 1.0 / d[1], 1.0 / d[2]};
    double best_t = INF;
    u32 best_i = 0xffffffffu;
    u32 stack[128];
    int sp = 0;
    if (n_tris > 0) stack[sp++] = root;
    while (sp > 0) {
        u32 c = stack[--sp];
        if (c & LEAF) {
            u32 i = c & ~LEAF;
            double t;
            if (ray_triangle(o, d, tris + 9 * (size_t)i, &t)
                    && (t < best_t || (t == best_t && i < best_i))) {
                best_t = t;
                best_i = i;
            }
            continue;
        }
        if (slab(bounds + 6 * (size_t)c, o, inv, best_t)) {
            stack[sp++] = children[2 * c + 1];
            stack[sp++] = children[2 * c];
        }
    }
    out_t[r] = best_t;
    out_i[r] = best_i;
}

// ---- Per-face / per-triangle batches -----------------------------------------------

__device__ void normalize(double* v) {
    double rcp = 1.0 / sqrt((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]);
    v[0] = v[0] * rcp; v[1] = v[1] * rcp; v[2] = v[2] * rcp;
}

__device__ void cross(const double* a, const double* b, double* out) {
    out[0] = a[1] * b[2] - b[1] * a[2];
    out[1] = a[2] * b[0] - b[2] * a[0];
    out[2] = a[0] * b[1] - b[0] * a[1];
}

// mt_kernel::geom::Plane::{new, basis, project}.
extern "C" __global__ void project_points(const double* normals, const u32* faces,
        const double* points, u32 m, double* out) {
    u32 i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= m) return;
    const double* nf = normals + 3 * (size_t)faces[i];
    double n[3] = {nf[0], nf[1], nf[2]};
    normalize(n);
    double helper[3] = {0.0, 0.0, 0.0};
    if (fabs(n[0]) < 0.9) helper[0] = 1.0; else helper[1] = 1.0;
    double u[3], v[3];
    cross(helper, n, u);
    normalize(u);
    cross(n, u, v);
    const double* p = points + 3 * (size_t)i;
    out[2 * (size_t)i] = (p[0] * u[0] + p[1] * u[1]) + p[2] * u[2];
    out[2 * (size_t)i + 1] = (p[0] * v[0] + p[1] * v[1]) + p[2] * v[2];
}

// (b - a) x (c - a), normalized, or zero (glam's normalize_or_zero).
extern "C" __global__ void triangle_normals(const double* tris, u32 n, double* out) {
    u32 i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= n) return;
    const double* t = tris + 9 * (size_t)i;
    double e1[3] = {t[3] - t[0], t[4] - t[1], t[5] - t[2]};
    double e2[3] = {t[6] - t[0], t[7] - t[1], t[8] - t[2]};
    double c[3];
    cross(e1, e2, c);
    double rcp = 1.0 / sqrt((c[0] * c[0] + c[1] * c[1]) + c[2] * c[2]);
    bool ok = isfinite(rcp) && rcp > 0.0;
    for (int k = 0; k < 3; ++k) out[3 * (size_t)i + k] = ok ? c[k] * rcp : 0.0;
}
"#;

const KERNELS: [&str; 10] = [
    "centroid_bounds",
    "morton_codes",
    "radix_hist",
    "scan_exclusive",
    "radix_scatter",
    "hierarchy",
    "bounds_up",
    "rays_nearest",
    "project_points",
    "triangle_normals",
];

/// Blocks used by the centroid bounds reduction (grid-stride over the triangles).
const BOUNDS_BLOCKS: u32 = 256;

#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<cudarc::driver::DriverError> for Error {
    fn from(e: cudarc::driver::DriverError) -> Self {
        Self(format!("CUDA driver: {e}"))
    }
}

fn err(msg: impl Into<String>) -> Error {
    Error(msg.into())
}

/// A CUDA device with the kernels loaded.
pub struct Gpu {
    stream: Arc<CudaStream>,
    name: String,
    functions: Vec<CudaFunction>,
}

fn cfg(n: u32, block: u32) -> LaunchConfig {
    LaunchConfig {
        grid_dim: (n.div_ceil(block).max(1), 1, 1),
        block_dim: (block, 1, 1),
        shared_mem_bytes: 0,
    }
}

fn len_u32(n: usize) -> Result<u32, Error> {
    u32::try_from(n)
        .ok()
        .filter(|&n| n < LEAF)
        .ok_or_else(|| err("batch too large"))
}

impl Gpu {
    /// Opens device 0 and compiles the kernels (NVRTC, a fraction of a second).
    pub fn new() -> Result<Self, Error> {
        let ctx = CudaContext::new(0)?;
        let name = ctx.name()?;
        let opts = CompileOptions {
            fmad: Some(false),
            ..Default::default()
        };
        let ptx = compile_ptx_with_opts(SOURCE, opts).map_err(|e| err(format!("NVRTC: {e}")))?;
        let module = ctx.load_module(ptx)?;
        let functions = KERNELS
            .iter()
            .map(|k| module.load_function(k))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            stream: ctx.default_stream(),
            name,
            functions,
        })
    }

    /// Device name, e.g. "NVIDIA GeForce RTX 4070".
    pub fn name(&self) -> &str {
        &self.name
    }

    fn f(&self, name: &str) -> &CudaFunction {
        &self.functions[KERNELS.iter().position(|k| *k == name).unwrap()]
    }

    fn upload_triangles(&self, triangles: &[[DVec3; 3]]) -> Result<CudaSlice<f64>, Error> {
        let flat: Vec<f64> = triangles
            .iter()
            .flat_map(|t| t.iter().flat_map(|p| p.to_array()))
            .collect();
        Ok(self.stream.clone_htod(&flat)?)
    }

    /// Builds the BVH nodes on the GPU; same result as [`Lbvh::build`].
    pub fn build_bvh(&self, triangles: &[[DVec3; 3]]) -> Result<(Vec<Node>, u32), Error> {
        let n = len_u32(triangles.len())?;
        if n < 2 {
            return Ok((Vec::new(), LEAF));
        }
        let s = &self.stream;
        let tris = self.upload_triangles(triangles)?;

        // Centroid bounds and Morton codes.
        let mut partial = s.alloc_zeros::<f64>(6 * BOUNDS_BLOCKS as usize)?;
        let mut launch = s.launch_builder(self.f("centroid_bounds"));
        launch.arg(&tris).arg(&n).arg(&mut partial);
        // SAFETY: arguments match the kernel signature; `partial` has 6 per block.
        unsafe {
            launch.launch(LaunchConfig {
                grid_dim: (BOUNDS_BLOCKS, 1, 1),
                block_dim: (256, 1, 1),
                shared_mem_bytes: 0,
            })
        }?;
        let partial = s.clone_dtoh(&partial)?;
        let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for b in partial.as_chunks::<6>().0.iter() {
            lo = lo.min(DVec3::new(b[0], b[1], b[2]));
            hi = hi.max(DVec3::new(b[3], b[4], b[5]));
        }
        let (lo, scale) = lbvh::morton_params(lo, hi);
        let mut keys = s.alloc_zeros::<u64>(n as usize)?;
        let mut vals = s.alloc_zeros::<u32>(n as usize)?;
        let mut launch = s.launch_builder(self.f("morton_codes"));
        launch.arg(&tris).arg(&n);
        for v in lo.iter().chain(&scale) {
            launch.arg(v);
        }
        launch.arg(&mut keys).arg(&mut vals);
        // SAFETY: as above; `keys` and `vals` hold `n` elements.
        unsafe { launch.launch(cfg(n, 256)) }?;

        // Radix sort by key, stable, so equal keys stay in triangle order.
        let blocks = n.div_ceil(256);
        let mut keys2 = s.alloc_zeros::<u64>(n as usize)?;
        let mut vals2 = s.alloc_zeros::<u32>(n as usize)?;
        let mut hist = s.alloc_zeros::<u32>(16 * blocks as usize)?;
        let hist_len = 16 * blocks;
        for pass in 0..16u32 {
            let shift = 4 * pass;
            let (kin, vin, kout, vout) = if pass % 2 == 0 {
                (&keys, &vals, &mut keys2, &mut vals2)
            } else {
                (&keys2, &vals2, &mut keys, &mut vals)
            };
            let mut launch = s.launch_builder(self.f("radix_hist"));
            launch
                .arg(kin)
                .arg(&n)
                .arg(&shift)
                .arg(&mut hist)
                .arg(&blocks);
            // SAFETY: `hist` holds 16 counters per block.
            unsafe { launch.launch(cfg(n, 256)) }?;
            let mut launch = s.launch_builder(self.f("scan_exclusive"));
            launch.arg(&mut hist).arg(&hist_len);
            // SAFETY: one block scanning `hist_len` entries.
            unsafe {
                launch.launch(LaunchConfig {
                    grid_dim: (1, 1, 1),
                    block_dim: (1024, 1, 1),
                    shared_mem_bytes: 0,
                })
            }?;
            let mut launch = s.launch_builder(self.f("radix_scatter"));
            launch
                .arg(kin)
                .arg(vin)
                .arg(kout)
                .arg(vout)
                .arg(&n)
                .arg(&shift)
                .arg(&hist)
                .arg(&blocks);
            // SAFETY: offsets from the scan are a permutation of 0..n.
            unsafe { launch.launch(cfg(n, 256)) }?;
        }
        // An even number of passes leaves the result in `keys`/`vals`.

        // Hierarchy and bounds.
        let internal = n as usize - 1;
        let mut children = s.alloc_zeros::<u32>(2 * internal)?;
        let mut parent = s.alloc_zeros::<u32>(internal)?;
        let mut leaf_parent = s.alloc_zeros::<u32>(n as usize)?;
        let mut launch = s.launch_builder(self.f("hierarchy"));
        launch
            .arg(&keys)
            .arg(&vals)
            .arg(&n)
            .arg(&mut children)
            .arg(&mut parent)
            .arg(&mut leaf_parent);
        // SAFETY: one thread per internal node, outputs sized accordingly.
        unsafe { launch.launch(cfg(n - 1, 256)) }?;
        let mut flags = s.alloc_zeros::<u32>(internal)?;
        let mut bounds = s.alloc_zeros::<f64>(6 * internal)?;
        let mut launch = s.launch_builder(self.f("bounds_up"));
        launch
            .arg(&tris)
            .arg(&children)
            .arg(&parent)
            .arg(&leaf_parent)
            .arg(&mut flags)
            .arg(&mut bounds)
            .arg(&n);
        // SAFETY: one thread per leaf; every internal node is reached twice.
        unsafe { launch.launch(cfg(n, 256)) }?;

        let children = s.clone_dtoh(&children)?;
        let bounds = s.clone_dtoh(&bounds)?;
        let nodes = children
            .as_chunks::<2>()
            .0
            .iter()
            .zip(bounds.as_chunks::<6>().0.iter())
            .map(|(c, b)| Node {
                min: [b[0], b[1], b[2]],
                max: [b[3], b[4], b[5]],
                left: c[0],
                right: c[1],
            })
            .collect();
        Ok((nodes, 0))
    }

    /// Nearest hit for each ray; same result as [`Lbvh::rays`].
    pub fn rays(&self, bvh: &Lbvh, rays: &[crate::Ray]) -> Result<Vec<Option<Hit>>, Error> {
        let m = len_u32(rays.len())?;
        if m == 0 {
            return Ok(Vec::new());
        }
        let s = &self.stream;
        let mut flat_bounds = Vec::with_capacity(6 * bvh.nodes().len() + 6);
        let mut flat_children = Vec::with_capacity(2 * bvh.nodes().len() + 2);
        for n in bvh.nodes() {
            flat_bounds.extend(n.min.iter().chain(&n.max));
            flat_children.extend([n.left, n.right]);
        }
        // Device buffers may not be empty.
        flat_bounds.resize(flat_bounds.len().max(6), 0.0);
        flat_children.resize(flat_children.len().max(2), 0);
        let tris = if bvh.is_empty() {
            s.alloc_zeros::<f64>(9)?
        } else {
            self.upload_triangles(bvh.triangles())?
        };
        let bounds = s.clone_htod(&flat_bounds)?;
        let children = s.clone_htod(&flat_children)?;
        let flat_rays: Vec<f64> = rays
            .iter()
            .flat_map(|(o, d)| o.to_array().into_iter().chain(d.to_array()))
            .collect();
        let rays_dev = s.clone_htod(&flat_rays)?;
        let mut out_t = s.alloc_zeros::<f64>(m as usize)?;
        let mut out_i = s.alloc_zeros::<u32>(m as usize)?;
        let n_tris = len_u32(bvh.len())?;
        let root = bvh.root();
        let mut launch = s.launch_builder(self.f("rays_nearest"));
        launch
            .arg(&tris)
            .arg(&n_tris)
            .arg(&bounds)
            .arg(&children)
            .arg(&root)
            .arg(&rays_dev)
            .arg(&m)
            .arg(&mut out_t)
            .arg(&mut out_i);
        // SAFETY: one thread per ray; node and triangle indices come from a valid BVH.
        unsafe { launch.launch(cfg(m, 128)) }?;
        let t = s.clone_dtoh(&out_t)?;
        let i = s.clone_dtoh(&out_i)?;
        Ok(t.into_iter()
            .zip(i)
            .map(|(t, i)| {
                (i != u32::MAX).then_some(Hit {
                    t,
                    triangle: i as usize,
                })
            })
            .collect())
    }

    /// Same result as [`crate::batch::project_points`].
    pub fn project_points(
        &self,
        normals: &[DVec3],
        points: &[(u32, DVec3)],
    ) -> Result<Vec<DVec2>, Error> {
        let m = len_u32(points.len())?;
        if m == 0 {
            return Ok(Vec::new());
        }
        if points.iter().any(|&(f, _)| f as usize >= normals.len()) {
            return Err(err("point refers to a missing face"));
        }
        let s = &self.stream;
        let flat_n: Vec<f64> = normals.iter().flat_map(|n| n.to_array()).collect();
        let faces: Vec<u32> = points.iter().map(|p| p.0).collect();
        let flat_p: Vec<f64> = points.iter().flat_map(|p| p.1.to_array()).collect();
        let normals_dev = s.clone_htod(&flat_n)?;
        let faces_dev = s.clone_htod(&faces)?;
        let points_dev = s.clone_htod(&flat_p)?;
        let mut out = s.alloc_zeros::<f64>(2 * m as usize)?;
        let mut launch = s.launch_builder(self.f("project_points"));
        launch
            .arg(&normals_dev)
            .arg(&faces_dev)
            .arg(&points_dev)
            .arg(&m)
            .arg(&mut out);
        // SAFETY: one thread per point; face indices checked above.
        unsafe { launch.launch(cfg(m, 256)) }?;
        let out = s.clone_dtoh(&out)?;
        Ok(out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| DVec2::new(c[0], c[1]))
            .collect())
    }

    /// Same result as [`crate::batch::triangle_normals`].
    pub fn triangle_normals(&self, triangles: &[[DVec3; 3]]) -> Result<Vec<DVec3>, Error> {
        let n = len_u32(triangles.len())?;
        if n == 0 {
            return Ok(Vec::new());
        }
        let s = &self.stream;
        let tris = self.upload_triangles(triangles)?;
        let mut out = s.alloc_zeros::<f64>(3 * n as usize)?;
        let mut launch = s.launch_builder(self.f("triangle_normals"));
        launch.arg(&tris).arg(&n).arg(&mut out);
        // SAFETY: one thread per triangle.
        unsafe { launch.launch(cfg(n, 256)) }?;
        let out = s.clone_dtoh(&out)?;
        Ok(out
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| DVec3::new(c[0], c[1], c[2]))
            .collect())
    }
}
