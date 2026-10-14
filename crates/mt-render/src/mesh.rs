//! Shaded, two-sided faces.

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::Vec3;
use wgpu::util::DeviceExt;

/// One face vertex. Triangles are wound counter-clockwise around `normal` when seen
/// from the front side.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FaceVertex {
    pub position: [f32; 3],
    /// Front-face normal (unit). Back side uses the negated normal.
    pub normal: [f32; 3],
    /// sRGB + alpha of the front and back material. Alpha below 1 (SketchUp material
    /// opacity) draws the triangle blended, after the opaque ones.
    pub front: [f32; 4],
    pub back: [f32; 4],
    /// Texture coordinates of the front (xy) and back (zw) side, in repeats with v up
    /// (SketchUp's convention: the image's bottom row at v = 0).
    pub uv: [f32; 4],
    /// Texture of the front and back side: an index into
    /// [`crate::Viewport::set_textures`], or [`FaceVertex::NO_TEXTURE`]. Shown in
    /// [`crate::FaceStyle::ShadedWithTextures`] instead of the color's RGB (alpha
    /// multiplies).
    pub texture: [u32; 2],
    /// Projective denominators of the front and back texture coordinates (1 for an
    /// affine mapping): the drawn coordinates are `uv / q`, divided per pixel, so a
    /// distorted (perspective) texture position is drawn exactly.
    pub q: [f32; 2],
}

impl FaceVertex {
    pub const NO_TEXTURE: u32 = u32::MAX;

    pub(crate) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<FaceVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4,
            4 => Float32x4, 5 => Uint32x2, 6 => Float32x2
        ],
    };

    /// A vertex of an untextured face.
    pub fn new(position: [f32; 3], normal: [f32; 3], front: [f32; 4], back: [f32; 4]) -> Self {
        Self {
            position,
            normal,
            front,
            back,
            uv: [0.0; 4],
            texture: [Self::NO_TEXTURE; 2],
            q: [1.0; 2],
        }
    }

    fn opaque(&self) -> bool {
        self.front[3] >= 1.0 && self.back[3] >= 1.0
    }
}

/// Per-instance data of a mesh drawn many times: the instance's transform.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct InstanceData {
    /// Column-major.
    pub columns: [[f32; 4]; 4],
}

impl InstanceData {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4
    ];

    /// Faces: one transform per instance.
    pub(crate) const FACE_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<InstanceData>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &Self::ATTRIBUTES,
    };

    /// Edges: the segments are the instances and the transform is repeated for each
    /// of the 6 vertices of a segment's quad.
    pub(crate) const EDGE_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<InstanceData>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

/// GPU buffers of the faces: opaque triangles first, then translucent ones.
pub(crate) struct FaceBuffers {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    /// Index ranges into `indices`.
    pub opaque: std::ops::Range<u32>,
    pub translucent: std::ops::Range<u32>,
    /// Axis-aligned bounds of all referenced vertices.
    pub bounds: (Vec3, Vec3),
    /// With sequential indices: the bounds of each block of [`BOUNDS_BLOCK`] vertices
    /// (an update bounds only the blocks it changes again).
    block_bounds: Option<Vec<(Vec3, Vec3)>>,
    /// Translucent triangles and their centroids, for sorting back to front.
    translucent_tris: Vec<([u32; 3], Vec3)>,
    /// The view the translucent range was last sorted for.
    sorted_for: Option<(Vec3, Vec3, bool)>,
    /// Number of vertices and indices.
    counts: (usize, usize),
    /// Per triangle, whether it is opaque (one bit each), when the indices are
    /// `0..n` (triangle `t` is vertices `3 t..3 t + 3`): [`FaceBuffers::update`]
    /// rewrites only changed vertices then.
    opaque_bits: Option<Vec<u64>>,
}

fn index_buffer(device: &wgpu::Device, sorted: &[u32]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("face indices"),
        contents: bytemuck::cast_slice(sorted),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
    })
}

/// Whether `indices` are `0, 1, 2, ...`.
fn sequential(indices: &[u32]) -> bool {
    indices.iter().enumerate().all(|(i, &v)| v as usize == i)
}

/// Vertices per block of `FaceBuffers::block_bounds`.
const BOUNDS_BLOCK: usize = 1 << 16;

/// The bounds of each block of vertices, when the indices are sequential.
fn block_bounds(vertices: &[FaceVertex], indices: &[u32]) -> Option<Vec<(Vec3, Vec3)>> {
    (indices.len() == vertices.len() && sequential(indices)).then(|| {
        (0..vertices.len().div_ceil(BOUNDS_BLOCK))
            .map(|b| bounds_of_block(vertices, b))
            .collect()
    })
}

fn bounds_of_block(vertices: &[FaceVertex], b: usize) -> (Vec3, Vec3) {
    let r = b * BOUNDS_BLOCK..((b + 1) * BOUNDS_BLOCK).min(vertices.len());
    vertices[r].iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), v| {
            let p = Vec3::from(v.position);
            (lo.min(p), hi.max(p))
        },
    )
}

fn union_of(blocks: &[(Vec3, Vec3)]) -> (Vec3, Vec3) {
    blocks.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), (a, b)| (lo.min(*a), hi.max(*b)),
    )
}

/// The bounds of the vertices `indices` reference.
fn bounds_of(vertices: &[FaceVertex], indices: &[u32]) -> (Vec3, Vec3) {
    let position = |i: u32| Vec3::from(vertices[i as usize].position);
    indices.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), &i| (lo.min(position(i)), hi.max(position(i))),
    )
}

/// The index side of [`FaceBuffers`]: the indices with the opaque triangles first,
/// where the translucent ones start, the translucent triangles with their centroids,
/// and the opaque bits (sequential indices only).
struct IndexSide {
    sorted: Vec<u32>,
    split: u32,
    translucent_tris: Vec<([u32; 3], Vec3)>,
    opaque_bits: Option<Vec<u64>>,
}

impl IndexSide {
    fn new(vertices: &[FaceVertex], indices: &[u32]) -> Self {
        debug_assert_eq!(indices.len() % 3, 0, "indices must form triangles");
        // Opacity per triangle, also kept as bits when the indices are sequential.
        let mut bits = sequential(indices).then(|| vec![0u64; (indices.len() / 3).div_ceil(64)]);
        let (mut opaque, mut translucent): (Vec<&[u32; 3]>, Vec<&[u32; 3]>) = Default::default();
        for (t, tri) in indices.as_chunks::<3>().0.iter().enumerate() {
            if tri.iter().all(|&i| vertices[i as usize].opaque()) {
                opaque.push(tri);
                if let Some(bits) = &mut bits {
                    bits[t / 64] |= 1 << (t % 64);
                }
            } else {
                translucent.push(tri);
            }
        }
        let opaque_bits = bits;
        let position = |i: u32| Vec3::from(vertices[i as usize].position);
        let translucent_tris = translucent
            .iter()
            .map(|t| {
                let c = (position(t[0]) + position(t[1]) + position(t[2])) / 3.0;
                ([t[0], t[1], t[2]], c)
            })
            .collect();
        let split = (opaque.len() * 3) as u32;
        let sorted: Vec<u32> = opaque
            .into_iter()
            .chain(translucent)
            .flatten()
            .copied()
            .collect();
        Self {
            sorted,
            split,
            translucent_tris,
            opaque_bits,
        }
    }
}

impl FaceBuffers {
    /// `None` when there is nothing to draw. A triangle is translucent when any of its
    /// vertices has a front or back alpha below 1.
    pub fn new(device: &wgpu::Device, vertices: &[FaceVertex], indices: &[u32]) -> Option<Self> {
        let side = IndexSide::new(vertices, indices);
        if side.sorted.is_empty() {
            return None;
        }
        let end = side.sorted.len() as u32;
        let blocks = block_bounds(vertices, indices);
        Some(Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("face vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }),
            indices: index_buffer(device, &side.sorted),
            opaque: 0..side.split,
            translucent: side.split..end,
            bounds: (blocks.as_deref()).map_or_else(|| bounds_of(vertices, indices), union_of),
            block_bounds: blocks,
            translucent_tris: side.translucent_tris,
            sorted_for: None,
            counts: (vertices.len(), indices.len()),
            opaque_bits: side.opaque_bits,
        })
    }

    /// Takes `vertices` and `indices` as [`FaceBuffers::new`] would, rewriting only
    /// the vertices in `changed` (the others must be those last given) and the
    /// indices when they or a triangle's opacity changed. Returns false, doing
    /// nothing, when the number of vertices differs or nothing would be drawn: the
    /// caller makes new buffers.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[FaceVertex],
        indices: &[u32],
        changed: &[std::ops::Range<usize>],
    ) -> bool {
        if self.counts.0 != vertices.len() || indices.is_empty() {
            return false;
        }
        // The same indices, and no triangle in the changed vertices turned opaque or
        // translucent: the index buffer stays.
        let mut keep = self.counts.1 == indices.len() && sequential(indices);
        let mut translucent_moved = false;
        if let (true, Some(bits)) = (keep, &self.opaque_bits) {
            let tris = indices.len() / 3;
            'check: for r in changed {
                for t in r.start / 3..r.end.div_ceil(3).min(tris) {
                    let was = bits[t / 64] & (1 << (t % 64)) != 0;
                    let opaque = vertices[3 * t..3 * t + 3].iter().all(FaceVertex::opaque);
                    if was != opaque {
                        keep = false;
                        break 'check;
                    }
                    translucent_moved |= !opaque;
                }
            }
        } else {
            keep = false;
        }
        let size = std::mem::size_of::<FaceVertex>();
        for r in changed {
            let r = r.start.min(vertices.len())..r.end.min(vertices.len());
            if !r.is_empty() {
                let bytes = bytemuck::cast_slice(&vertices[r.clone()]);
                queue.write_buffer(&self.vertices, (r.start * size) as u64, bytes);
            }
        }
        if keep {
            if translucent_moved {
                let position = |i: u32| Vec3::from(vertices[i as usize].position);
                for (t, c) in &mut self.translucent_tris {
                    *c = (position(t[0]) + position(t[1]) + position(t[2])) / 3.0;
                }
                self.sorted_for = None;
            }
        } else {
            let side = IndexSide::new(vertices, indices);
            if side.sorted.len() == self.counts.1 {
                queue.write_buffer(&self.indices, 0, bytemuck::cast_slice(&side.sorted));
            } else {
                self.indices = index_buffer(device, &side.sorted);
            }
            let end = side.sorted.len() as u32;
            self.opaque = 0..side.split;
            self.translucent = side.split..end;
            self.translucent_tris = side.translucent_tris;
            self.sorted_for = None;
            self.opaque_bits = side.opaque_bits;
            self.counts.1 = indices.len();
        }
        match (&mut self.block_bounds, keep) {
            (Some(blocks), true) => {
                // Only the blocks of changed vertices are bounded again.
                for r in changed {
                    let r = r.start.min(vertices.len())..r.end.min(vertices.len());
                    if !r.is_empty() {
                        let first = r.start / BOUNDS_BLOCK;
                        let touched = &mut blocks[first..=(r.end - 1) / BOUNDS_BLOCK];
                        for (k, b) in touched.iter_mut().enumerate() {
                            *b = bounds_of_block(vertices, first + k);
                        }
                    }
                }
                self.bounds = union_of(blocks);
            }
            _ => {
                self.block_bounds = block_bounds(vertices, indices);
                self.bounds = (self.block_bounds.as_deref())
                    .map_or_else(|| bounds_of(vertices, indices), union_of);
            }
        }
        true
    }

    /// Reorders the translucent triangles back to front for a camera at `eye` looking
    /// along `forward` (perspective: by centroid distance from the eye; parallel: by
    /// centroid depth along `forward`). Per-triangle centroid sorting, so intersecting
    /// or long overlapping triangles can still blend in the wrong order. Does nothing
    /// when the view is unchanged.
    pub fn sort_translucent(
        &mut self,
        queue: &wgpu::Queue,
        eye: Vec3,
        forward: Vec3,
        perspective: bool,
    ) {
        if self.translucent_tris.len() < 2 || self.sorted_for == Some((eye, forward, perspective)) {
            return;
        }
        self.sorted_for = Some((eye, forward, perspective));
        let key = |c: Vec3| {
            if perspective {
                c.distance_squared(eye)
            } else {
                c.dot(forward)
            }
        };
        self.translucent_tris
            .sort_by(|a, b| key(b.1).total_cmp(&key(a.1)));
        let sorted: Vec<u32> = self.translucent_tris.iter().flat_map(|t| t.0).collect();
        queue.write_buffer(
            &self.indices,
            u64::from(self.translucent.start) * 4,
            bytemuck::cast_slice(&sorted),
        );
    }
}
