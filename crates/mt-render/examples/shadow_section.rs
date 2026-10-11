//! Renders a box on the ground with sun shadows, a translucent pane and a section plane
//! to PNGs, headlessly: `cargo run -p mt-render --example shadow_section [out_dir]`.

use egui_wgpu::wgpu;
use glam::DVec3;
use mt_render::{
    Camera, FaceVertex, SectionCut, Segment, Shadows, Style, Viewport, section_lines, sun_direction,
};
use mt_spec::Spec;

const SIZE: [u32; 2] = [800, 500];

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("no wgpu adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default())).expect("no device");

    let spec = Spec::builtin().unwrap().default_model;
    let mut viewport = Viewport::new(&device, Style::from_spec(&spec));
    let white = [0.95, 0.95, 0.95, 1.0];
    let back = [0.64, 0.68, 0.75, 1.0];
    let glass = [0.4, 0.6, 0.9, 0.4];
    let mut quads = box_quads(DVec3::new(-60.0, -40.0, 0.0), DVec3::new(60.0, 40.0, 80.0))
        .into_iter()
        .map(|q| (q, white, back))
        .collect::<Vec<_>>();
    // A slab floating over the box's corner, and a translucent pane.
    quads.extend(
        box_quads(
            DVec3::new(30.0, 20.0, 100.0),
            DVec3::new(110.0, 80.0, 106.0),
        )
        .into_iter()
        .map(|q| (q, white, back)),
    );
    quads.push((
        (
            DVec3::NEG_Y,
            [
                DVec3::new(-150.0, -80.0, 0.0),
                DVec3::new(-90.0, -80.0, 0.0),
                DVec3::new(-90.0, -80.0, 70.0),
                DVec3::new(-150.0, -80.0, 70.0),
            ],
        ),
        glass,
        glass,
    ));
    let (vertices, indices, triangles) = mesh(&quads);
    viewport.set_faces(&device, &vertices, &indices);

    let mut edges: Vec<Segment> = quads
        .iter()
        .flat_map(|((_, c), _, _)| {
            (0..4).map(move |i| {
                Segment::line(
                    c[i].as_vec3().into(),
                    c[(i + 1) % 4].as_vec3().into(),
                    [0.0, 0.0, 0.0, 1.0],
                    [1.0, 0.0],
                )
            })
        })
        .collect();
    viewport.set_edges(&device, &edges);

    let camera = Camera {
        eye: DVec3::new(-350.0, -420.0, 260.0),
        target: DVec3::new(0.0, 0.0, 30.0),
        fov_deg: 35.0,
        perspective: true,
        two_point: false,
    };
    // Boulder, CO, 2024-06-21 09:00 MDT.
    let sun = sun_direction(40.0, -105.0, -6.0, 2024, 6, 21, 9, 0);
    eprintln!("sun {sun}");
    viewport.set_shadows(Some(Shadows {
        sun_dir: sun.as_vec3().into(),
        light: 0.80,
        dark: 0.45,
        on_faces: true,
        on_ground: true,
    }));
    save(&device, &queue, &mut viewport, &camera, &out_dir, "shadows");

    let point = DVec3::new(20.0, 0.0, 0.0);
    let normal = DVec3::new(1.0, 0.3, 0.0);
    viewport.set_section(Some(SectionCut {
        point: point.as_vec3().into(),
        normal: normal.as_vec3().into(),
    }));
    edges.extend(
        section_lines(&triangles, point, normal)
            .iter()
            .map(|[a, b]| {
                Segment::line(
                    a.as_vec3().into(),
                    b.as_vec3().into(),
                    [0.0, 0.0, 0.0, 1.0],
                    [3.0, 0.0],
                )
            }),
    );
    viewport.set_edges(&device, &edges);
    save(
        &device,
        &queue,
        &mut viewport,
        &camera,
        &out_dir,
        "shadows_section",
    );

    // From the cut side: the box is open, its inside (back faces) in the sun.
    let inside = Camera {
        eye: DVec3::new(420.0, -260.0, 300.0),
        ..camera
    };
    save(
        &device,
        &queue,
        &mut viewport,
        &inside,
        &out_dir,
        "section_inside",
    );

    viewport.set_shadows(None);
    save(&device, &queue, &mut viewport, &camera, &out_dir, "section");
}

type Quad = (DVec3, [DVec3; 4]);

fn box_quads(lo: DVec3, hi: DVec3) -> Vec<Quad> {
    let mut out = Vec::new();
    let c = (lo + hi) / 2.0;
    let h = (hi - lo) / 2.0;
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            let n = DVec3::AXES[axis] * sign;
            let (mut u, mut v) = (DVec3::AXES[(axis + 1) % 3], DVec3::AXES[(axis + 2) % 3]);
            if sign < 0.0 {
                std::mem::swap(&mut u, &mut v);
            }
            let corners = [-u - v, u - v, u + v, -u + v].map(|d| c + (n + d) * h);
            out.push((n, corners));
        }
    }
    out
}

#[allow(clippy::type_complexity)]
fn mesh(quads: &[(Quad, [f32; 4], [f32; 4])]) -> (Vec<FaceVertex>, Vec<u32>, Vec<[DVec3; 3]>) {
    let (mut vertices, mut indices, mut triangles) = (Vec::new(), Vec::new(), Vec::new());
    for ((n, c), front, back) in quads {
        let base = vertices.len() as u32;
        for p in c {
            vertices.push(FaceVertex::new(
                p.as_vec3().into(),
                n.as_vec3().into(),
                *front,
                *back,
            ));
        }
        indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        triangles.push([c[0], c[1], c[2]]);
        triangles.push([c[0], c[2], c[3]]);
    }
    (vertices, indices, triangles)
}

fn save(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    viewport: &mut Viewport,
    camera: &Camera,
    dir: &str,
    name: &str,
) {
    let texture = viewport.render(device, queue, camera, SIZE, 1.0).clone();
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
    let path = std::path::Path::new(dir).join(format!("{name}.png"));
    img.save(&path).unwrap();
    eprintln!("saved {}", path.display());
}
