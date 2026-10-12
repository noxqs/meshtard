//! Material textures: decoding, colorizing and the GPU texture array faces sample.
//!
//! All textures share one 2D array texture, each image resized to a common square
//! power-of-two size with a full mip chain, so faces with any texture draw in one call
//! and repeat with the sampler's wrapping.

use std::sync::Arc;

use egui_wgpu::wgpu;

/// Largest layer size. Textures are shown at most this sharp.
const MAX_SIZE: u32 = 1024;
/// Memory budget of the whole array (all mips) in bytes; layers shrink to fit.
const BUDGET: u64 = 256 << 20;

/// A texture for [`crate::Viewport::set_textures`]: the image file and an optional
/// colorize color.
#[derive(Debug, Clone)]
pub struct TextureSource {
    /// PNG, JPEG or BMP file bytes. Decoded images are cached by this allocation, so
    /// pass the same `Arc` again for an unchanged texture.
    pub data: Arc<[u8]>,
    /// SketchUp's colorize: when set and noticeably different from the image's average
    /// color, the image is recolored towards it (keeping each pixel's brightness
    /// relative to the average). A draft of SketchUp's colorize, which shifts hue,
    /// saturation and lightness (unverified).
    pub tint: Option<[u8; 3]>,
}

/// A decoded RGBA image.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, top row first.
    pub rgba: Vec<u8>,
}

impl TextureImage {
    /// Decodes PNG, JPEG or BMP bytes; `None` for anything else or broken data.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let img = image::load_from_memory(bytes).ok()?.into_rgba8();
        let (width, height) = img.dimensions();
        (width > 0 && height > 0).then(|| Self {
            width,
            height,
            rgba: img.into_raw(),
        })
    }

    /// Mean RGBA over all pixels.
    pub fn average(&self) -> [u8; 4] {
        let mut sum = [0u64; 4];
        for p in self.rgba.as_chunks::<4>().0 {
            for (s, &c) in sum.iter_mut().zip(p) {
                *s += u64::from(c);
            }
        }
        let n = (self.rgba.len() / 4).max(1) as u64;
        sum.map(|s| (s / n) as u8)
    }

    /// Recolors towards `color` (see [`TextureSource::tint`]). No change when the
    /// average is within a few levels of `color` already.
    pub fn colorize(&mut self, color: [u8; 3]) {
        let avg = self.average();
        if (0..3).all(|i| avg[i].abs_diff(color[i]) <= 3) {
            return;
        }
        let luma = |p: &[u8; 4]| {
            0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])
        };
        let avg_luma = luma(&avg).max(1.0);
        for p in self.rgba.as_chunks_mut::<4>().0 {
            let k = luma(p) / avg_luma;
            for i in 0..3 {
                p[i] = (f32::from(color[i]) * k).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    /// Decoded and colorized as `source` says.
    fn load(source: &TextureSource) -> Option<Self> {
        let mut img = Self::decode(&source.data)?;
        if let Some(c) = source.tint {
            img.colorize(c);
        }
        Some(img)
    }

    /// Resized to `size` x `size` (bilinear-ish), or `self` when it already is.
    fn square(&self, size: u32) -> Vec<u8> {
        if self.width == size && self.height == size {
            return self.rgba.clone();
        }
        let img = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())
            .expect("consistent image size");
        image::imageops::resize(&img, size, size, image::imageops::FilterType::Triangle).into_raw()
    }
}

/// Halves a square RGBA image (2x2 box filter).
fn half(src: &[u8], size: u32) -> Vec<u8> {
    let n = (size / 2).max(1);
    let s = size as usize;
    let mut out = vec![0u8; (n * n * 4) as usize];
    for y in 0..n as usize {
        for x in 0..n as usize {
            for c in 0..4 {
                let at = |dx: usize, dy: usize| {
                    let (sx, sy) = ((2 * x + dx).min(s - 1), (2 * y + dy).min(s - 1));
                    u32::from(src[(sy * s + sx) * 4 + c])
                };
                out[(y * n as usize + x) * 4 + c] =
                    ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
            }
        }
    }
    out
}

/// Decoded images by source, and the GPU array of the current list.
pub(crate) struct Textures {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Sources and their images (`None`: undecodable), in layer order.
    current: Vec<(TextureSource, Option<Arc<TextureImage>>)>,
    /// The array and its bind group are out of date.
    dirty: bool,
    pub bind_group: wgpu::BindGroup,
    /// Layers in the array that hold textures (excess textures are dropped).
    pub layers: u32,
    /// The array behind `bind_group`, kept to rebind with another sampler.
    array: wgpu::Texture,
    smooth: bool,
}

/// The face texture sampler: linear with mipmaps (anti-aliased) or nearest.
fn face_sampler(device: &wgpu::Device, smooth: bool) -> wgpu::Sampler {
    let (filter, mip) = if smooth {
        (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear)
    } else {
        (wgpu::FilterMode::Nearest, wgpu::MipmapFilterMode::Nearest)
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("face textures"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: mip,
        anisotropy_clamp: if smooth { 8 } else { 1 },
        ..Default::default()
    })
}

impl Textures {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("face textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
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
        let sampler = face_sampler(device, true);
        let array = Self::array(device, 1, 1, 1);
        let bind_group = Self::bind(device, &layout, &sampler, &array);
        Self {
            layout,
            sampler,
            current: Vec::new(),
            dirty: false,
            bind_group,
            layers: 0,
            array,
            smooth: true,
        }
    }

    /// Switches between linear-with-mipmaps (`smooth`) and nearest sampling.
    pub fn set_smooth(&mut self, device: &wgpu::Device, smooth: bool) {
        if smooth == self.smooth {
            return;
        }
        self.smooth = smooth;
        self.sampler = face_sampler(device, smooth);
        self.bind_group = Self::bind(device, &self.layout, &self.sampler, &self.array);
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    fn array(device: &wgpu::Device, size: u32, layers: u32, mips: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("face textures"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Material colors are sRGB values used as is; textures are treated alike.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        texture: &wgpu::Texture,
    ) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("face textures"),
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
        })
    }

    /// Replaces the list; decodes new sources (in parallel), reuses decoded ones.
    pub fn set(&mut self, sources: &[TextureSource]) {
        let same = |a: &TextureSource, b: &TextureSource| {
            Arc::ptr_eq(&a.data, &b.data) && a.tint == b.tint
        };
        if sources.len() == self.current.len()
            && sources
                .iter()
                .zip(&self.current)
                .all(|(s, (c, _))| same(s, c))
        {
            return;
        }
        let known = |s: &TextureSource| {
            self.current
                .iter()
                .find(|(c, _)| same(s, c))
                .map(|(_, img)| img.clone())
        };
        let cached: Vec<Option<Option<Arc<TextureImage>>>> = sources.iter().map(known).collect();
        // Decode the new ones on a few threads.
        let todo: Vec<usize> = (0..sources.len())
            .filter(|&i| cached[i].is_none())
            .collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let threads = std::thread::available_parallelism()
            .map_or(4, usize::from)
            .min(todo.len());
        let fresh: Vec<(usize, Option<Arc<TextureImage>>)> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(&i) = todo.get(k) else {
                                return out;
                            };
                            out.push((i, TextureImage::load(&sources[i]).map(Arc::new)));
                        }
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|w| w.join().unwrap_or_default())
                .collect()
        });
        let mut decoded: Vec<Option<Arc<TextureImage>>> =
            cached.into_iter().map(Option::flatten).collect();
        for (i, img) in fresh {
            decoded[i] = img;
        }
        self.current = sources.iter().cloned().zip(decoded).collect();
        self.dirty = true;
    }

    /// Uploads the array when the list changed.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let limits = device.limits();
        let count = (self.current.len() as u32).min(limits.max_texture_array_layers);
        let images: Vec<Option<&TextureImage>> = self.current[..count as usize]
            .iter()
            .map(|(_, img)| img.as_deref())
            .collect();
        let largest = images.iter().flatten().map(|i| i.width.max(i.height)).max();
        let Some(largest) = largest else {
            self.layers = 0;
            self.array = Self::array(device, 1, 1, 1);
            self.bind_group = Self::bind(device, &self.layout, &self.sampler, &self.array);
            return;
        };
        let mut size = largest
            .next_power_of_two()
            .min(MAX_SIZE)
            .min(limits.max_texture_dimension_2d);
        while size > 64 && u64::from(count) * u64::from(size * size) * 4 * 4 / 3 > BUDGET {
            size /= 2;
        }
        let mips = size.ilog2() + 1;
        let texture = Self::array(device, size, count, mips);
        for (layer, img) in images.iter().enumerate() {
            // Undecodable textures leave a white layer (faces still have their color
            // in Shaded; with textures they show white, like a missing image).
            let mut level = img.map_or_else(
                || vec![255u8; (size * size * 4) as usize],
                |i| i.square(size),
            );
            let mut s = size;
            for mip in 0..mips {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &level,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * s),
                        rows_per_image: Some(s),
                    },
                    wgpu::Extent3d {
                        width: s,
                        height: s,
                        depth_or_array_layers: 1,
                    },
                );
                if s > 1 {
                    level = half(&level, s);
                    s /= 2;
                }
            }
        }
        self.layers = count;
        self.bind_group = Self::bind(device, &self.layout, &self.sampler, &texture);
        self.array = texture;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn decodes_and_averages() {
        let img = TextureImage::decode(&png(2, 2, |x, _| {
            if x == 0 {
                [0, 0, 0, 255]
            } else {
                [200, 100, 50, 255]
            }
        }))
        .unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(img.average(), [100, 50, 25, 255]);
        assert!(TextureImage::decode(b"not an image").is_none());
    }

    #[test]
    fn colorize_keeps_relative_brightness() {
        let mut img = TextureImage::decode(&png(2, 1, |x, _| {
            if x == 0 {
                [50, 50, 50, 255]
            } else {
                [150, 150, 150, 255]
            }
        }))
        .unwrap();
        let before = img.clone();
        img.colorize([100, 100, 100]);
        assert_eq!(img, before, "already this color on average");
        img.colorize([200, 0, 0]);
        assert_eq!(&img.rgba[..4], &[100, 0, 0, 255]);
        assert_eq!(&img.rgba[4..], &[255, 0, 0, 255]);
    }

    #[test]
    fn halving_averages_blocks() {
        let src = [0, 0, 0, 0, 4, 4, 4, 4, 8, 8, 8, 8, 12, 12, 12, 12];
        assert_eq!(half(&src, 2), vec![6, 6, 6, 6]);
    }
}
