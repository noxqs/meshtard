//! Viewport effects of the Styles panel: screen-space ambient occlusion and
//! watermarks (see [`crate::Viewport`]).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::viewport::{COLOR_FORMAT, DEPTH_FORMAT};

/// Styles > Face > Ambient Occlusion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbientOcclusion {
    /// 0..1.
    pub intensity: f32,
    /// How far occluders reach, in world units (inches).
    pub distance: f32,
    /// sRGB occlusion color (black for plain darkening).
    pub color: [f32; 3],
}

/// Where a watermark goes on the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WatermarkPlacement {
    /// Over the whole viewport; with `lock_aspect`, fit inside it centered.
    Stretched { lock_aspect: bool },
    /// Repeated from the top left at `scale` times its size in logical points.
    Tiled { scale: f32 },
    /// One copy at `scale` times its size in logical points, at `anchor` (0..1
    /// across and down the free space; 0, 0 is the top left corner).
    Positioned { anchor: [f32; 2], scale: f32 },
}

/// A watermark image.
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    /// PNG, JPEG or BMP bytes; decoded images are kept for the same allocation.
    pub image: Arc<[u8]>,
    /// Drawn over the model (else under it, over the background).
    pub overlay: bool,
    /// 0..1.
    pub opacity: f32,
    pub placement: WatermarkPlacement,
}

/// The quad of a watermark of `image` pixels on a `viewport` of physical pixels:
/// top left and bottom right corners in pixels and the texture coordinates there.
pub fn watermark_rect(
    placement: WatermarkPlacement,
    image: [f32; 2],
    viewport: [f32; 2],
    pixels_per_point: f32,
) -> ([f32; 4], [f32; 4]) {
    let [vw, vh] = viewport;
    let [iw, ih] = [image[0].max(1.0), image[1].max(1.0)];
    match placement {
        WatermarkPlacement::Stretched { lock_aspect: false } => {
            ([0.0, 0.0, vw, vh], [0.0, 0.0, 1.0, 1.0])
        }
        WatermarkPlacement::Stretched { lock_aspect: true } => {
            let k = (vw / iw).min(vh / ih);
            let (w, h) = (iw * k, ih * k);
            let (x, y) = ((vw - w) / 2.0, (vh - h) / 2.0);
            ([x, y, x + w, y + h], [0.0, 0.0, 1.0, 1.0])
        }
        WatermarkPlacement::Tiled { scale } => {
            let k = scale.max(0.01) * pixels_per_point;
            ([0.0, 0.0, vw, vh], [0.0, 0.0, vw / (iw * k), vh / (ih * k)])
        }
        WatermarkPlacement::Positioned { anchor, scale } => {
            let k = scale.max(0.01) * pixels_per_point;
            let (w, h) = (iw * k, ih * k);
            let x = (vw - w) * anchor[0].clamp(0.0, 1.0);
            let y = (vh - h) * anchor[1].clamp(0.0, 1.0);
            ([x, y, x + w, y + h], [0.0, 0.0, 1.0, 1.0])
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AoUniform {
    inv_view_proj: [[f32; 4]; 4],
    viewport: [f32; 4],
    params: [f32; 4],
    color: [f32; 4],
}

/// The ambient occlusion pass: darkens the color target from the
/// depth of the opaque faces (both multisampled unless the viewport draws without).
pub(crate) struct AoPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
}

impl AoPass {
    pub(crate) fn new(device: &wgpu::Device, samples: u32) -> Self {
        // The depth is multisampled unless the viewport draws without multisampling
        // (`textureLoad` takes a sample index or a mip level as its last argument:
        // 0 either way).
        let source = include_str!("ao.wgsl");
        let source = if samples > 1 {
            std::borrow::Cow::Borrowed(source)
        } else {
            std::borrow::Cow::Owned(
                source.replace("texture_depth_multisampled_2d", "texture_depth_2d"),
            )
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ao.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ambient occlusion"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: samples > 1,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ambient occlusion"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ambient occlusion"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("ao_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("ao_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ambient occlusion"),
            size: std::mem::size_of::<AoUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniform,
        }
    }

    /// Records the pass: `color` is the color target drawn into (loaded and kept),
    /// `depth` the depth of what was drawn so far.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        view_proj: Mat4,
        size: [u32; 2],
        pixels_per_point: f32,
        ao: &AmbientOcclusion,
    ) {
        let u = AoUniform {
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            viewport: [size[0] as f32, size[1] as f32, 0.0, 0.0],
            params: [
                ao.intensity.clamp(0.0, 1.0),
                ao.distance.max(1e-3),
                48.0 * pixels_per_point,
                0.0,
            ],
            color: [ao.color[0], ao.color[1], ao.color[2], 1.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ambient occlusion"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ambient occlusion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WmVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
}

/// A watermark's GPU texture.
struct WmTexture {
    bind_group: wgpu::BindGroup,
    size: [f32; 2],
}

/// Watermark images and the pipeline that draws them.
pub(crate) struct Watermarks {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    list: Vec<Watermark>,
    /// Decoded textures by image allocation (`None`: undecodable).
    textures: Vec<(Arc<[u8]>, Option<WmTexture>)>,
    /// Quads of the last [`Watermarks::prepare`]: buffer and per watermark its
    /// vertex range and texture index, under and over the model.
    vertices: Option<wgpu::Buffer>,
    under: Vec<(std::ops::Range<u32>, usize)>,
    over: Vec<(std::ops::Range<u32>, usize)>,
}

impl Watermarks {
    pub(crate) fn new(device: &wgpu::Device, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("watermark.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("watermark"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("watermark"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("watermark"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("wm_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WmVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("wm_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("watermark"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            list: Vec::new(),
            textures: Vec::new(),
            vertices: None,
            under: Vec::new(),
            over: Vec::new(),
        }
    }

    pub(crate) fn set(&mut self, list: &[Watermark]) {
        self.list = list.to_vec();
        // Drop textures no longer used.
        self.textures
            .retain(|(data, _)| list.iter().any(|w| Arc::ptr_eq(&w.image, data)));
    }

    /// Decodes and uploads new images, then builds the quads for a viewport of `size`
    /// physical pixels.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        pixels_per_point: f32,
    ) {
        self.under.clear();
        self.over.clear();
        self.vertices = None;
        if self.list.is_empty() {
            return;
        }
        for w in &self.list {
            if self.textures.iter().any(|(d, _)| Arc::ptr_eq(d, &w.image)) {
                continue;
            }
            let tex = crate::texture::TextureImage::decode(&w.image)
                .map(|img| self.upload(device, queue, &img));
            self.textures.push((w.image.clone(), tex));
        }
        let mut verts: Vec<WmVertex> = Vec::new();
        let viewport = [size[0] as f32, size[1] as f32];
        for w in &self.list {
            let Some(index) = self
                .textures
                .iter()
                .position(|(d, t)| Arc::ptr_eq(d, &w.image) && t.is_some())
            else {
                continue;
            };
            let tex = self.textures[index].1.as_ref().expect("checked");
            let ([x0, y0, x1, y1], [u0, v0, u1, v1]) =
                watermark_rect(w.placement, tex.size, viewport, pixels_per_point);
            let ndc = |x: f32, y: f32| [x / viewport[0] * 2.0 - 1.0, 1.0 - y / viewport[1] * 2.0];
            let v = |x, y, u, t| WmVertex {
                pos: ndc(x, y),
                uv: [u, t],
                opacity: w.opacity.clamp(0.0, 1.0),
            };
            let start = verts.len() as u32;
            verts.extend([
                v(x0, y0, u0, v0),
                v(x1, y0, u1, v0),
                v(x1, y1, u1, v1),
                v(x0, y0, u0, v0),
                v(x1, y1, u1, v1),
                v(x0, y1, u0, v1),
            ]);
            let range = start..verts.len() as u32;
            if w.overlay {
                self.over.push((range, index));
            } else {
                self.under.push((range, index));
            }
        }
        if !verts.is_empty() {
            self.vertices = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("watermarks"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            );
        }
    }

    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &crate::texture::TextureImage,
    ) -> WmTexture {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("watermark"),
                size: wgpu::Extent3d {
                    width: img.width,
                    height: img.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &img.rgba,
        );
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("watermark"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        WmTexture {
            bind_group,
            size: [img.width as f32, img.height as f32],
        }
    }

    /// Draws the watermarks over (`overlay`) or under the model.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, overlay: bool) {
        let Some(buffer) = &self.vertices else { return };
        let list = if overlay { &self.over } else { &self.under };
        if list.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, buffer.slice(..));
        for (range, index) in list {
            if let Some(t) = &self.textures[*index].1 {
                pass.set_bind_group(0, &t.bind_group, &[]);
                pass.draw(range.clone(), 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watermark_placements() {
        let vp = [200.0, 100.0];
        // Stretched over everything, or fit keeping the aspect ratio.
        let (r, _) = watermark_rect(
            WatermarkPlacement::Stretched { lock_aspect: false },
            [10.0, 10.0],
            vp,
            1.0,
        );
        assert_eq!(r, [0.0, 0.0, 200.0, 100.0]);
        let (r, _) = watermark_rect(
            WatermarkPlacement::Stretched { lock_aspect: true },
            [10.0, 10.0],
            vp,
            1.0,
        );
        assert_eq!(r, [50.0, 0.0, 150.0, 100.0]);
        // Tiled: texture coordinates count the repeats.
        let (_, uv) = watermark_rect(
            WatermarkPlacement::Tiled { scale: 0.5 },
            [20.0, 10.0],
            vp,
            2.0,
        );
        assert_eq!(uv, [0.0, 0.0, 10.0, 10.0]);
        // Positioned bottom right at double size.
        let (r, _) = watermark_rect(
            WatermarkPlacement::Positioned {
                anchor: [1.0, 1.0],
                scale: 2.0,
            },
            [20.0, 10.0],
            vp,
            1.0,
        );
        assert_eq!(r, [160.0, 80.0, 200.0, 100.0]);
    }
}
