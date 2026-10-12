//! Environments (FEATURES.md 66, help page "Adding an environment to models"):
//! equirectangular panoramas drawn as a skydome behind the model and used for cheap
//! reflections on faces.
//!
//! An [`EnvImage`] holds linear RGB (HDR or LDR alike: PNG and JPEG are decoded from
//! sRGB). On the GPU it is an `Rgba16Float` texture; exposure, clamping and the sRGB
//! encoding happen in the shader (`env_color` in `viewport.wgsl`), since the viewport
//! writes sRGB values straight to its target.
//!
//! Mapping (mirrored by the shader): world Z is up; `v` runs from the zenith (0) to
//! the nadir (1); `u` = 0.5 looks along +X with the rotation at 0 and decreases
//! counter-clockwise seen from above, so the panorama is not mirrored when seen from
//! inside. Rotation turns the panorama counter-clockwise about Z. SketchUp's own
//! orientation convention is unverified.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use egui_wgpu::wgpu;

/// Widest panorama uploaded to the GPU; larger ones are box-filtered down.
pub const MAX_WIDTH: u32 = 4096;

/// A decoded equirectangular panorama in linear RGB, rows from the zenith down.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
}

/// The bundled procedural skies (generated at run time, no third-party images).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProceduralSky {
    /// Blue sky over a grey-green ground, a bright sun glow.
    ClearDay,
    /// Orange horizon, purple zenith, dark ground, low sun.
    Sunset,
    /// Even grey sky, soft light.
    Overcast,
}

impl ProceduralSky {
    pub const ALL: [Self; 3] = [Self::ClearDay, Self::Sunset, Self::Overcast];

    pub fn name(self) -> &'static str {
        match self {
            Self::ClearDay => "Clear Day",
            Self::Sunset => "Sunset",
            Self::Overcast => "Overcast",
        }
    }

    /// (sun u, sun v) on the panorama: where the sun glow is drawn.
    pub fn sun_uv(self) -> Option<[f32; 2]> {
        match self {
            Self::ClearDay => Some([0.30, 0.22]),
            Self::Sunset => Some([0.62, 0.46]),
            Self::Overcast => None,
        }
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear to sRGB-encoded, clamped to 0..1. Mirrors `linear_to_srgb` in the shader.
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Unit direction of panorama point (`u`, `v`) with the panorama turned
/// `rotation_deg` counter-clockwise about Z.
pub fn uv_to_dir(u: f32, v: f32, rotation_deg: f32) -> [f32; 3] {
    let lon = (0.5 - u) * TAU + rotation_deg.to_radians();
    let polar = v.clamp(0.0, 1.0) * PI;
    [
        polar.sin() * lon.cos(),
        polar.sin() * lon.sin(),
        polar.cos(),
    ]
}

/// The panorama point seen along `dir` (any length but zero). Inverse of
/// [`uv_to_dir`]; mirrors `env_uv` in the shader.
pub fn dir_to_uv(dir: [f32; 3], rotation_deg: f32) -> [f32; 2] {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
        .sqrt()
        .max(1e-12);
    let lon = dir[1].atan2(dir[0]) - rotation_deg.to_radians();
    let u = (0.5 - lon / TAU).rem_euclid(1.0);
    let v = (dir[2] / len).clamp(-1.0, 1.0).acos() / PI;
    [u, v]
}

impl EnvImage {
    /// Decodes HDR (Radiance .hdr), OpenEXR or LDR (PNG, JPEG, ...) bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
        let img = image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?;
        let hdr = matches!(
            format,
            image::ImageFormat::Hdr | image::ImageFormat::OpenExr
        );
        let rgb = img.to_rgb32f();
        let (width, height) = rgb.dimensions();
        if width == 0 || height == 0 {
            return Err("empty image".into());
        }
        let pixels = rgb
            .pixels()
            .map(|p| {
                if hdr {
                    p.0.map(|c| if c.is_finite() { c.max(0.0) } else { 0.0 })
                } else {
                    p.0.map(srgb_to_linear)
                }
            })
            .collect();
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Reads and decodes a file.
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Self::decode(&bytes)
    }

    /// One of the bundled skies, `width` x `width / 2` pixels.
    pub fn procedural(sky: ProceduralSky, width: u32) -> Self {
        let width = width.max(4);
        let height = width / 2;
        let (zenith, horizon, ground, sun_color, sun_size): ([f32; 3], [f32; 3], [f32; 3], _, _) =
            match sky {
                ProceduralSky::ClearDay => (
                    [0.08, 0.22, 0.60],
                    [0.62, 0.78, 0.95],
                    [0.18, 0.20, 0.15],
                    [30.0, 28.0, 24.0],
                    0.03,
                ),
                ProceduralSky::Sunset => (
                    [0.10, 0.06, 0.25],
                    [1.10, 0.45, 0.15],
                    [0.05, 0.04, 0.05],
                    [20.0, 9.0, 3.0],
                    0.05,
                ),
                ProceduralSky::Overcast => (
                    [0.55, 0.57, 0.60],
                    [0.80, 0.81, 0.82],
                    [0.25, 0.25, 0.24],
                    [0.0; 3],
                    1.0,
                ),
            };
        let sun = sky.sun_uv().map(|[u, v]| uv_to_dir(u, v, 0.0));
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let u = (x as f32 + 0.5) / width as f32;
                let v = (y as f32 + 0.5) / height as f32;
                let d = uv_to_dir(u, v, 0.0);
                let mut c = if d[2] >= 0.0 {
                    lerp3(horizon, zenith, d[2].powf(0.45))
                } else {
                    // A soft blend just below the horizon, then flat ground.
                    lerp3(horizon, ground, (-d[2] * 12.0).min(1.0))
                };
                if let Some(s) = sun {
                    let cos = d[0] * s[0] + d[1] * s[1] + d[2] * s[2];
                    let angle = cos.clamp(-1.0, 1.0).acos();
                    let disc = if angle < sun_size { 1.0 } else { 0.0 };
                    let glow = (-angle * 6.0).exp() * 0.6;
                    for i in 0..3 {
                        c[i] += sun_color[i] * disc + sun_color[i] * 0.02 * glow;
                    }
                }
                pixels.push(c);
            }
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Average color over the sphere (rows weighted by the solid angle they cover).
    pub fn average(&self) -> [f32; 3] {
        let mut sum = [0.0f64; 3];
        let mut weight = 0.0f64;
        for y in 0..self.height {
            let v = (y as f64 + 0.5) / f64::from(self.height);
            let w = (v * std::f64::consts::PI).sin();
            let row = &self.pixels[(y * self.width) as usize..((y + 1) * self.width) as usize];
            for p in row {
                for i in 0..3 {
                    sum[i] += f64::from(p[i]) * w;
                }
            }
            weight += w * f64::from(self.width);
        }
        sum.map(|s| (s / weight.max(1e-12)) as f32)
    }

    /// The panorama box-filtered down to at most `max_width` wide (same aspect).
    pub fn downsized(&self, max_width: u32) -> Self {
        if self.width <= max_width {
            return self.clone();
        }
        let f = self.width.div_ceil(max_width.max(1));
        let (w, h) = (self.width / f, (self.height / f).max(1));
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let mut c = [0.0f32; 3];
                let mut n = 0.0;
                for yy in y * f..((y + 1) * f).min(self.height) {
                    for xx in x * f..((x + 1) * f).min(self.width) {
                        let p = self.pixels[(yy * self.width + xx) as usize];
                        for i in 0..3 {
                            c[i] += p[i];
                        }
                        n += 1.0;
                    }
                }
                pixels.push(c.map(|v| v / n));
            }
        }
        Self {
            width: w,
            height: h,
            pixels,
        }
    }

    /// An sRGB RGBA thumbnail (or full-size preview) at `exposure`, at most
    /// `max_width` wide.
    pub fn preview_rgba(&self, max_width: u32, exposure: f32) -> (u32, u32, Vec<u8>) {
        let small = self.downsized(max_width);
        let mut out = Vec::with_capacity(small.pixels.len() * 4);
        for p in &small.pixels {
            for c in p {
                out.push((linear_to_srgb(c * exposure) * 255.0).round() as u8);
            }
            out.push(255);
        }
        (small.width, small.height, out)
    }

    /// The brightest pixel's panorama coordinates: a guess at where the sun is.
    pub fn brightest_uv(&self) -> [f32; 2] {
        let (i, _) = self
            .pixels
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |(bi, bl), (i, p)| {
                let l = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                if l > bl { (i, l) } else { (bi, bl) }
            });
        let (x, y) = (i as u32 % self.width, i as u32 / self.width);
        [
            (x as f32 + 0.5) / self.width as f32,
            (y as f32 + 0.5) / self.height as f32,
        ]
    }
}

/// How the viewport uses the environment (Environments panel, Edit section).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvView {
    /// Use as Skydome: draw the panorama behind the model (replaces sky, ground and
    /// background color).
    pub skydome: bool,
    /// Use for Reflections: faces are tinted by the average color and reflect the
    /// panorama at grazing angles (a cheap stand-in; meshtard has no glossy materials).
    pub reflections: bool,
    pub rotation_deg: f32,
    pub skydome_exposure: f32,
    pub reflection_exposure: f32,
}

impl Default for EnvView {
    fn default() -> Self {
        Self {
            skydome: true,
            reflections: true,
            rotation_deg: 0.0,
            skydome_exposure: 1.0,
            reflection_exposure: 1.0,
        }
    }
}

/// Face tint for reflections: the average color scaled to unit luminance, a third
/// of the way from white. Mirrors nothing in the shader (passed as a uniform).
pub fn reflection_tint(average: [f32; 3]) -> [f32; 3] {
    let lum = 0.2126 * average[0] + 0.7152 * average[1] + 0.0722 * average[2];
    if lum <= 1e-6 {
        return [1.0; 3];
    }
    average.map(|c| 1.0 + (c / lum - 1.0) * 0.35)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EnvParams {
    /// x: 1 skydome, y: skydome exposure, z: rotation radians, w: 1 reflections.
    a: [f32; 4],
    /// rgb: reflection tint, w: reflection exposure.
    b: [f32; 4],
}

/// The GPU side: bind group 3 of the viewport pipelines.
pub(crate) struct EnvPass {
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
    /// The image now on the GPU (pointer identity) and the one to show.
    uploaded: Option<Arc<EnvImage>>,
    wanted: Option<(Arc<EnvImage>, EnvView)>,
    tint: [f32; 3],
}

impl EnvPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment"),
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("environment params"),
            size: std::mem::size_of::<EnvParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("environment"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let texture = Self::texture(device, 1, 1);
        let bind_group = Self::bind(device, &layout, &params, &sampler, &texture);
        Self {
            layout,
            params,
            sampler,
            bind_group,
            uploaded: None,
            wanted: None,
            tint: [1.0; 3],
        }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    fn texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("environment"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        params: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
        texture: &wgpu::Texture,
    ) -> wgpu::BindGroup {
        let view = texture.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("environment"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    pub fn set(&mut self, env: Option<(Arc<EnvImage>, EnvView)>) {
        self.wanted = env;
    }

    /// Uploads a new image if needed and writes the parameters.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let image = self.wanted.as_ref().map(|(i, _)| i.clone());
        let same = match (&image, &self.uploaded) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            let texture = match &image {
                Some(img) => {
                    let small =
                        img.downsized(MAX_WIDTH.min(device.limits().max_texture_dimension_2d));
                    let texture = Self::texture(device, small.width, small.height);
                    let data: Vec<u16> = small
                        .pixels
                        .iter()
                        .flat_map(|p| [p[0], p[1], p[2], 1.0])
                        .map(|c| half::f16::from_f32(c.min(65000.0)).to_bits())
                        .collect();
                    queue.write_texture(
                        texture.as_image_copy(),
                        bytemuck::cast_slice(&data),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(small.width * 8),
                            rows_per_image: Some(small.height),
                        },
                        texture.size(),
                    );
                    self.tint = reflection_tint(img.average());
                    texture
                }
                None => Self::texture(device, 1, 1),
            };
            self.bind_group =
                Self::bind(device, &self.layout, &self.params, &self.sampler, &texture);
            self.uploaded = image;
        }
        let params = match &self.wanted {
            Some((_, v)) => EnvParams {
                a: [
                    f32::from(u8::from(v.skydome)),
                    v.skydome_exposure,
                    v.rotation_deg.to_radians(),
                    f32::from(u8::from(v.reflections)),
                ],
                b: [
                    self.tint[0],
                    self.tint[1],
                    self.tint[2],
                    v.reflection_exposure,
                ],
            },
            None => EnvParams {
                a: [0.0; 4],
                b: [1.0, 1.0, 1.0, 0.0],
            },
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_and_direction_round_trip() {
        for rot in [0.0, 37.0, -120.0] {
            for (u, v) in [(0.1, 0.2), (0.5, 0.5), (0.9, 0.7), (0.25, 0.95)] {
                let d = uv_to_dir(u, v, rot);
                let [u2, v2] = dir_to_uv(d, rot);
                assert!((u - u2).abs() < 1e-4, "{u} {u2}");
                assert!((v - v2).abs() < 1e-4);
            }
        }
        // Zenith at v = 0, +X at the center without rotation, +Y left of it.
        assert!(uv_to_dir(0.3, 0.0, 0.0)[2] > 0.999);
        let x = uv_to_dir(0.5, 0.5, 0.0);
        assert!(x[0] > 0.999);
        let y = uv_to_dir(0.25, 0.5, 0.0);
        assert!(y[1] > 0.999);
        // Rotating by 90 degrees turns +X to +Y.
        assert!(uv_to_dir(0.5, 0.5, 90.0)[1] > 0.999);
    }

    #[test]
    fn procedural_skies_are_bright_above_and_dark_below() {
        for sky in ProceduralSky::ALL {
            let img = EnvImage::procedural(sky, 64);
            assert_eq!((img.width, img.height), (64, 32));
            let top = img.pixels[3];
            let bottom = img.pixels[img.pixels.len() - 3];
            let lum = |p: [f32; 3]| p[0] + p[1] + p[2];
            assert!(lum(top) > lum(bottom), "{sky:?}");
            let avg = img.average();
            assert!(avg.iter().all(|c| *c > 0.0));
        }
        // The sun is the brightest spot.
        let img = EnvImage::procedural(ProceduralSky::ClearDay, 256);
        let [u, v] = img.brightest_uv();
        let [su, sv] = ProceduralSky::ClearDay.sun_uv().unwrap();
        assert!((u - su).abs() < 0.03 && (v - sv).abs() < 0.03, "{u} {v}");
    }

    #[test]
    fn decodes_ldr_and_hdr() {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(4, 2, image::Rgb([255, 128, 0]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let img = EnvImage::decode(&png).unwrap();
        assert_eq!((img.width, img.height), (4, 2));
        assert!((img.pixels[0][0] - 1.0).abs() < 1e-5);
        assert!((img.pixels[0][1] - srgb_to_linear(128.0 / 255.0)).abs() < 1e-4);
        let mut hdr = Vec::new();
        let px = vec![image::Rgb([4.0f32, 0.5, 0.25]); 8];
        image::codecs::hdr::HdrEncoder::new(&mut hdr)
            .encode(&px, 4, 2)
            .unwrap();
        let img = EnvImage::decode(&hdr).unwrap();
        assert!((img.pixels[0][0] - 4.0).abs() < 0.1, "{:?}", img.pixels[0]);
        assert!(EnvImage::decode(b"not an image").is_err());
    }

    #[test]
    fn downsizing_keeps_the_average_and_tint_is_neutral_for_grey() {
        let img = EnvImage::procedural(ProceduralSky::Sunset, 256);
        let small = img.downsized(64);
        assert_eq!(small.width, 64);
        let (a, b) = (img.average(), small.average());
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 0.05 * a[i].max(0.1), "{a:?} {b:?}");
        }
        let t = reflection_tint([0.5, 0.5, 0.5]);
        assert!(t.iter().all(|c| (c - 1.0).abs() < 1e-5));
        let (w, h, rgba) = img.preview_rgba(32, 1.0);
        assert_eq!(rgba.len() as u32, w * h * 4);
    }
}
