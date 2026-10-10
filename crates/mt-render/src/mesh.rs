//! Shaded, two-sided faces.

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

/// One face vertex. Triangles are wound counter-clockwise around `normal` when seen
/// from the front side.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FaceVertex {
    pub position: [f32; 3],
    /// Front-face normal (unit). Back side uses the negated normal.
    pub normal: [f32; 3],
    /// sRGB + alpha of the front and back material.
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
                usage: wgpu::BufferUsages::INDEX,
            }),
            opaque: 0..split,
            translucent: split..end,
        })
    }
}
