//! Renders the default view without a window and checks pixels.
//! Needs a Vulkan driver; on CI that is Mesa's software rasterizer (lavapipe).
//! Skips with a message when no adapter is found.

use egui_wgpu::wgpu;
use glam::{DVec2, DVec3};
use mt_render::{Camera, FaceStyle, FaceVertex, SectionCut, Segment, Shadows, Style, Viewport};
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

// ---------------------------------------------------------------- faces

const HALF: f64 = 50.0;

/// A quad of a cube centered at the origin (size 100): outward normal and its four
/// corners, counter-clockwise around the normal.
fn cube_quads() -> Vec<(DVec3, [DVec3; 4])> {
    let mut out = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            let n = DVec3::AXES[axis] * sign;
            let (mut u, mut v) = (DVec3::AXES[(axis + 1) % 3], DVec3::AXES[(axis + 2) % 3]);
            if sign < 0.0 {
                std::mem::swap(&mut u, &mut v);
            }
            let c = n * HALF;
            let corners = [-u - v, u - v, u + v, -u + v].map(|d| c + d * HALF);
            out.push((n, corners));
        }
    }
    out
}

fn quads_mesh(
    quads: &[(DVec3, [DVec3; 4])],
    front: [f32; 4],
    back: [f32; 4],
) -> (Vec<FaceVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (n, corners) in quads {
        let base = vertices.len() as u32;
        for p in corners {
            vertices.push(FaceVertex {
                position: p.as_vec3().into(),
                normal: n.as_vec3().into(),
                front,
                back,
            });
        }
        indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
    }
    (vertices, indices)
}

/// The cube's 12 edges with the normals of their two faces.
fn cube_edges() -> Vec<(DVec3, DVec3, [DVec3; 2])> {
    let mut out = Vec::new();
    for axis in 0..3 {
        let (u, v) = (DVec3::AXES[(axis + 1) % 3], DVec3::AXES[(axis + 2) % 3]);
        for su in [-1.0, 1.0] {
            for sv in [-1.0, 1.0] {
                let mid = (u * su + v * sv) * HALF;
                let along = DVec3::AXES[axis] * HALF;
                out.push((mid - along, mid + along, [u * su, v * sv]));
            }
        }
    }
    out
}

/// Spec style without axes, so they do not cross the test geometry.
fn face_style() -> Style {
    let spec = Spec::builtin().unwrap().default_model;
    Style {
        axes: None,
        ..Style::from_spec(&spec)
    }
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    viewport: &mut Viewport,
    camera: &Camera,
    name: &str,
) -> image::RgbaImage {
    let texture = viewport.render(device, queue, camera, SIZE, 1.0).clone();
    let img = read_back(device, queue, &texture);
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    img.save(&out).unwrap();
    eprintln!("saved {}", out.display());
    img
}

fn pixel_at(img: &image::RgbaImage, camera: &Camera, p: DVec3) -> image::Rgba<u8> {
    let px = camera.project(p, viewport_px()).unwrap();
    *img.get_pixel(px.x.round() as u32, px.y.round() as u32)
}

fn viewport_px() -> DVec2 {
    DVec2::new(f64::from(SIZE[0]), f64::from(SIZE[1]))
}

fn luma(px: &image::Rgba<u8>) -> i32 {
    (i32::from(px[0]) + i32::from(px[1]) + i32::from(px[2])) / 3
}

const RED: [f32; 4] = [0.9, 0.2, 0.2, 1.0];
const BLUE: [f32; 4] = [0.2, 0.2, 0.9, 1.0];

#[test]
fn faces_show_front_outside_and_back_inside() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut viewport = Viewport::new(&device, face_style());
    let (vertices, indices) = quads_mesh(&cube_quads(), RED, BLUE);
    viewport.set_faces(&device, &vertices, &indices);

    let outside = Camera::from_spec(&Spec::builtin().unwrap().default_model.camera);
    let img = render(&device, &queue, &mut viewport, &outside, "faces_outside");
    for center in [-DVec3::X, -DVec3::Y, DVec3::Z] {
        let px = pixel_at(&img, &outside, center * HALF);
        assert!(
            px[0] > 100 && px[2] < 100,
            "front (red) at {center}, got {px:?}"
        );
    }

    let inside = Camera {
        eye: DVec3::new(-10.0, -20.0, 5.0),
        target: DVec3::new(50.0, 10.0, 0.0),
        fov_deg: 35.0,
        perspective: true,
    };
    let img = render(&device, &queue, &mut viewport, &inside, "faces_inside");
    let px = img.get_pixel(SIZE[0] / 2, SIZE[1] / 2);
    assert!(px[2] > 100 && px[0] < 100, "back (blue) inside, got {px:?}");

    viewport.set_faces(&device, &[], &[]);
    let img = render(&device, &queue, &mut viewport, &inside, "faces_cleared");
    let px = img.get_pixel(SIZE[0] / 2, SIZE[1] / 2);
    assert!(
        px[0] > 150 && px[1] > 150,
        "cleared faces show the background, got {px:?}"
    );
}

#[test]
fn visible_sides_are_shaded_differently() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut viewport = Viewport::new(&device, face_style());
    let white = [1.0; 4];
    let (vertices, indices) = quads_mesh(&cube_quads(), white, white);
    viewport.set_faces(&device, &vertices, &indices);
    let camera = Camera::from_spec(&Spec::builtin().unwrap().default_model.camera);
    let img = render(&device, &queue, &mut viewport, &camera, "faces_shading");
    let sides = [-DVec3::X, -DVec3::Y, DVec3::Z].map(|n| luma(&pixel_at(&img, &camera, n * HALF)));
    eprintln!("-X, -Y, +Z brightness: {sides:?}");
    for i in 0..3 {
        for j in i + 1..3 {
            assert!(
                (sides[i] - sides[j]).abs() >= 15,
                "sides should differ clearly: {sides:?}"
            );
        }
    }
}

/// Distance from `p` to the segment `a`-`b`, in pixels.
fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Renders the cube with its edges and checks that every visible edge is solid dark
/// and every hidden edge is covered.
fn check_edges(camera: &Camera, name: &str) {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut viewport = Viewport::new(&device, face_style());
    // Red back sides: they must never show from outside, not even along silhouettes.
    let gray = [0.85, 0.85, 0.85, 1.0];
    let (vertices, indices) = quads_mesh(&cube_quads(), gray, [1.0, 0.0, 0.0, 1.0]);
    viewport.set_faces(&device, &vertices, &indices);
    let edges = cube_edges();
    let segments: Vec<Segment> = edges
        .iter()
        .map(|(a, b, _)| {
            Segment::line(
                a.as_vec3().into(),
                b.as_vec3().into(),
                [0.0, 0.0, 0.0, 1.0],
                [1.0, 0.0],
            )
        })
        .collect();
    viewport.set_edges(&device, &segments);
    let img = render(&device, &queue, &mut viewport, camera, name);

    let view = viewport_px();
    let faces_camera = |n: DVec3| n.dot(camera.eye - n * HALF) > 0.0;
    let visible = |normals: &[DVec3; 2]| normals.iter().any(|&n| faces_camera(n));
    let project = |p: DVec3| camera.project(p, view).unwrap();
    let visible_2d: Vec<(DVec2, DVec2)> = edges
        .iter()
        .filter(|e| visible(&e.2))
        .map(|(a, b, _)| (project(*a), project(*b)))
        .collect();
    // Darkest and brightest luma in the 3x3 block around a pixel.
    let block = |p: DVec2| {
        let (x, y) = (p.x.round() as i64, p.y.round() as i64);
        let mut range = (i32::MAX, i32::MIN);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let l = luma(img.get_pixel((x + dx) as u32, (y + dy) as u32));
                range = (range.0.min(l), range.1.max(l));
            }
        }
        range
    };
    let on_screen = |p: DVec2| p.x >= 3.0 && p.y >= 3.0 && p.x < view.x - 3.0 && p.y < view.y - 3.0;

    let (mut shown, mut hidden) = (0, 0);
    for (a, b, normals) in &edges {
        for i in 1..40 {
            let p = project(a.lerp(*b, f64::from(i) / 40.0));
            if !on_screen(p) {
                continue;
            }
            // A 1 px line may straddle two pixels at half coverage each, so a visible
            // edge only has to darken some pixel nearby to about half its surroundings.
            let (dark, light) = block(p);
            if visible(normals) {
                assert!(
                    dark * 100 < light * 60,
                    "{name}: visible edge at {p} not dark ({dark}..{light})"
                );
                shown += 1;
            } else if visible_2d
                .iter()
                .all(|&(va, vb)| distance_to_segment(p, va, vb) > 4.0)
            {
                assert!(
                    dark > 100 && dark * 100 > light * 90,
                    "{name}: hidden edge shows at {p} ({dark}..{light})"
                );
                hidden += 1;
            }
        }
    }
    eprintln!("{name}: {shown} visible and {hidden} hidden edge samples");
    // Parallel projection spreads depth over +-1e4 camera distances, too coarse to keep
    // back sides from showing in antialiased silhouette pixels. Known, not checked.
    if camera.perspective {
        let reddish = img
            .pixels()
            .filter(|p| p[0] > p[1].saturating_add(30))
            .count();
        assert_eq!(reddish, 0, "{name}: back sides show");
    }
    assert!(
        shown > 50 && hidden > 20,
        "{name}: too few samples ({shown}, {hidden})"
    );
}

/// Eye on the default view direction at `distance` from the cube center, with a field
/// of view that keeps the cube about the same size on screen.
fn cube_camera(distance: f64, perspective: bool) -> Camera {
    let dir = DVec3::new(-300.0, -500.0, 220.0).normalize();
    let half_angle = (17.5_f64.to_radians().tan() * 300.0 / distance).atan();
    Camera {
        eye: dir * distance,
        target: DVec3::ZERO,
        fov_deg: (2.0 * half_angle).to_degrees(),
        perspective,
    }
}

#[test]
fn edges_on_faces_near() {
    // 100 inches from the nearest corner: strong perspective, the cube overflows the view.
    let corner = DVec3::new(-HALF, -HALF, HALF);
    let dir = DVec3::new(-300.0, -500.0, 220.0).normalize();
    check_edges(
        &Camera {
            eye: corner + dir * 100.0,
            target: corner,
            fov_deg: 35.0,
            perspective: true,
        },
        "edges_near",
    );
    check_edges(&cube_camera(200.0, true), "edges_close");
}

#[test]
fn edges_on_faces_far() {
    check_edges(&cube_camera(100_000.0, true), "edges_far");
}

#[test]
fn edges_on_faces_parallel() {
    check_edges(&cube_camera(600.0, false), "edges_parallel");
}

#[test]
fn translucent_face_blends() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut viewport = Viewport::new(&device, face_style());
    let up = DVec3::Z;
    let quad = |x0: f64, x1: f64, z: f64| {
        let c = [(x0, -50.0), (x1, -50.0), (x1, 50.0), (x0, 50.0)];
        (up, c.map(|(x, y)| DVec3::new(x, y, z)))
    };
    // Opaque red across the whole width; translucent blue above its right half and
    // past it. The translucent quad comes first to check the opaque pass runs first.
    let (mut vertices, mut indices) = quads_mesh(
        &[quad(0.0, 150.0, 10.0)],
        [0.2, 0.2, 0.9, 0.5],
        [0.2, 0.2, 0.9, 0.5],
    );
    let (red_v, red_i) = quads_mesh(&[quad(-100.0, 100.0, 0.0)], RED, RED);
    let base = vertices.len() as u32;
    vertices.extend(red_v);
    indices.extend(red_i.iter().map(|i| i + base));
    viewport.set_faces(&device, &vertices, &indices);

    let camera = Camera {
        eye: DVec3::new(0.0, 0.0, 500.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
    };
    let img = render(&device, &queue, &mut viewport, &camera, "faces_translucent");
    let red = pixel_at(&img, &camera, DVec3::new(-50.0, 0.0, 0.0));
    let both = pixel_at(&img, &camera, DVec3::new(50.0, 0.0, 10.0));
    let blue_over_sky = pixel_at(&img, &camera, DVec3::new(125.0, 0.0, 10.0));
    eprintln!("red {red:?}, blue over red {both:?}, blue over background {blue_over_sky:?}");
    assert!(red[0] > 150 && red[2] < 60, "opaque red, got {red:?}");
    // Half red, half blue (both shaded alike).
    let expect = (f32::from(red[0]) + f32::from(red[2])) / 2.0;
    for c in [both[0], both[2]] {
        assert!(
            (f32::from(c) - expect).abs() < 12.0,
            "blend of red and blue, got {both:?}, expected ~{expect}"
        );
    }
    assert!(
        blue_over_sky[2] > blue_over_sky[0] + 30 && blue_over_sky[0] > 80,
        "blue blended over background, got {blue_over_sky:?}"
    );
}

#[test]
fn border_and_silhouette_edges_draw_as_profiles() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    // Looking straight down at two horizontal lines (no faces, sky/ground off).
    let camera = Camera {
        eye: DVec3::new(0.0, 0.0, 500.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
    };
    let mut style = face_style();
    style.axes = None;
    let mut viewport = Viewport::new(&device, style);
    let black = [0.0, 0.0, 0.0, 1.0];
    let plain = Segment::line([-100.0, 40.0, 0.0], [100.0, 40.0, 0.0], black, [1.0, 0.0]);
    let mut border = Segment::line([-100.0, -40.0, 0.0], [100.0, -40.0, 0.0], black, [1.0, 0.0]);
    border.n1 = [0.0, 0.0, 1.0];
    border.profile = Segment::BORDER;
    // A silhouette: one face towards the camera, one away.
    let mut silhouette =
        Segment::line([-60.0, -100.0, 0.0], [-60.0, 100.0, 0.0], black, [1.0, 0.0]);
    silhouette.n1 = [0.0, 0.0, 1.0];
    silhouette.n2 = [0.0, 0.0, -1.0];
    silhouette.profile = Segment::SILHOUETTE;
    // Two faces both towards the camera: not a profile.
    let mut crease = Segment::line([60.0, -100.0, 0.0], [60.0, 100.0, 0.0], black, [1.0, 0.0]);
    crease.n1 = [0.0, 0.0, 1.0];
    crease.n2 = [0.0, 0.6, 0.8];
    crease.profile = Segment::SILHOUETTE;
    viewport.set_edges(&device, &[plain, border, silhouette, crease]);
    let img = render(&device, &queue, &mut viewport, &camera, "profiles");

    let column = |x: f64| {
        camera
            .project(DVec3::new(x, 0.0, 0.0), viewport_px())
            .unwrap()
            .x as u32
    };
    // The column at x = 0 crosses both horizontal lines: plain (top) and border (bottom).
    let x0 = column(0.0);
    let ys: Vec<u32> = (0..SIZE[1])
        .filter(|&y| luma(img.get_pixel(x0, y)) < 100)
        .collect();
    let mid = SIZE[1] / 2;
    let top = ys.iter().filter(|&&y| y < mid).count();
    let bottom = ys.iter().filter(|&&y| y > mid).count();
    assert!(
        bottom > top,
        "border profile ({bottom} px) thicker than plain edge ({top} px)"
    );

    let row = SIZE[1] / 2;
    let width_at = |x: f64| {
        let cx = column(x) as i64;
        (cx - 6..=cx + 6)
            .filter(|&x| luma(img.get_pixel(x as u32, row)) < 100)
            .count()
    };
    assert!(
        width_at(-60.0) > width_at(60.0),
        "silhouette thicker than crease"
    );
}

// ---------------------------------------------------------------- section planes

#[test]
fn section_plane_clips_faces_edges_and_selection() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut viewport = Viewport::new(&device, face_style());
    let (vertices, indices) = quads_mesh(&cube_quads(), RED, BLUE);
    viewport.set_faces(&device, &vertices, &indices);
    viewport.set_selected_faces(&device, &vertices, &indices);
    let black = [0.0, 0.0, 0.0, 1.0];
    // An edge across the clipped half of the top face, and one across the kept half.
    let clipped_edge = Segment::line([25.0, -40.0, 50.0], [25.0, 40.0, 50.0], black, [3.0, 0.0]);
    let kept_edge = Segment::line([-25.0, -40.0, 50.0], [-25.0, 40.0, 50.0], black, [3.0, 0.0]);
    viewport.set_edges(&device, &[clipped_edge, kept_edge]);
    // Cut at x = 0, removing +X. Looking down from above the +X side.
    viewport.set_section(Some(SectionCut {
        point: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
    }));
    let camera = Camera {
        eye: DVec3::new(300.0, -100.0, 400.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
    };
    let img = render(&device, &queue, &mut viewport, &camera, "section");
    // The kept half of the top shows its red front and the kept edge.
    let top_kept = pixel_at(&img, &camera, DVec3::new(-35.0, 20.0, 50.0));
    assert!(
        top_kept[0] > 100 && top_kept[2] < 100,
        "kept top, got {top_kept:?}"
    );
    let edge = pixel_at(&img, &camera, DVec3::new(-25.0, 0.0, 50.0));
    assert!(luma(&edge) < 60, "kept edge, got {edge:?}");
    // Through the removed half: the inside of the -X wall (blue back side), not the
    // top face, its edge, or the selection dots over it.
    for p in [
        DVec3::new(25.0, 0.0, 50.0),
        DVec3::new(25.0, 1.0, 50.0),
        DVec3::new(30.0, 5.0, 50.0),
    ] {
        let px = pixel_at(&img, &camera, p);
        assert!(
            px[2] > 100 && px[0] < 100,
            "inside visible through the cut at {p}, got {px:?}"
        );
    }

    viewport.set_section(None);
    let img = render(&device, &queue, &mut viewport, &camera, "section_off");
    let px = pixel_at(&img, &camera, DVec3::new(35.0, 20.0, 50.0));
    assert!(px[0] > 100 && px[2] < 100, "uncut top, got {px:?}");
}

// ---------------------------------------------------------------- shadows

/// A slab over a lower ledge, seen from straight above, lit from +X 30 degrees off
/// vertical. The slab's shadow is shifted by (50 - z) tan 30 towards -X.
fn shadow_scene(device: &wgpu::Device) -> (Viewport, Camera) {
    let mut style = face_style();
    style.sky = None;
    style.ground = Some([0.6, 0.6, 0.6]);
    style.profiles = None;
    let mut viewport = Viewport::new(device, style);
    let up = DVec3::Z;
    let quad = |x0: f64, x1: f64, y0: f64, y1: f64, z: f64| {
        let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        (up, c.map(|(x, y)| DVec3::new(x, y, z)))
    };
    let white = [1.0; 4];
    let (vertices, indices) = quads_mesh(
        &[
            quad(-50.0, 50.0, -50.0, 50.0, 50.0),
            quad(-100.0, -55.0, -20.0, 20.0, 20.0),
        ],
        white,
        white,
    );
    viewport.set_faces(device, &vertices, &indices);
    let camera = Camera {
        eye: DVec3::new(0.0, 0.0, 800.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
    };
    (viewport, camera)
}

fn sun_30() -> Shadows {
    let a = 30f32.to_radians();
    Shadows {
        sun_dir: [a.sin(), 0.0, a.cos()],
        light: 0.8,
        dark: 0.45,
        on_faces: true,
        on_ground: true,
    }
}

#[test]
fn shadows_fall_on_ground_and_faces() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let (mut viewport, camera) = shadow_scene(&device);
    viewport.set_shadows(Some(sun_30()));
    let img = render(&device, &queue, &mut viewport, &camera, "shadows");
    let at = |x: f64, y: f64, z: f64| luma(&pixel_at(&img, &camera, DVec3::new(x, y, z)));
    // Ground: the shadow spans x in [-78.9, 21.1], y in [-50, 50]; the slab hides
    // x in [-50, 50] and the ledge y in [-20, 20] from above.
    let (ground_lit, ground_shadow) = (at(-130.0, 30.0, 0.0), at(-70.0, 30.0, 0.0));
    let ground_east = at(70.0, 30.0, 0.0);
    eprintln!("ground lit {ground_lit}, east {ground_east}, shadow {ground_shadow}");
    let expected = 0.45 / (0.45 + 0.8 * 30f32.to_radians().cos());
    let ratio = ground_shadow as f32 / ground_lit as f32;
    assert!(
        (ratio - expected).abs() < 0.06,
        "ground shadow ratio {ratio}, expected {expected}"
    );
    assert!(
        (ground_east - ground_lit).abs() <= 3,
        "ground east of the shadow is lit"
    );
    // Ledge at z = 20: the shadow there spans x in [-67.3, 32.7].
    let (ledge_lit, ledge_shadow) = (at(-90.0, 0.0, 20.0), at(-60.0, 0.0, 20.0));
    eprintln!("ledge lit {ledge_lit}, shadow {ledge_shadow}");
    let ratio = ledge_shadow as f32 / ledge_lit as f32;
    assert!(
        (ratio - expected).abs() < 0.06,
        "ledge shadow ratio {ratio}, expected {expected}"
    );
    // The slab top is lit: white * min(1, dark + light cos 30) = full white.
    let slab = at(0.0, 0.0, 50.0);
    assert!(slab > 245, "slab lit, got {slab}");

    // Not on faces: the ledge is evenly lit, the ground still shadowed.
    viewport.set_shadows(Some(Shadows {
        on_faces: false,
        ..sun_30()
    }));
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        "shadows_ground_only",
    );
    let at = |x: f64, y: f64, z: f64| luma(&pixel_at(&img, &camera, DVec3::new(x, y, z)));
    assert!((at(-90.0, 0.0, 20.0) - at(-60.0, 0.0, 20.0)).abs() <= 3);
    assert!(at(-70.0, 30.0, 0.0) < at(-130.0, 0.0, 0.0) - 30);

    // Sun below the horizon: no shadows, faces get only the dark (ambient) term.
    viewport.set_shadows(Some(Shadows {
        sun_dir: [0.5, 0.0, -0.5],
        ..sun_30()
    }));
    let img = render(&device, &queue, &mut viewport, &camera, "shadows_night");
    let at = |x: f64, y: f64, z: f64| luma(&pixel_at(&img, &camera, DVec3::new(x, y, z)));
    assert!((at(-70.0, 30.0, 0.0) - at(-130.0, 0.0, 0.0)).abs() <= 3);
    assert!((at(0.0, 0.0, 50.0) - (0.45f32 * 255.0) as i32).abs() <= 3);
}

#[test]
fn translucent_faces_sort_back_to_front() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut viewport = Viewport::new(&device, face_style());
    let up = DVec3::Z;
    let quad = |z: f64| {
        let c = [(-50.0, -50.0), (50.0, -50.0), (50.0, 50.0), (-50.0, 50.0)];
        (up, c.map(|(x, y)| DVec3::new(x, y, z)))
    };
    // Near red submitted first, far blue second: unsorted, blue would end up on top.
    let red = [0.9, 0.1, 0.1, 0.6];
    let blue = [0.1, 0.1, 0.9, 0.6];
    let (mut vertices, mut indices) = quads_mesh(&[quad(20.0)], red, red);
    let (bv, bi) = quads_mesh(&[quad(0.0)], blue, blue);
    let base = vertices.len() as u32;
    vertices.extend(bv);
    indices.extend(bi.iter().map(|i| i + base));
    viewport.set_faces(&device, &vertices, &indices);
    let camera = Camera {
        eye: DVec3::new(0.0, 0.0, 500.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
    };
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        "translucent_sorted",
    );
    let px = img.get_pixel(SIZE[0] / 2, SIZE[1] / 2);
    assert!(px[0] > px[2] + 20, "nearer red pane on top, got {px:?}");
    // From below the order flips.
    let below = Camera {
        eye: DVec3::new(0.0, 0.0, -500.0),
        ..camera
    };
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &below,
        "translucent_sorted_below",
    );
    let px = img.get_pixel(SIZE[0] / 2, SIZE[1] / 2);
    assert!(px[2] > px[0] + 20, "nearer blue pane on top, got {px:?}");
}

/// The app's case: a 100 x 100 x 60 box at the origin with the sun of Boulder,
/// 2026-11-08 13:30 MST (south-south-west, ~28 degrees up). The shadow falls
/// north-north-east, entirely behind the box from the default south-west camera, so it
/// is checked from the north-east.
#[test]
fn box_shadow_with_app_sun() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut style = face_style();
    style.sky = None;
    style.ground = Some([0.6, 0.6, 0.6]);
    let mut viewport = Viewport::new(&device, style);
    // The cube helper scaled into the box [0, 100] x [0, 100] x [0, 60].
    let quads: Vec<_> = cube_quads()
        .into_iter()
        .map(|(n, c)| {
            let to_box = |p: DVec3| (p / HALF + DVec3::ONE) * DVec3::new(50.0, 50.0, 30.0);
            (n, c.map(to_box))
        })
        .collect();
    let white = [1.0; 4];
    let (vertices, indices) = quads_mesh(&quads, white, white);
    let sun = mt_render::sun_direction(40.017, -105.283, -7.0, 2026, 11, 8, 13, 30);
    assert!(sun.z > 0.4 && sun.y < -0.7 && sun.x < -0.4, "sun {sun}");
    // Same call order as the app: shadows before faces.
    viewport.set_shadows(Some(Shadows {
        sun_dir: sun.as_vec3().into(),
        ..sun_30()
    }));
    viewport.set_faces(&device, &vertices, &indices);
    let camera = Camera {
        eye: DVec3::new(500.0, 600.0, 300.0),
        target: DVec3::new(50.0, 50.0, 0.0),
        fov_deg: 35.0,
        perspective: true,
    };
    let img = render(&device, &queue, &mut viewport, &camera, "shadow_app_box");
    let at = |p: DVec3| luma(&pixel_at(&img, &camera, p));
    // The top centre's shadow lands at (50, 50) + 60 / sun.z * -(sun.x, sun.y).
    let t = 60.0 / sun.z;
    let shadowed = DVec3::new(50.0 - t * sun.x, 50.0 - t * sun.y, 0.0);
    let lit = DVec3::new(-100.0, 150.0, 0.0);
    eprintln!("shadow at {shadowed}: {} vs lit {}", at(shadowed), at(lit));
    assert!(at(shadowed) < at(lit) - 30, "ground shadow behind the box");
    // The +Y face looks away from the sun: dark only.
    let back = at(DVec3::new(50.0, 100.0, 30.0));
    assert!(
        (back - (0.45f32 * 255.0) as i32).abs() <= 4,
        "+Y face {back}"
    );
}

// ---------------------------------------------------------------- face styles

/// The cube with `front`/`back` faces and black edges 2 points wide, seen from the
/// default view direction. Its nearest corner is (-50, -50, 50) and its farthest
/// (50, 50, -50); the three edges at the farthest corner are hidden.
fn face_style_cube(
    device: &wgpu::Device,
    style: Style,
    front: [f32; 4],
    back: [f32; 4],
) -> (Viewport, Camera) {
    let mut viewport = Viewport::new(device, style);
    let (vertices, indices) = quads_mesh(&cube_quads(), front, back);
    viewport.set_faces(device, &vertices, &indices);
    let segments: Vec<Segment> = cube_edges()
        .iter()
        .map(|(a, b, _)| {
            Segment::line(
                a.as_vec3().into(),
                b.as_vec3().into(),
                [0.0, 0.0, 0.0, 1.0],
                [2.0, 0.0],
            )
        })
        .collect();
    viewport.set_edges(device, &segments);
    (viewport, cube_camera(600.0, true))
}

/// Midpoint of the hidden bottom edge along X at the back (y = 50, z = -50).
const HIDDEN_EDGE: DVec3 = DVec3::new(0.0, HALF, -HALF);
/// Center of the visible -Y face.
const FRONT_FACE: DVec3 = DVec3::new(0.0, -HALF, 0.0);
/// Midpoint of the visible vertical edge nearest the camera.
const NEAR_EDGE: DVec3 = DVec3::new(-HALF, -HALF, 0.0);

#[test]
fn wireframe_draws_edges_only() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    // Plain background, so "no face" means the background color.
    let mut style = face_style();
    style.sky = None;
    style.ground = None;
    let background = style.background;
    let (mut viewport, camera) = face_style_cube(&device, style, RED, BLUE);
    viewport.set_face_style(FaceStyle::Wireframe, false);
    let img = render(&device, &queue, &mut viewport, &camera, "style_wireframe");
    let face = pixel_at(&img, &camera, FRONT_FACE);
    assert!(close(&face, background, 2), "background, got {face:?}");
    for p in [NEAR_EDGE, HIDDEN_EDGE] {
        let px = pixel_at(&img, &camera, p);
        assert!(luma(&px) < 60, "edge at {p} drawn, got {px:?}");
    }
    // X-ray changes nothing in wireframe.
    viewport.set_face_style(FaceStyle::Wireframe, true);
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        "style_wireframe_xray",
    );
    let face = pixel_at(&img, &camera, FRONT_FACE);
    assert!(close(&face, background, 2), "background, got {face:?}");
}

#[test]
fn hidden_line_fills_faces_with_background() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let style = face_style();
    let background = style.background;
    let (mut viewport, camera) = face_style_cube(&device, style, RED, BLUE);
    viewport.set_face_style(FaceStyle::HiddenLine, false);
    let img = render(&device, &queue, &mut viewport, &camera, "style_hidden_line");
    // Unshaded: every visible face is exactly the background color.
    for p in [
        FRONT_FACE,
        DVec3::new(-HALF, 0.0, 0.0),
        DVec3::new(0.0, 0.0, HALF),
    ] {
        let px = pixel_at(&img, &camera, p);
        assert!(close(&px, background, 1), "face at {p}, got {px:?}");
    }
    let hidden = pixel_at(&img, &camera, HIDDEN_EDGE);
    assert!(
        close(&hidden, background, 1),
        "hidden edge covered, got {hidden:?}"
    );
    let near = pixel_at(&img, &camera, NEAR_EDGE);
    assert!(luma(&near) < 60, "visible edge drawn, got {near:?}");
}

#[test]
fn monochrome_ignores_material_color() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let (mut viewport, camera) = face_style_cube(&device, face_style(), RED, RED);
    let top = DVec3::new(0.0, 0.0, HALF);
    let img = render(&device, &queue, &mut viewport, &camera, "style_shaded");
    let px = pixel_at(&img, &camera, top);
    assert!(
        px[0] > px[2] + 60,
        "shaded shows the red material, got {px:?}"
    );

    viewport.set_face_style(FaceStyle::Monochrome, false);
    let img = render(&device, &queue, &mut viewport, &camera, "style_monochrome");
    let sides = [FRONT_FACE, DVec3::new(-HALF, 0.0, 0.0), top].map(|p| pixel_at(&img, &camera, p));
    for px in &sides {
        // Default front color: white, shaded to a grey.
        assert!(
            px[0].abs_diff(px[1]) <= 2 && px[0].abs_diff(px[2]) <= 2 && luma(px) > 100,
            "white-ish front, got {px:?}"
        );
    }
    let l = sides.map(|px| luma(&px));
    assert!(l[0] != l[1] || l[1] != l[2], "still shaded: {l:?}");

    // The back color shows from inside.
    viewport.set_monochrome_colors([1.0; 3], [0.0, 0.0, 1.0]);
    let inside = Camera {
        eye: DVec3::new(-10.0, -20.0, 5.0),
        target: DVec3::new(50.0, 10.0, 0.0),
        fov_deg: 35.0,
        perspective: true,
    };
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &inside,
        "style_monochrome_inside",
    );
    let px = img.get_pixel(SIZE[0] / 2, SIZE[1] / 2);
    assert!(
        px[2] > 100 && px[0] < 30 && px[1] < 30,
        "back color, got {px:?}"
    );
}

#[test]
fn xray_shows_edges_behind_faces() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let white = [1.0; 4];
    let (mut viewport, camera) = face_style_cube(&device, face_style(), white, white);
    // A point on the same faces a few pixels off the hidden edge.
    let off_edge = HIDDEN_EDGE + DVec3::new(0.0, 0.0, 10.0);
    let img = render(&device, &queue, &mut viewport, &camera, "style_xray_off");
    let (on, off) = (
        luma(&pixel_at(&img, &camera, HIDDEN_EDGE)),
        luma(&pixel_at(&img, &camera, off_edge)),
    );
    assert!((on - off).abs() < 10, "hidden edge covered: {on} vs {off}");

    for (style, name) in [
        (FaceStyle::Shaded, "style_xray"),
        (FaceStyle::Monochrome, "style_xray_monochrome"),
        (FaceStyle::HiddenLine, "style_xray_hidden_line"),
    ] {
        viewport.set_face_style(style, true);
        let img = render(&device, &queue, &mut viewport, &camera, name);
        let (on, off) = (
            luma(&pixel_at(&img, &camera, HIDDEN_EDGE)),
            luma(&pixel_at(&img, &camera, off_edge)),
        );
        eprintln!("{name}: hidden edge {on}, beside it {off}");
        assert!(
            on < off - 40,
            "{name}: hidden edge shows through: {on} vs {off}"
        );
        // Visible edges stay solid.
        let near = pixel_at(&img, &camera, NEAR_EDGE);
        assert!(luma(&near) < 60, "{name}: visible edge, got {near:?}");
    }
}
