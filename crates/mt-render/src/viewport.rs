//! Renders the 3D viewport into an offscreen texture.
//!
//! Offscreen so the same code path serves the app (shown as an egui image) and headless
//! tests (read back and compared).

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::{DVec2, DVec3, Mat4};
use mt_spec::DefaultModel;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::mesh::{FaceBuffers, FaceVertex};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// Axis half-length in inches. SketchUp's axes reach far past any normal model.
const AXIS_LENGTH: f32 = 1.0e6;
/// Dash length of the negative axis halves, in logical points.
const AXIS_DASH: f32 = 2.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    viewport: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    ground: [f32; 4],
    background: [f32; 4],
    /// xyz: unit direction towards the light, w: ambient fraction.
    light: [f32; 4],
    /// How far lines are moved towards the camera; see `pull_depth` in the shader. x: perspective
    /// radians per physical pixel, y: parallel NDC depth per physical pixel.
    line_depth: [f32; 4],
}

/// One line segment, drawn as a screen-space quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Segment {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub color: [f32; 4],
    /// Width and dash length in logical points; dash 0 means solid.
    pub style: [f32; 2],
}

/// Scene-independent display settings, from the model's style.
#[derive(Debug, Clone)]
pub struct Style {
    pub sky: Option<([f32; 3], [f32; 3])>,
    pub background: [f32; 3],
    pub ground: Option<[f32; 3]>,
    pub ground_from_below: bool,
    pub axes: Option<AxesStyle>,
    pub shading: ShadingStyle,
}

#[derive(Debug, Clone)]
pub struct AxesStyle {
    pub colors: [[f32; 3]; 3],
    pub dotted_negative: bool,
    pub width: f32,
}

/// Flat diffuse face shading: `color * (ambient + (1 - ambient) * max(0, n·light))`.
#[derive(Debug, Clone, Copy)]
pub struct ShadingStyle {
    pub ambient: f32,
    pub light: Light,
}

#[derive(Debug, Clone, Copy)]
pub enum Light {
    /// Follows the camera: the direction towards the camera, turned `yaw_deg` towards
    /// the camera's right and then `pitch_deg` up.
    Camera { yaw_deg: f32, pitch_deg: f32 },
    /// Fixed world direction towards the light.
    World([f32; 3]),
}

impl ShadingStyle {
    /// Unit world direction towards the light for this camera.
    pub fn light_dir(&self, camera: &Camera) -> DVec3 {
        match self.light {
            Light::Camera { yaw_deg, pitch_deg } => {
                let (yaw, pitch) = (
                    f64::from(yaw_deg).to_radians(),
                    f64::from(pitch_deg).to_radians(),
                );
                let horizontal = -camera.forward() * yaw.cos() + camera.right() * yaw.sin();
                (horizontal * pitch.cos() + camera.up() * pitch.sin()).normalize()
            }
            Light::World(dir) => DVec3::from(dir.map(f64::from)).normalize_or(DVec3::Z),
        }
    }
}

impl Style {
    pub fn from_spec(spec: &DefaultModel) -> Self {
        Self {
            sky: spec
                .sky
                .enabled
                .then_some((spec.sky.zenith, spec.sky.horizon)),
            background: spec.sky.background,
            ground: spec.ground.enabled.then_some(spec.ground.color),
            ground_from_below: spec.ground.show_from_below,
            axes: Some(AxesStyle {
                colors: [spec.axes.x, spec.axes.y, spec.axes.z],
                dotted_negative: spec.axes.dotted_negative,
                width: spec.axes.width_px,
            }),
            shading: ShadingStyle {
                ambient: spec.shading.ambient,
                light: if spec.shading.light_from_camera {
                    let [yaw_deg, pitch_deg] = spec.shading.light_offset_deg;
                    Light::Camera { yaw_deg, pitch_deg }
                } else {
                    Light::World(spec.shading.light_direction)
                },
            },
        }
    }
}

fn axis_segments(axes: &AxesStyle) -> Vec<Segment> {
    let mut out = Vec::with_capacity(6);
    for (i, color) in axes.colors.iter().enumerate() {
        let mut dir = [0.0; 3];
        dir[i] = AXIS_LENGTH;
        let neg = dir.map(|v| -v);
        let color = [color[0], color[1], color[2], 1.0];
        out.push(Segment {
            a: [0.0; 3],
            b: dir,
            color,
            style: [axes.width, 0.0],
        });
        let dash = if axes.dotted_negative { AXIS_DASH } else { 0.0 };
        out.push(Segment {
            a: [0.0; 3],
            b: neg,
            color,
            style: [axes.width, dash],
        });
    }
    out
}

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let texture = |label, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = texture("viewport msaa", COLOR_FORMAT, SAMPLES, attach);
        let depth = texture("viewport depth", DEPTH_FORMAT, SAMPLES, attach);
        let resolve = texture(
            "viewport color",
            COLOR_FORMAT,
            1,
            attach | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            size,
            msaa: msaa.create_view(&Default::default()),
            depth: depth.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            resolve,
        }
    }
}

pub struct Viewport {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    background: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    faces_opaque: wgpu::RenderPipeline,
    faces_translucent: wgpu::RenderPipeline,
    faces: Option<FaceBuffers>,
    axes: Option<wgpu::Buffer>,
    axes_count: u32,
    edges: Option<wgpu::Buffer>,
    edges_count: u32,
    style: Style,
    targets: Option<Targets>,
}

impl Viewport {
    pub fn new(device: &wgpu::Device, style: Style) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("viewport.wgsl"));
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        // Default primitive state: triangle list, counter-clockwise front (in y-up NDC,
        // which matches `FaceVertex` winding), no culling, so faces are two-sided.
        let pipeline =
            |label, vs, fs, buffers: &[Option<wgpu::VertexBufferLayout>], depth_write, blend| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(vs),
                        compilation_options: Default::default(),
                        buffers,
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(depth_write),
                        // Reverse Z: nearer is larger.
                        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: SAMPLES,
                        ..Default::default()
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: COLOR_FORMAT,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };

        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let background = pipeline("background", "bg_vs", "bg_fs", &[], false, alpha);
        let segment_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Segment>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2
            ],
        };
        let lines = pipeline(
            "lines",
            "line_vs",
            "line_fs",
            &[Some(segment_layout)],
            true,
            alpha,
        );
        let faces = [Some(FaceVertex::LAYOUT)];
        let faces_opaque = pipeline(
            "opaque faces",
            "face_vs",
            "face_fs",
            &faces,
            true,
            Some(wgpu::BlendState::REPLACE),
        );
        // No depth write: translucent faces neither hide each other nor what is
        // drawn after them. They are not sorted, so overlapping ones blend in
        // submission order.
        let faces_translucent = pipeline(
            "translucent faces",
            "face_vs",
            "face_fs",
            &faces,
            false,
            alpha,
        );

        let mut viewport = Self {
            globals,
            bind_group,
            background,
            lines,
            faces_opaque,
            faces_translucent,
            faces: None,
            axes: None,
            axes_count: 0,
            edges: None,
            edges_count: 0,
            style: style.clone(),
            targets: None,
        };
        viewport.set_style(device, style);
        viewport
    }

    /// Replaces the model's edges.
    pub fn set_edges(&mut self, device: &wgpu::Device, segments: &[Segment]) {
        self.edges_count = segments.len() as u32;
        self.edges = (!segments.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("model edges"),
                contents: bytemuck::cast_slice(segments),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
    }

    /// Replaces the model's faces: an indexed triangle list. Empty slices clear.
    /// Triangles with any alpha below 1 are drawn translucent, after everything else.
    pub fn set_faces(&mut self, device: &wgpu::Device, vertices: &[FaceVertex], indices: &[u32]) {
        self.faces = FaceBuffers::new(device, vertices, indices);
    }

    pub fn set_style(&mut self, device: &wgpu::Device, style: Style) {
        let segments = style.axes.as_ref().map(axis_segments).unwrap_or_default();
        self.axes_count = segments.len() as u32;
        self.axes = (!segments.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("axes"),
                contents: bytemuck::cast_slice(&segments),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        self.style = style;
    }

    /// Renders one frame at `size` physical pixels and returns the resolved color texture.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &Camera,
        size: [u32; 2],
        pixels_per_point: f32,
    ) -> &wgpu::Texture {
        let size = [size[0].max(1), size[1].max(1)];
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(Targets::new(device, size));
        }
        let targets = self.targets.as_ref().expect("targets just created");

        let aspect = f64::from(size[0]) / f64::from(size[1]);
        let view_proj = camera.view_proj(aspect).as_mat4();
        let style = &self.style;
        let rgb_w = |c: [f32; 3], w: bool| [c[0], c[1], c[2], f32::from(u8::from(w))];
        let (zenith, horizon) = style.sky.unwrap_or(([0.0; 3], [0.0; 3]));
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: Mat4::inverse(&view_proj).to_cols_array_2d(),
            viewport: [
                size[0] as f32,
                size[1] as f32,
                pixels_per_point,
                if camera.perspective {
                    camera.near() as f32
                } else {
                    0.0
                },
            ],
            sky_zenith: rgb_w(zenith, false),
            sky_horizon: rgb_w(horizon, style.sky.is_some()),
            ground: rgb_w(style.ground.unwrap_or_default(), style.ground.is_some()),
            background: rgb_w(style.background, style.ground_from_below),
            light: {
                let dir = style.shading.light_dir(camera).as_vec3();
                [dir.x, dir.y, dir.z, style.shading.ambient]
            },
            line_depth: if camera.perspective {
                let rad_per_px =
                    2.0 * (camera.fov_deg.to_radians() / 2.0).tan() / f64::from(size[1]);
                [rad_per_px as f32, 0.0, 0.0, 0.0]
            } else {
                let viewport = DVec2::new(f64::from(size[0]), f64::from(size[1]));
                let depth_per_unit = camera.projection(aspect).z_axis.z.abs();
                let depth_per_px = depth_per_unit * camera.units_per_px(viewport);
                [0.0, depth_per_px as f32, 0.0, 0.0]
            },
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);
            // Order: opaque faces, then edges and axes, which are pulled towards the
            // camera so lines on a face win the depth test and lines behind it fail.
            // Translucent faces last (without depth write) so they blend over
            // everything behind them, including the edges they cover; edges on their
            // own boundary are in front of them thanks to the same pull.
            if let Some(faces) = &self.faces {
                pass.set_vertex_buffer(0, faces.vertices.slice(..));
                pass.set_index_buffer(faces.indices.slice(..), wgpu::IndexFormat::Uint32);
                if !faces.opaque.is_empty() {
                    pass.set_pipeline(&self.faces_opaque);
                    pass.draw_indexed(faces.opaque.clone(), 0, 0..1);
                }
            }
            if let Some(edges) = &self.edges {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, edges.slice(..));
                pass.draw(0..6, 0..self.edges_count);
            }
            if let Some(axes) = &self.axes {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, axes.slice(..));
                pass.draw(0..6, 0..self.axes_count);
            }
            if let Some(faces) = self.faces.as_ref().filter(|f| !f.translucent.is_empty()) {
                pass.set_pipeline(&self.faces_translucent);
                pass.set_vertex_buffer(0, faces.vertices.slice(..));
                pass.set_index_buffer(faces.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(faces.translucent.clone(), 0, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
        &targets.resolve
    }

    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.resolve_view)
    }
}
