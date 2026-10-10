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
}

impl FaceVertex {
    pub(crate) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<FaceVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4
        ],
    };

    fn opaque(&self) -> bool {
        self.front[3] >= 1.0 && self.back[3] >= 1.0
    }
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
    /// Translucent triangles and their centroids, for sorting back to front.
    translucent_tris: Vec<([u32; 3], Vec3)>,
    /// The view the translucent range was last sorted for.
    sorted_for: Option<(Vec3, Vec3, bool)>,
}

impl FaceBuffers {
    /// `None` when there is nothing to draw. A triangle is translucent when any of its
    /// vertices has a front or back alpha below 1.
    pub fn new(device: &wgpu::Device, vertices: &[FaceVertex], indices: &[u32]) -> Option<Self> {
        debug_assert_eq!(indices.len() % 3, 0, "indices must form triangles");
        let (opaque, translucent): (Vec<&[u32]>, Vec<&[u32]>) = indices
            .chunks_exact(3)
            .partition(|tri| tri.iter().all(|&i| vertices[i as usize].opaque()));
        if opaque.is_empty() && translucent.is_empty() {
            return None;
        }
        let position = |i: u32| Vec3::from(vertices[i as usize].position);
        let bounds = indices.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), &i| (lo.min(position(i)), hi.max(position(i))),
        );
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
        let end = sorted.len() as u32;
        Some(Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("face vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("face indices"),
                contents: bytemuck::cast_slice(&sorted),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            }),
            opaque: 0..split,
            translucent: split..end,
            bounds,
            translucent_tris,
            sorted_for: None,
        })
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
