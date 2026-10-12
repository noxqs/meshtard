//! Renders the 3D viewport into an offscreen texture.
//!
//! Offscreen so the same code path serves the app (shown as an egui image) and headless
//! tests (read back and compared).

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::{DVec2, DVec3, Mat4, Vec3};
use mt_spec::DefaultModel;
use wgpu::util::DeviceExt;

use crate::camera::{Camera, look_at_rh, orthographic_rh};
use crate::effects::{AmbientOcclusion, AoPass, Watermark, Watermarks};
use crate::environment::{EnvImage, EnvPass, EnvView};
use crate::mesh::{FaceBuffers, FaceVertex};
use crate::section::SectionCut;
use crate::texture::{TextureSource, Textures};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Multisample count when none is chosen (Preferences > Graphics).
pub const DEFAULT_SAMPLES: u32 = 4;
/// Axis half-length in inches. SketchUp's axes reach far past any normal model.
const AXIS_LENGTH: f32 = 1.0e6;
/// Dash length of the negative axis halves, in logical points.
const AXIS_DASH: f32 = 2.0;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Shadow map resolution, fitted to the bounds of all faces. One map for the whole
/// model, so detail drops as the model grows (SketchUp's own quality is unknown).
const SHADOW_SIZE: u32 = 2048;

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
    /// xyz: camera eye (perspective) or direction towards the camera (parallel); w: 1 for
    /// perspective.
    eye: [f32; 4],
    /// x: profile edge width in logical points, y: 1 when profiles are drawn.
    profile: [f32; 4],
    /// World to shadow map clip space.
    light_view_proj: [[f32; 4]; 4],
    /// Section plane as (normal, -normal·point - tolerance); (0, 0, 0, -1) when off.
    section: [f32; 4],
    /// xyz: unit direction towards the sun (zero below the horizon), w: 1 when shadow
    /// lighting is on.
    sun: [f32; 4],
    /// x: light, y: dark, z: 1 when faces receive shadows, w: 1 when the ground does.
    shadow: [f32; 4],
    /// x: world size of a shadow texel, y: 1 / shadow map size.
    shadow_texel: [f32; 4],
    /// x: face coloring (0 material, 1 background color unshaded, 2 monochrome),
    /// y: opacity cap (1, or [`XRAY_OPACITY`] in X-ray), z: 1 when textures show,
    /// w: number of texture layers.
    face_style: [f32; 4],
    /// Monochrome front and back colors (rgb, w unused).
    mono_front: [f32; 4],
    mono_back: [f32; 4],
    /// Edge Style: x: 1 when Edges is on (off draws only the profiles of model edges),
    /// y: depth cue width in logical points (0 = off), z: extension in logical points
    /// (0 = off), w: back edge dash in logical points (0 = no back edges).
    edge_style: [f32; 4],
    /// View depth of a world point p is dot(xyz, p) + w.
    view_axis: [f32; 4],
    /// x, y: view depth of the model's nearest and farthest point (bounding sphere),
    /// z, w: fog start and end view depth (z >= w: no fog).
    depth_range: [f32; 4],
    /// rgb: fog color, w: 1 to fade towards the background (sky, ground) instead.
    fog_color: [f32; 4],
    /// x: endpoint length in logical points (0 = off), y: 1 for jitter, z: 1 when
    /// transparency is off, w: ground opacity.
    extra: [f32; 4],
    /// x: sky gradient exponent (horizon blend).
    extra2: [f32; 4],
    /// Selected faces' dot color.
    select_color: [f32; 4],
}

/// Back edge dash length in logical points (SketchUp's dash pattern is unverified).
pub const BACK_EDGE_DASH: f32 = 3.0;

/// Face opacity in X-ray mode. SketchUp's X-ray looks like about 50% (unverified).
pub const XRAY_OPACITY: f32 = 0.5;

/// SketchUp's View > Face Style. Set with [`Viewport::set_face_style`]; X-ray is a
/// separate toggle that combines with every style except [`FaceStyle::Wireframe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FaceStyle {
    /// Edges only. Faces are not drawn at all (no color, no depth), so every edge is
    /// visible. Profiles are drawn at normal edge width and there is no lighting or
    /// shadows (SketchUp hides profiles in wireframe as far as known; unverified).
    /// Selected faces show no dots, since there is no face to put them on.
    Wireframe,
    /// Faces filled with the style's background color, unshaded, hiding what is behind
    /// them. No shadows. Material opacity still applies (unverified vs SketchUp).
    HiddenLine,
    /// Material colors with shading (the default).
    #[default]
    Shaded,
    /// Like [`FaceStyle::Shaded`], with the textures of [`Viewport::set_textures`].
    ShadedWithTextures,
    /// Every face in the monochrome front/back colors (see
    /// [`Viewport::set_monochrome_colors`]) with shading, ignoring material colors
    /// but keeping material opacity (unverified vs SketchUp).
    Monochrome,
}

/// SketchUp's Shadows panel. Draft: the lighting and shadow look only roughly
/// follow SketchUp (see [`Viewport::set_shadows`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadows {
    /// Direction from the ground towards the sun, e.g. from [`crate::sun_direction`].
    /// Need not be unit length. At or below the horizon (z <= 0) nothing casts shadows.
    pub sun_dir: [f32; 3],
    /// The panel's Light and Dark sliders (0..100) mapped to 0..1.
    pub light: f32,
    pub dark: f32,
    /// "On faces": faces receive shadows.
    pub on_faces: bool,
    /// "On ground": the ground plane z = 0 receives shadows.
    pub on_ground: bool,
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
    /// Front normals of the (first) two faces using the edge, for profile detection.
    pub n1: [f32; 3],
    pub n2: [f32; 3],
    /// Profile rule: [`Segment::NO_PROFILE`], [`Segment::SILHOUETTE`] or [`Segment::BORDER`].
    pub profile: f32,
    /// [`Segment::MODEL_EDGE`] or 0.
    pub flags: u32,
}

impl Segment {
    /// Never drawn as a profile (axes, guides, edges without faces).
    pub const NO_PROFILE: f32 = 0.0;
    /// A profile where one face turns towards the camera and the other away.
    pub const SILHOUETTE: f32 = 1.0;
    /// Always a profile: the edge borders a single face.
    pub const BORDER: f32 = 2.0;
    /// A model edge: the Edge Style settings of [`Style`] (Edges, Back Edges, Depth Cue,
    /// Extension) apply to it. Guides, axes, section cut lines and overlays leave it off.
    pub const MODEL_EDGE: u32 = 1;

    pub fn line(a: [f32; 3], b: [f32; 3], color: [f32; 4], style: [f32; 2]) -> Self {
        Self {
            a,
            b,
            color,
            style,
            ..Self::zeroed()
        }
    }
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
    /// Profile edge width in logical points, or `None` when profiles are off.
    pub profiles: Option<f32>,
    /// Edge Style > Edges. Off: of the [`Segment::MODEL_EDGE`] segments only profiles
    /// are drawn (none when `profiles` is `None` too).
    pub edges: bool,
    /// Edge Style > Back Edges: model edges hidden behind faces are drawn dashed
    /// ([`BACK_EDGE_DASH`]).
    pub back_edges: bool,
    /// Edge Style > Depth Cue: the width in logical points of the model's nearest edges;
    /// the width falls linearly to 1 at the farthest (see [`depth_cue_width`]).
    pub depth_cue: Option<f32>,
    /// Edge Style > Extension: model edges overshoot both ends by this many logical
    /// points (see [`extend_segment`]).
    pub extension: Option<f32>,
    pub fog: Option<Fog>,
    /// Edge Settings > Endpoints: model edge ends drawn wider over this many logical
    /// points.
    pub endpoints: Option<f32>,
    /// Edge Settings > Jitter: model edges drawn as two slightly offset strokes.
    pub jitter: bool,
    /// Face Settings > transparency.
    pub transparency: Transparency,
    /// Face Settings > Ambient Occlusion.
    pub ambient_occlusion: Option<AmbientOcclusion>,
    /// Ground opacity (1 - the Background panel's transparency). The ground is drawn
    /// as the plane z = 0 after the opaque faces, so it hides what is below it unless
    /// translucent.
    pub ground_opacity: f32,
    /// Sky gradient: the sky is `mix(horizon, zenith, elevation^horizon_blend)`.
    pub horizon_blend: f32,
    /// Watermarks, bottom first.
    pub watermarks: Vec<Watermark>,
    /// Selected faces' dot color (sRGB).
    pub selection_color: [f32; 3],
}

/// Face Settings > transparency: off (translucent materials drawn opaque), Faster
/// (translucent faces in model order) or Nicer (sorted back to front every frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transparency {
    Off,
    Faster,
    #[default]
    Nicer,
}

/// View > Fog: faces and model edges fade linearly towards the fog color from `start`
/// to `end`, both fractions of the model's view depth range (0 at the near side of the
/// bounding sphere of faces and model edges, 1 at the far side), so the fog spans the
/// model however far the camera is. SketchUp's distance scale is unverified.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    pub start: f32,
    pub end: f32,
    /// sRGB; `None` fades towards the background behind the point (sky, ground or
    /// background color).
    pub color: Option<[f32; 3]>,
}

/// Depth cue edge width: `max_width` at view depth `near` falling linearly to 1 at
/// `far` (clamped). Mirrors `depth_cue_width` in the shader.
pub fn depth_cue_width(depth: f32, near: f32, far: f32, max_width: f32) -> f32 {
    let t = ((depth - near) / (far - near).max(1e-6)).clamp(0.0, 1.0);
    max_width + (1.0 - max_width) * t
}

/// Fog amount (0 clear, 1 fully fogged) at view depth `depth` for fog from `start` to
/// `end`. Mirrors `fog_amount` in the shader.
pub fn fog_amount(depth: f32, start: f32, end: f32) -> f32 {
    if end <= start {
        return f32::from(u8::from(depth >= end));
    }
    ((depth - start) / (end - start)).clamp(0.0, 1.0)
}

/// The screen segment `a`-`b` lengthened by `by` at both ends (Extension). A
/// degenerate segment is returned unchanged. Mirrors the extension in `line_vs`.
pub fn extend_segment(a: glam::Vec2, b: glam::Vec2, by: f32) -> (glam::Vec2, glam::Vec2) {
    let d = b - a;
    let len = d.length();
    if len < 1e-6 {
        return (a, b);
    }
    let dir = d / len;
    (a - dir * by, b + dir * by)
}

#[derive(Debug, Clone)]
pub struct AxesStyle {
    pub colors: [[f32; 3]; 3],
    pub dotted_negative: bool,
    pub width: f32,
    /// Where the axes cross and their unit red, green and blue directions (Axes tool).
    pub origin: [f32; 3],
    pub dirs: [[f32; 3]; 3],
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
                origin: [0.0; 3],
                dirs: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            }),
            // Profile width 2 matches item 1007 of SketchUp's default style seen in OpenSKP's
            // fixtures (unverified that 1007 is the profile width).
            profiles: Some(2.0),
            edges: true,
            back_edges: false,
            depth_cue: None,
            extension: None,
            fog: None,
            endpoints: None,
            jitter: false,
            transparency: Transparency::Nicer,
            ambient_occlusion: None,
            ground_opacity: 1.0,
            horizon_blend: 0.5,
            watermarks: Vec::new(),
            selection_color: [0.0, 0.0, 1.0],
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

/// SketchUp's default front and back face colors, (255, 255, 255) and (164, 178, 187).
const DEFAULT_MONOCHROME: ([f32; 3], [f32; 3]) = (
    [1.0, 1.0, 1.0],
    [164.0 / 255.0, 178.0 / 255.0, 187.0 / 255.0],
);

fn axis_segments(axes: &AxesStyle) -> Vec<Segment> {
    let mut out = Vec::with_capacity(6);
    let o = axes.origin;
    for (color, d) in axes.colors.iter().zip(axes.dirs) {
        let at = |s: f32| [0, 1, 2].map(|k| o[k] + d[k] * s);
        let color = [color[0], color[1], color[2], 1.0];
        out.push(Segment::line(o, at(AXIS_LENGTH), color, [axes.width, 0.0]));
        let dash = if axes.dotted_negative { AXIS_DASH } else { 0.0 };
        out.push(Segment::line(
            o,
            at(-AXIS_LENGTH),
            color,
            [axes.width, dash],
        ));
    }
    out
}

/// The sun's depth map and the bind group the color passes sample it through.
struct ShadowMap {
    size: u32,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

impl ShadowMap {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow map"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self {
            size,
            view,
            bind_group,
        }
    }
}

/// Orthographic sun projection fitted around the box `bounds`, and the world size of
/// one shadow map texel. `sun` is the unit direction towards the sun.
fn fit_sun(sun: Vec3, bounds: (Vec3, Vec3)) -> (Mat4, f32) {
    let (lo, hi) = (bounds.0.as_dvec3(), bounds.1.as_dvec3());
    let sun = sun.as_dvec3();
    let center = (lo + hi) * 0.5;
    let radius = (hi - lo).length().max(1.0);
    let up = if sun.z.abs() > 0.99 {
        DVec3::Y
    } else {
        DVec3::Z
    };
    let view = look_at_rh(center + sun * radius * 2.0, center, up);
    let (mut min, mut max) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for i in 0..8 {
        let corner = DVec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        );
        let p = view.transform_point3(corner);
        min = min.min(p);
        max = max.max(p);
    }
    // A couple of texels of margin, so PCF at the rim does not read past the casters.
    let extent = (max.x - min.x).max(max.y - min.y).max(1e-3);
    let pad = extent * 2.0 / f64::from(SHADOW_SIZE);
    let depth_pad = radius * 0.01;
    let proj = orthographic_rh(
        min.x - pad,
        max.x + pad,
        min.y - pad,
        max.y + pad,
        -max.z - depth_pad,
        -min.z + depth_pad,
    );
    (
        (proj * view).as_mat4(),
        ((extent + 2.0 * pad) / f64::from(SHADOW_SIZE)) as f32,
    )
}

/// The multisample counts (of 1, 2, 4, 8) the adapter supports for the viewport's
/// color and depth formats.
pub fn supported_samples(adapter: &wgpu::Adapter) -> Vec<u32> {
    let color = adapter.get_texture_format_features(COLOR_FORMAT).flags;
    let depth = adapter.get_texture_format_features(DEPTH_FORMAT).flags;
    [1, 2, 4, 8]
        .into_iter()
        .filter(|&n| n == 1 || (color.sample_count_supported(n) && depth.sample_count_supported(n)))
        .collect()
}

struct Targets {
    size: [u32; 2],
    /// The multisampled color target; `None` without anti-aliasing (one sample),
    /// when the pass draws straight into `resolve`.
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, size: [u32; 2], samples: u32) -> Self {
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
        let msaa = (samples > 1).then(|| texture("viewport msaa", COLOR_FORMAT, samples, attach));
        // Ambient occlusion reads the depth.
        let depth = texture(
            "viewport depth",
            DEPTH_FORMAT,
            samples,
            attach | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let resolve = texture(
            "viewport color",
            COLOR_FORMAT,
            1,
            attach | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            size,
            msaa: msaa.map(|t| t.create_view(&Default::default())),
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
    /// The ground plane, with and without depth writes (opaque / translucent).
    ground_opaque: wgpu::RenderPipeline,
    ground_blend: wgpu::RenderPipeline,
    ao: AoPass,
    watermarks: Watermarks,
    lines: wgpu::RenderPipeline,
    /// Lines the section plane does not clip (axes).
    overlay_lines: wgpu::RenderPipeline,
    back_lines: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_layout: wgpu::BindGroupLayout,
    shadow_sampler: wgpu::Sampler,
    /// 1x1 while shadows have never been on.
    shadow_map: ShadowMap,
    shadows: Option<Shadows>,
    /// Faces drawn into the shadow map instead of `faces`, when set.
    shadow_casters: Option<FaceBuffers>,
    section: Option<SectionCut>,
    face_style: FaceStyle,
    xray: bool,
    mono_colors: ([f32; 3], [f32; 3]),
    faces_opaque: wgpu::RenderPipeline,
    faces_translucent: wgpu::RenderPipeline,
    faces: Option<FaceBuffers>,
    textures: Textures,
    /// Environments panel: skydome and reflections (bind group 3).
    env: EnvPass,
    selection: wgpu::RenderPipeline,
    selected: Option<FaceBuffers>,
    /// Faces drawn live during Move, Rotate and Scale (see `set_preview_faces`).
    preview: Option<FaceBuffers>,
    /// Section fill: flat, unlit triangles in the section plane.
    fill_pipeline: wgpu::RenderPipeline,
    fill: Option<FaceBuffers>,
    axes: Option<wgpu::Buffer>,
    axes_count: u32,
    edges: Option<wgpu::Buffer>,
    edges_count: u32,
    /// Bounds of the [`Segment::MODEL_EDGE`] segments.
    edge_bounds: Option<(Vec3, Vec3)>,
    style: Style,
    targets: Option<Targets>,
    /// Multisample count of the color and depth targets and pipelines (1: none).
    samples: u32,
    /// Leave the background (sky, ground, background color) out and clear to
    /// transparent: [`Viewport::render_image`] for PNG export.
    transparent: bool,
}

impl Viewport {
    pub fn new(device: &wgpu::Device, style: Style) -> Self {
        Self::with_samples(device, style, DEFAULT_SAMPLES)
    }

    /// A viewport drawn with `samples` multisamples (1, 2, 4 or 8; 1 is no
    /// anti-aliasing). The device must support the count for [`COLOR_FORMAT`] and the
    /// depth format: see [`supported_samples`].
    pub fn with_samples(device: &wgpu::Device, style: Style, samples: u32) -> Self {
        let samples = samples.max(1);
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
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow map"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Lit when the receiver is not farther from the sun than the nearest caster.
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_map = ShadowMap::new(device, &shadow_layout, &shadow_sampler, 1);
        let textures = Textures::new(device);
        let env = EnvPass::new(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[
                Some(&layout),
                Some(&shadow_layout),
                Some(textures.layout()),
                Some(env.layout()),
            ],
            immediate_size: 0,
        });

        // Default primitive state: triangle list, counter-clockwise front (in y-up NDC,
        // which matches `FaceVertex` winding), no culling, so faces are two-sided.
        let pipeline_cmp = |label,
                            vs,
                            fs,
                            buffers: &[Option<wgpu::VertexBufferLayout>],
                            depth_write,
                            blend,
                            depth_compare| {
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
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
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
        // Reverse Z: nearer is larger.
        let pipeline =
            |label, vs, fs, buffers: &[Option<wgpu::VertexBufferLayout>], depth_write, blend| {
                pipeline_cmp(
                    label,
                    vs,
                    fs,
                    buffers,
                    depth_write,
                    blend,
                    wgpu::CompareFunction::GreaterEqual,
                )
            };

        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let background = pipeline("background", "bg_vs", "bg_fs", &[], false, alpha);
        let ground_opaque = pipeline("ground", "bg_vs", "ground_fs", &[], true, alpha);
        let ground_blend = pipeline(
            "translucent ground",
            "bg_vs",
            "ground_fs",
            &[],
            false,
            alpha,
        );
        let segment_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Segment>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2,
            4 => Float32x3, 5 => Float32x3, 6 => Float32, 7 => Uint32
            ],
        };
        let lines = pipeline(
            "lines",
            "line_vs",
            "line_fs",
            &[Some(segment_layout.clone())],
            true,
            alpha,
        );
        let overlay_lines = pipeline(
            "overlay lines",
            "line_vs",
            "overlay_line_fs",
            &[Some(segment_layout.clone())],
            true,
            alpha,
        );
        // Back edges: only the fragments behind what is already drawn (strictly
        // farther), so visible edges, which wrote their own depth, are not drawn twice.
        let back_lines = pipeline_cmp(
            "back edges",
            "back_line_vs",
            "back_line_fs",
            &[Some(segment_layout)],
            false,
            alpha,
            wgpu::CompareFunction::Less,
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
        // drawn after them. They are sorted back to front per triangle before drawing.
        let faces_translucent = pipeline(
            "translucent faces",
            "face_vs",
            "face_fs",
            &faces,
            false,
            alpha,
        );

        // Selected faces: a dot pattern over the face, at the face's own depth.
        let selection = pipeline(
            "selected faces",
            "face_vs",
            "select_fs",
            &faces,
            false,
            alpha,
        );

        // Section fill: opaque, unlit, not clipped (it lies in the plane).
        let fill_pipeline = pipeline(
            "section fill",
            "face_vs",
            "fill_fs",
            &faces,
            true,
            Some(wgpu::BlendState::REPLACE),
        );

        // Depth from the sun, regular Z (cleared to 1, nearer is smaller). Slope-scaled
        // bias against self-shadowing acne, on top of the normal offset in the shader.
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow map"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow map"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("shadow_vs"),
                compilation_options: Default::default(),
                buffers: &faces,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("shadow_fs"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });

        let mut viewport = Self {
            globals,
            bind_group,
            background,
            ground_opaque,
            ground_blend,
            ao: AoPass::new(device, samples),
            watermarks: Watermarks::new(device, samples),
            lines,
            overlay_lines,
            back_lines,
            shadow_pipeline,
            shadow_layout,
            shadow_sampler,
            shadow_map,
            shadows: None,
            shadow_casters: None,
            section: None,
            face_style: FaceStyle::Shaded,
            xray: false,
            mono_colors: DEFAULT_MONOCHROME,
            faces_opaque,
            faces_translucent,
            faces: None,
            textures,
            env,
            selection,
            selected: None,
            preview: None,
            fill_pipeline,
            fill: None,
            axes: None,
            axes_count: 0,
            edges: None,
            edges_count: 0,
            edge_bounds: None,
            style: style.clone(),
            targets: None,
            samples,
            transparent: false,
        };
        viewport.set_style(device, style);
        viewport
    }

    /// Replaces the model's edges.
    pub fn set_edges(&mut self, device: &wgpu::Device, segments: &[Segment]) {
        self.edge_bounds = segments
            .iter()
            .filter(|s| s.flags & Segment::MODEL_EDGE != 0)
            .flat_map(|s| [Vec3::from(s.a), Vec3::from(s.b)])
            .fold(None, |acc, p| match acc {
                None => Some((p, p)),
                Some((lo, hi)) => Some((p.min(lo), p.max(hi))),
            });
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

    /// Faces that cast shadows in place of the drawn ones (components whose "Shadows
    /// face sun" turns them toward the sun for their shadow). Empty slices clear: the
    /// drawn faces cast.
    pub fn set_shadow_casters(
        &mut self,
        device: &wgpu::Device,
        vertices: &[FaceVertex],
        indices: &[u32],
    ) {
        self.shadow_casters = FaceBuffers::new(device, vertices, indices);
    }

    /// The textures [`FaceVertex::texture`] indexes, decoded (PNG, JPEG, BMP) and
    /// uploaded at the next render when the list changed. Decoded images are kept for
    /// sources passed again (same `data` allocation and tint). Undecodable images draw
    /// white; past the device's array layer limit textures are not shown.
    pub fn set_textures(&mut self, textures: &[TextureSource]) {
        self.textures.set(textures);
    }

    /// Texture filtering (Model Info > Rendering > Use Anti-Aliased Textures): on is
    /// linear with mipmaps, off is nearest texel.
    pub fn set_smooth_textures(&mut self, device: &wgpu::Device, smooth: bool) {
        self.textures.set_smooth(device, smooth);
    }

    /// The Environments panel's environment and how it is used; `None` draws the
    /// style's sky and ground and no reflections. The image is uploaded when it
    /// changes (compared by pointer).
    pub fn set_environment(&mut self, env: Option<(std::sync::Arc<EnvImage>, EnvView)>) {
        self.env.set(env);
    }

    /// Extra faces drawn like the model's (the targets of Move, Rotate and Scale at
    /// their transformed position while the tool runs); empty slices clear them.
    pub fn set_preview_faces(
        &mut self,
        device: &wgpu::Device,
        vertices: &[FaceVertex],
        indices: &[u32],
    ) {
        self.preview = FaceBuffers::new(device, vertices, indices);
    }

    /// Faces shown as selected (SketchUp's blue dot pattern); empty slices clear it.
    pub fn set_selected_faces(
        &mut self,
        device: &wgpu::Device,
        vertices: &[FaceVertex],
        indices: &[u32],
    ) {
        self.selected = FaceBuffers::new(device, vertices, indices);
    }

    /// Sets or clears the section plane. Faces (including selected ones) and model
    /// edges on the side the normal points to are discarded per fragment, and do not
    /// cast shadows; axes are not clipped. Geometry within a small tolerance of the plane
    /// is kept so cut edges from [`crate::section_lines`] lying in it stay visible. The
    /// cut is not capped here; see [`Viewport::set_section_fill`].
    pub fn set_section(&mut self, plane: Option<SectionCut>) {
        self.section = plane;
    }

    /// Sets the section fill: triangles (e.g. from [`crate::section_fill`]) drawn in a
    /// flat, unlit `color` (sRGB) with depth, after the opaque faces, unclipped by the
    /// section plane. Not drawn in [`FaceStyle::Wireframe`]. Empty clears it.
    pub fn set_section_fill(
        &mut self,
        device: &wgpu::Device,
        triangles: &[[[f32; 3]; 3]],
        color: [f32; 3],
    ) {
        let c = [color[0], color[1], color[2], 1.0];
        let vertices: Vec<FaceVertex> = triangles
            .iter()
            .flatten()
            .map(|&p| FaceVertex::new(p, [0.0, 0.0, 1.0], c, c))
            .collect();
        let indices: Vec<u32> = (0..vertices.len() as u32).collect();
        self.fill = FaceBuffers::new(device, &vertices, &indices);
    }

    /// Sets or clears sun shadows. While set, faces are shaded by the sun instead of
    /// the style's light: `dark + light * max(0, n·sun) * visibility`, clamped to 1,
    /// with `visibility` from a shadow map when `on_faces` (1 otherwise). With the sun
    /// at or below the horizon everything gets `dark` alone and nothing casts shadows.
    /// Shadowed ground keeps the ratio a horizontal face would have. A draft: SketchUp's
    /// exact formula, shadow softness and bias are not known.
    pub fn set_shadows(&mut self, shadows: Option<Shadows>) {
        self.shadows = shadows;
    }

    /// Sets the face style and X-ray. X-ray draws every face at most
    /// [`XRAY_OPACITY`] opaque (faces with lower material opacity keep theirs), without
    /// depth, after the edges, so edges behind faces show through dimmed. It has no
    /// effect in [`FaceStyle::Wireframe`]. Shadows (when set) are only drawn in the
    /// shaded styles; in X-ray faces still cast them as their material says.
    pub fn set_face_style(&mut self, style: FaceStyle, xray: bool) {
        self.face_style = style;
        self.xray = xray;
    }

    /// Front and back colors (sRGB 0..1) of [`FaceStyle::Monochrome`]. Defaults to
    /// SketchUp's default front and back face colors, white and (164, 178, 187); the
    /// app should pass the model's front/back colors.
    pub fn set_monochrome_colors(&mut self, front: [f32; 3], back: [f32; 3]) {
        self.mono_colors = (front, back);
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
        self.watermarks.set(&style.watermarks);
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
            self.targets = Some(Targets::new(device, size, self.samples));
        }

        let face_style = self.face_style;
        let wireframe = face_style == FaceStyle::Wireframe;
        let xray = self.xray && !wireframe;
        // Shadows (and sun lighting) only in the shaded styles.
        let shadows = self.shadows.filter(|_| {
            matches!(
                face_style,
                FaceStyle::Shaded | FaceStyle::ShadedWithTextures | FaceStyle::Monochrome
            )
        });
        // Sun: lighting whenever shadows are set, a shadow map only when it is up and
        // something receives shadows.
        let sun = shadows
            .map(|s| Vec3::from(s.sun_dir).normalize_or_zero())
            .filter(|d| d.z > 0.0);
        let shadow_fit = match (shadows, sun, &self.faces) {
            (Some(s), Some(sun), Some(faces)) if s.on_faces || s.on_ground => {
                Some(fit_sun(sun, faces.bounds))
            }
            _ => None,
        };
        if shadow_fit.is_some() && self.shadow_map.size != SHADOW_SIZE {
            self.shadow_map = ShadowMap::new(
                device,
                &self.shadow_layout,
                &self.shadow_sampler,
                SHADOW_SIZE,
            );
        }
        self.textures.upload(device, queue);
        self.env.prepare(device, queue);
        if let Some(faces) = self
            .faces
            .as_mut()
            .filter(|_| self.style.transparency == Transparency::Nicer || self.xray)
        {
            let eye = camera.eye.as_vec3();
            let forward = camera.forward().as_vec3();
            faces.sort_translucent(queue, eye, forward, camera.perspective);
        }
        let targets = self.targets.as_ref().expect("targets just created");

        let aspect = f64::from(size[0]) / f64::from(size[1]);
        let view_proj = camera.view_proj(aspect).as_mat4();
        let style = &self.style;
        // View depth range of the model (faces and model edges), for depth cue and fog.
        let forward = camera.forward().as_vec3();
        let eye = camera.eye.as_vec3();
        let bounds = [self.faces.as_ref().map(|f| f.bounds), self.edge_bounds]
            .into_iter()
            .flatten()
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        let (near, far) = bounds.map_or((0.0, 1.0), |(lo, hi)| {
            let center = forward.dot((lo + hi) * 0.5 - eye);
            let radius = (hi - lo).length() * 0.5;
            let near = center - radius;
            let near = if camera.perspective {
                near.max(0.0)
            } else {
                near
            };
            (near, (center + radius).max(near + 1e-3))
        });
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
            eye: if camera.perspective {
                let e = camera.eye.as_vec3();
                [e.x, e.y, e.z, 1.0]
            } else {
                let d = -camera.forward().as_vec3();
                [d.x, d.y, d.z, 0.0]
            },
            profile: [
                style.profiles.unwrap_or(0.0),
                f32::from(u8::from(style.profiles.is_some() && !wireframe)),
                0.0,
                0.0,
            ],
            light_view_proj: shadow_fit
                .map_or(Mat4::IDENTITY, |(m, _)| m)
                .to_cols_array_2d(),
            section: self
                .section
                .and_then(|cut| {
                    let n = Vec3::from(cut.normal).try_normalize()?;
                    let p = Vec3::from(cut.point);
                    // Keep what is within float noise of the plane (cut edges lie in it).
                    let tolerance = 1e-5 * (1.0 + p.abs().max_element());
                    Some([n.x, n.y, n.z, -n.dot(p) - tolerance])
                })
                .unwrap_or([0.0, 0.0, 0.0, -1.0]),
            sun: {
                let d = sun.unwrap_or(Vec3::ZERO);
                [d.x, d.y, d.z, f32::from(u8::from(shadows.is_some()))]
            },
            shadow: {
                let s = shadows.unwrap_or(Shadows {
                    sun_dir: [0.0; 3],
                    light: 0.0,
                    dark: 0.0,
                    on_faces: false,
                    on_ground: false,
                });
                let mapped = shadow_fit.is_some();
                [
                    s.light.clamp(0.0, 1.0),
                    s.dark.clamp(0.0, 1.0),
                    f32::from(u8::from(mapped && s.on_faces)),
                    f32::from(u8::from(mapped && s.on_ground)),
                ]
            },
            shadow_texel: [
                shadow_fit.map_or(0.0, |(_, texel)| texel),
                1.0 / self.shadow_map.size as f32,
                0.0,
                0.0,
            ],
            face_style: [
                match face_style {
                    FaceStyle::HiddenLine => 1.0,
                    FaceStyle::Monochrome => 2.0,
                    _ => 0.0,
                },
                if xray { XRAY_OPACITY } else { 1.0 },
                f32::from(u8::from(
                    face_style == FaceStyle::ShadedWithTextures && self.textures.layers > 0,
                )),
                self.textures.layers as f32,
            ],
            mono_front: rgb_w(self.mono_colors.0, false),
            mono_back: rgb_w(self.mono_colors.1, false),
            edge_style: [
                f32::from(u8::from(style.edges)),
                style.depth_cue.unwrap_or(0.0).max(0.0),
                style.extension.unwrap_or(0.0).max(0.0),
                if style.back_edges && !wireframe && !xray {
                    BACK_EDGE_DASH
                } else {
                    0.0
                },
            ],
            view_axis: [forward.x, forward.y, forward.z, -forward.dot(eye)],
            depth_range: [
                near,
                far,
                style.fog.map_or(1.0, |f| near + f.start * (far - near)),
                style.fog.map_or(0.0, |f| near + f.end * (far - near)),
            ],
            fog_color: match style.fog.and_then(|f| f.color) {
                Some(c) => rgb_w(c, false),
                None => rgb_w([0.0; 3], true),
            },
            extra: [
                style.endpoints.unwrap_or(0.0).max(0.0),
                f32::from(u8::from(style.jitter)),
                f32::from(u8::from(style.transparency == Transparency::Off)),
                style.ground_opacity.clamp(0.0, 1.0),
            ],
            extra2: [style.horizon_blend, 0.0, 0.0, 0.0],
            select_color: rgb_w(style.selection_color, true),
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let mut encoder = device.create_command_encoder(&Default::default());
        if let (Some(_), Some(faces)) = (
            shadow_fit,
            self.shadow_casters.as_ref().or(self.faces.as_ref()),
        ) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_vertex_buffer(0, faces.vertices.slice(..));
            pass.set_index_buffer(faces.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..faces.translucent.end, 0, 0..1);
        }
        // Ambient occlusion splits the frame: the opaque faces first (keeping color
        // and depth), the occlusion pass reading their depth, then the rest.
        let ao = style
            .ambient_occlusion
            .filter(|_| !wireframe && !xray && self.faces.is_some());
        self.watermarks
            .prepare(device, queue, size, pixels_per_point);
        let split = ao.is_some();
        let depth_store = if split {
            wgpu::StoreOp::Store
        } else {
            wgpu::StoreOp::Discard
        };
        // The target drawn into: the multisampled one, else the result itself.
        let color_view = targets.msaa.as_ref().unwrap_or(&targets.resolve_view);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    // With occlusion the pass after it resolves.
                    resolve_target: targets
                        .msaa
                        .as_ref()
                        .filter(|_| !split)
                        .map(|_| &targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if self.transparent {
                            wgpu::Color::TRANSPARENT
                        } else {
                            wgpu::Color::WHITE
                        }),
                        // Without multisampling the pass draws into the result.
                        store: if split || targets.msaa.is_none() {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: depth_store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw_opaque(&mut pass, wireframe, xray);
            if !split {
                self.draw_rest(&mut pass, wireframe, xray);
            }
        }
        if let Some(ao) = ao {
            self.ao.record(
                device,
                queue,
                &mut encoder,
                color_view,
                &targets.depth,
                view_proj,
                size,
                pixels_per_point,
                &ao,
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport after occlusion"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: targets.msaa.as_ref().map(|_| &targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: if targets.msaa.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw_rest(&mut pass, wireframe, xray);
        }
        queue.submit([encoder.finish()]);
        &self.targets.as_ref().expect("targets just created").resolve
    }

    fn set_main_bind_groups(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &self.shadow_map.bind_group, &[]);
        pass.set_bind_group(2, &self.textures.bind_group, &[]);
        pass.set_bind_group(3, &self.env.bind_group, &[]);
    }

    /// Background, background watermarks and opaque faces (with the section fill and
    /// the live preview's opaque faces).
    fn draw_opaque(&self, pass: &mut wgpu::RenderPass<'_>, wireframe: bool, xray: bool) {
        self.set_main_bind_groups(pass);
        if !self.transparent {
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);
            self.watermarks.draw(pass, false);
            self.set_main_bind_groups(pass);
        }
        // Order: opaque faces, then edges and axes, which are pulled towards the
        // camera so lines on a face win the depth test and lines behind it fail.
        // Translucent faces last (without depth write) so they blend over
        // everything behind them, including the edges they cover; edges on their
        // own boundary are in front of them thanks to the same pull. In X-ray all
        // faces are drawn as translucent; in wireframe none are. With transparency
        // off translucent faces are drawn with the opaque ones (the shader makes them
        // opaque).
        let opaque_range = |f: &FaceBuffers| {
            if self.style.transparency == Transparency::Off {
                f.opaque.start..f.translucent.end.max(f.opaque.end)
            } else {
                f.opaque.clone()
            }
        };
        let faces = self.faces.as_ref().filter(|_| !wireframe);
        if let Some(faces) = faces.filter(|_| !xray) {
            let range = opaque_range(faces);
            if !range.is_empty() {
                pass.set_vertex_buffer(0, faces.vertices.slice(..));
                pass.set_index_buffer(faces.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_pipeline(&self.faces_opaque);
                pass.draw_indexed(range, 0, 0..1);
            }
        }
        if let Some(fill) = self.fill.as_ref().filter(|_| !wireframe) {
            pass.set_pipeline(&self.fill_pipeline);
            pass.set_vertex_buffer(0, fill.vertices.slice(..));
            pass.set_index_buffer(fill.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(fill.opaque.clone(), 0, 0..1);
        }
        if let Some(p) = self.preview.as_ref().filter(|_| !wireframe && !xray) {
            let range = opaque_range(p);
            if !range.is_empty() {
                pass.set_pipeline(&self.faces_opaque);
                pass.set_vertex_buffer(0, p.vertices.slice(..));
                pass.set_index_buffer(p.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(range, 0, 0..1);
            }
        }
    }

    /// The ground, selected faces, edges, axes, translucent faces and overlay
    /// watermarks.
    fn draw_rest(&self, pass: &mut wgpu::RenderPass<'_>, wireframe: bool, xray: bool) {
        let style = &self.style;
        self.set_main_bind_groups(pass);
        if !self.transparent && style.ground.is_some() {
            pass.set_pipeline(if style.ground_opacity >= 0.999 {
                &self.ground_opaque
            } else {
                &self.ground_blend
            });
            pass.draw(0..3, 0..1);
        }
        if let Some(sel) = self
            .selected
            .as_ref()
            .filter(|f| !wireframe && (!f.opaque.is_empty() || !f.translucent.is_empty()))
        {
            pass.set_pipeline(&self.selection);
            pass.set_vertex_buffer(0, sel.vertices.slice(..));
            pass.set_index_buffer(sel.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..sel.translucent.end.max(sel.opaque.end), 0, 0..1);
        }
        if let Some(edges) = &self.edges {
            pass.set_pipeline(&self.lines);
            pass.set_vertex_buffer(0, edges.slice(..));
            // Jitter draws a second stroke of each model edge (vertices 6..12).
            let vertices = if style.jitter { 12 } else { 6 };
            pass.draw(0..vertices, 0..self.edges_count);
        }
        if let Some(edges) = self
            .edges
            .as_ref()
            .filter(|_| style.back_edges && !wireframe && !xray)
        {
            pass.set_pipeline(&self.back_lines);
            pass.set_vertex_buffer(0, edges.slice(..));
            pass.draw(0..6, 0..self.edges_count);
        }
        if let Some(axes) = &self.axes {
            pass.set_pipeline(&self.overlay_lines);
            pass.set_vertex_buffer(0, axes.slice(..));
            pass.draw(0..6, 0..self.axes_count);
        }
        // X-ray: opaque triangles unsorted first, then the sorted translucent ones.
        // Transparency off: none here (drawn with the opaque faces).
        let off = style.transparency == Transparency::Off;
        let range = |f: &FaceBuffers| {
            if xray {
                f.opaque.start..f.translucent.end
            } else if off {
                0..0
            } else {
                f.translucent.clone()
            }
        };
        let faces = self.faces.as_ref().filter(|_| !wireframe);
        if let Some(faces) = faces.filter(|f| !range(f).is_empty()) {
            pass.set_pipeline(&self.faces_translucent);
            pass.set_vertex_buffer(0, faces.vertices.slice(..));
            pass.set_index_buffer(faces.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(range(faces), 0, 0..1);
        }
        let preview = self.preview.as_ref().filter(|_| !wireframe);
        if let Some(p) = preview.filter(|p| !range(p).is_empty()) {
            pass.set_pipeline(&self.faces_translucent);
            pass.set_vertex_buffer(0, p.vertices.slice(..));
            pass.set_index_buffer(p.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(range(p), 0, 0..1);
        }
        if !self.transparent {
            self.watermarks.draw(pass, true);
        }
    }

    /// Leaves the background (sky, ground, background color, watermarks) out of the
    /// on-screen frames too, clearing to transparent: a matched photo is drawn behind.
    pub fn set_transparent_background(&mut self, on: bool) {
        self.transparent = on;
    }

    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.resolve_view)
    }

    /// Renders one frame offscreen at `size` pixels and reads it back (File > Export >
    /// 2D Graphic). The on-screen targets are kept. With `transparent` the background
    /// is left out (alpha 0 there; colors are un-premultiplied for straight-alpha
    /// image files). `size` is clamped to the device's texture limit.
    pub fn render_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &Camera,
        size: [u32; 2],
        pixels_per_point: f32,
        transparent: bool,
    ) -> image::RgbaImage {
        let max = device.limits().max_texture_dimension_2d;
        let size = size.map(|x| x.clamp(1, max));
        let saved = self.targets.take();
        let was = std::mem::replace(&mut self.transparent, transparent);
        let texture = self
            .render(device, queue, camera, size, pixels_per_point)
            .clone();
        self.transparent = was;
        self.targets = saved;
        let mut img = read_back(device, queue, &texture);
        if transparent {
            for p in img.pixels_mut() {
                let a = p[3];
                if a > 0 && a < 255 {
                    for c in 0..3 {
                        p[c] = ((u32::from(p[c]) * 255 + u32::from(a) / 2) / u32::from(a)).min(255)
                            as u8;
                    }
                }
            }
        }
        img
    }
}

/// Copies a [`COLOR_FORMAT`] texture into an image (blocks until the GPU is done).
pub fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> image::RgbaImage {
    let (w, h) = (texture.width(), texture.height());
    let padded =
        (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read back"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let mut img = image::RgbaImage::new(w, h);
    if device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
        return img;
    }
    let Ok(data) = slice.get_mapped_range() else {
        return img;
    };
    for (y, row) in data.chunks(padded as usize).take(h as usize).enumerate() {
        let dst = &mut img.as_mut()[y * (w as usize) * 4..][..(w as usize) * 4];
        dst.copy_from_slice(&row[..(w as usize) * 4]);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn depth_cue_width_falls_from_near_to_far() {
        assert_eq!(depth_cue_width(100.0, 100.0, 300.0, 4.0), 4.0);
        assert_eq!(depth_cue_width(300.0, 100.0, 300.0, 4.0), 1.0);
        assert_eq!(depth_cue_width(200.0, 100.0, 300.0, 4.0), 2.5);
        // Clamped outside the model's depth range.
        assert_eq!(depth_cue_width(0.0, 100.0, 300.0, 4.0), 4.0);
        assert_eq!(depth_cue_width(1e6, 100.0, 300.0, 4.0), 1.0);
        // A flat range does not divide by zero.
        assert!(depth_cue_width(5.0, 5.0, 5.0, 4.0).is_finite());
    }

    #[test]
    fn fog_is_linear_between_start_and_end() {
        assert_eq!(fog_amount(10.0, 20.0, 40.0), 0.0);
        assert_eq!(fog_amount(30.0, 20.0, 40.0), 0.5);
        assert_eq!(fog_amount(50.0, 20.0, 40.0), 1.0);
        // Start at the end: a step.
        assert_eq!(fog_amount(19.0, 20.0, 20.0), 0.0);
        assert_eq!(fog_amount(20.0, 20.0, 20.0), 1.0);
    }

    #[test]
    fn extension_overshoots_both_ends() {
        let (a, b) = extend_segment(Vec2::new(10.0, 10.0), Vec2::new(10.0, 30.0), 2.0);
        assert_eq!(a, Vec2::new(10.0, 8.0));
        assert_eq!(b, Vec2::new(10.0, 32.0));
        let (a, b) = extend_segment(Vec2::ZERO, Vec2::new(3.0, 4.0), 5.0);
        assert!((a - Vec2::new(-3.0, -4.0)).length() < 1e-6);
        assert!((b - Vec2::new(6.0, 8.0)).length() < 1e-6);
        // A point stays a point.
        assert_eq!(
            extend_segment(Vec2::ONE, Vec2::ONE, 2.0),
            (Vec2::ONE, Vec2::ONE)
        );
    }

    #[test]
    fn segment_layout_matches_the_shader() {
        // 19 floats, then the flags word; the vertex layout in `Viewport::new` reads them.
        assert_eq!(std::mem::size_of::<Segment>(), 80);
        assert_eq!(std::mem::offset_of!(Segment, flags), 76);
    }
}
