//! Renders the default view without a window and checks pixels.
//! Needs a Vulkan driver; on CI that is Mesa's software rasterizer (lavapipe).
//! Skips with a message when no adapter is found.

use egui_wgpu::wgpu;
use glam::{DVec2, DVec3};
use mt_render::{Camera, Style, Viewport};
use mt_spec::Spec;

const SIZE: [u32; 2] = [640, 400];

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    pollster::block_on(adapter.request_device(&Default::default())).ok()
}

fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> image::RgbaImage {
    let [w, h] = SIZE;
    let padded =
        (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * h),
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
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap();
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        let row = &data[(y * padded) as usize..][..(w * 4) as usize];
        for x in 0..w {
            let p = &row[(x * 4) as usize..][..4];
            img.put_pixel(x, y, image::Rgba([p[0], p[1], p[2], p[3]]));
        }
    }
    img
}

fn close(px: &image::Rgba<u8>, rgb: [f32; 3], tolerance: u8) -> bool {
    (0..3).all(|i| px[i].abs_diff((rgb[i] * 255.0).round() as u8) <= tolerance)
}

#[test]
fn default_view_has_sky_ground_and_axes() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let spec = Spec::builtin().unwrap().default_model;
    let camera = Camera::from_spec(&spec.camera);
    let mut viewport = Viewport::new(&device, Style::from_spec(&spec));
    let texture = viewport.render(&device, &queue, &camera, SIZE, 1.0).clone();
    let img = read_back(&device, &queue, &texture);

    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("default_view.png");
    img.save(&out).unwrap();
    eprintln!("saved {}", out.display());

    // Top-center looks up into the sky near the zenith side, so it is a sky color
    // (blend of horizon and zenith), not ground.
    let top = img.get_pixel(SIZE[0] / 2, 2);
    assert!(top[2] > top[0], "sky should be bluish, got {top:?}");
    let bottom = img.get_pixel(SIZE[0] / 2, SIZE[1] - 2);
    assert!(
        close(bottom, spec.ground.color, 2),
        "ground, got {bottom:?}"
    );

    // A point on the positive X axis is drawn in the X axis color.
    let viewport_px = DVec2::new(f64::from(SIZE[0]), f64::from(SIZE[1]));
    let on_x = camera
        .project(DVec3::new(200.0, 0.0, 0.0), viewport_px)
        .unwrap();
    let px = img.get_pixel(on_x.x.round() as u32, on_x.y.round() as u32);
    assert!(
        px[0] > 150 && px[1] < 120 && px[2] < 120,
        "red X axis, got {px:?}"
    );

    // The positive Z axis is solid blue.
    let on_z = camera
        .project(DVec3::new(0.0, 0.0, 100.0), viewport_px)
        .unwrap();
    let px = img.get_pixel(on_z.x.round() as u32, on_z.y.round() as u32);
    assert!(px[2] > 150 && px[0] < 120, "blue Z axis, got {px:?}");
}

#[test]
fn negative_axis_is_dotted() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let spec = Spec::builtin().unwrap().default_model;
    // Look straight down so the -X axis runs horizontally across the screen.
    let camera = Camera {
        eye: DVec3::new(0.0, 0.0, 500.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
    };
    let mut viewport = Viewport::new(&device, Style::from_spec(&spec));
    let texture = viewport.render(&device, &queue, &camera, SIZE, 1.0).clone();
    let img = read_back(&device, &queue, &texture);

    let y = SIZE[1] / 2;
    let red = |x: u32| {
        let p = img.get_pixel(x, y);
        p[0] > 150 && p[1] < 150
    };
    let left: Vec<bool> = (20..SIZE[0] / 2 - 20).map(red).collect();
    let right: Vec<bool> = (SIZE[0] / 2 + 20..SIZE[0] - 20).map(red).collect();
    assert!(right.iter().all(|&r| r), "+X axis should be solid");
    let on = left.iter().filter(|&&r| r).count();
    assert!(
        on > left.len() / 4 && on < left.len() * 3 / 4,
        "-X axis should be dotted: {on}/{}",
        left.len()
    );
}
