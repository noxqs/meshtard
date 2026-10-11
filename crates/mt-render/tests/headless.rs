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
        two_point: false,
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
            vertices.push(FaceVertex::new(
                p.as_vec3().into(),
                n.as_vec3().into(),
                front,
                back,
            ));
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

/// Spec style without axes, so they do not cross the test geometry, and without the
/// ground, which hides the lower half of the test cubes (they straddle z = 0).
fn face_style() -> Style {
    let spec = Spec::builtin().unwrap().default_model;
    Style {
        axes: None,
        ground: None,
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
        two_point: false,
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
    // The ground color behind the cube (as when the ground was drawn behind
    // everything): edges against the lighter sky are too little contrast for the
    // half-coverage check.
    let ground = Spec::builtin().unwrap().default_model.ground.color;
    let style = Style {
        sky: None,
        background: ground,
        ..face_style()
    };
    let mut viewport = Viewport::new(&device, style);
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
        two_point: false,
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
            two_point: false,
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
        two_point: false,
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
        two_point: false,
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
        two_point: false,
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

#[test]
fn section_fill_caps_a_cut_cube() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut viewport = Viewport::new(&device, face_style());
    let quads = cube_quads();
    let (vertices, indices) = quads_mesh(&quads, RED, BLUE);
    viewport.set_faces(&device, &vertices, &indices);
    let tris: Vec<[DVec3; 3]> = quads
        .iter()
        .flat_map(|(_, c)| [[c[0], c[1], c[2]], [c[0], c[2], c[3]]])
        .collect();
    // Cut at x = 0 removing +X, seen from the +X side: the cut face shows the fill.
    let (point, normal) = (DVec3::ZERO, DVec3::X);
    viewport.set_section(Some(SectionCut {
        point: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
    }));
    let fill: Vec<[[f32; 3]; 3]> = mt_render::section_fill(&tris, point, normal)
        .iter()
        .map(|t| t.map(|p| p.as_vec3().to_array()))
        .collect();
    assert!(!fill.is_empty());
    let grey = [0.25, 0.25, 0.25];
    viewport.set_section_fill(&device, &fill, grey);
    let camera = Camera {
        eye: DVec3::new(400.0, -150.0, 120.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
        two_point: false,
    };
    let img = render(&device, &queue, &mut viewport, &camera, "section_fill");
    for p in [
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(0.0, 40.0, 40.0),
        DVec3::new(0.0, -40.0, -40.0),
    ] {
        let px = pixel_at(&img, &camera, p);
        assert!(close(&px, grey, 3), "fill at {p}, got {px:?}");
    }
    // The kept side face is still drawn next to the fill.
    let side = pixel_at(&img, &camera, DVec3::new(-25.0, -50.0, 0.0));
    assert!(side[0] > 100 && side[2] < 100, "front side, got {side:?}");

    viewport.set_section_fill(&device, &[], grey);
    let img = render(&device, &queue, &mut viewport, &camera, "section_fill_off");
    let px = pixel_at(&img, &camera, DVec3::ZERO);
    assert!(!close(&px, grey, 10), "no fill, got {px:?}");
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
        two_point: false,
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
        two_point: false,
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
        two_point: false,
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
        two_point: false,
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

/// A 64x64 PNG: red top-left, green top-right, blue bottom-left, yellow bottom-right.
fn quadrant_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba(match (x < 32, y < 32) {
            (true, true) => [255, 0, 0, 255],
            (false, true) => [0, 255, 0, 255],
            (true, false) => [0, 0, 255, 255],
            (false, false) => [255, 255, 0, 255],
        })
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn textured_faces_show_texture_pixels() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let grey = [0.5, 0.5, 0.5, 1.0];
    let (mut viewport, camera) = face_style_cube(&device, face_style(), grey, grey);
    // One tile per cube side, front texture 0; corners go (0,0) (1,0) (1,1) (0,1).
    let (mut vertices, indices) = quads_mesh(&cube_quads(), grey, grey);
    for (i, v) in vertices.iter_mut().enumerate() {
        let [u, w] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][i % 4];
        v.uv = [u, w, 0.0, 0.0];
        v.texture = [0, FaceVertex::NO_TEXTURE];
    }
    viewport.set_faces(&device, &vertices, &indices);
    viewport.set_textures(&[mt_render::TextureSource {
        data: quadrant_png().into(),
        tint: None,
    }]);

    // Shaded keeps the material color.
    let img = render(&device, &queue, &mut viewport, &camera, "textures_shaded");
    let px = pixel_at(&img, &camera, DVec3::new(-25.0, -HALF, 25.0));
    assert!(
        px[0].abs_diff(px[1]) <= 2 && px[1].abs_diff(px[2]) <= 2,
        "grey material, got {px:?}"
    );

    // The -Y side runs u along +X and v up +Z: the image's top-left quadrant is at
    // the upper left as seen from the front.
    viewport.set_face_style(FaceStyle::ShadedWithTextures, false);
    let img = render(&device, &queue, &mut viewport, &camera, "textures_shown");
    let at = |x: f64, z: f64| pixel_at(&img, &camera, DVec3::new(x, -HALF, z));
    let dominant = |px: image::Rgba<u8>, rgb: [bool; 3]| {
        (0..3).all(|i| if rgb[i] { px[i] > 100 } else { px[i] < 30 })
    };
    let red = at(-25.0, 25.0);
    let green = at(25.0, 25.0);
    let blue = at(-25.0, -25.0);
    let yellow = at(25.0, -25.0);
    assert!(dominant(red, [true, false, false]), "red, got {red:?}");
    assert!(
        dominant(green, [false, true, false]),
        "green, got {green:?}"
    );
    assert!(dominant(blue, [false, false, true]), "blue, got {blue:?}");
    assert!(
        dominant(yellow, [true, true, false]),
        "yellow, got {yellow:?}"
    );
}

// ---------------------------------------------------------------- edge style and fog

/// The cube with model edges (profiles flagged as the app does), 1 point wide.
fn model_edge_cube(device: &wgpu::Device, style: Style) -> (Viewport, Camera) {
    let mut viewport = Viewport::new(device, style);
    let white = [1.0; 4];
    let (vertices, indices) = quads_mesh(&cube_quads(), white, white);
    viewport.set_faces(device, &vertices, &indices);
    let segments: Vec<Segment> = cube_edges()
        .iter()
        .map(|(a, b, n)| model_edge(*a, *b, *n))
        .collect();
    viewport.set_edges(device, &segments);
    (viewport, cube_camera(600.0, true))
}

fn model_edge(a: DVec3, b: DVec3, n: [DVec3; 2]) -> Segment {
    let mut s = Segment::line(
        a.as_vec3().into(),
        b.as_vec3().into(),
        [0.0, 0.0, 0.0, 1.0],
        [1.0, 0.0],
    );
    s.n1 = n[0].as_vec3().into();
    s.n2 = n[1].as_vec3().into();
    s.profile = Segment::SILHOUETTE;
    s.flags = Segment::MODEL_EDGE;
    s
}

/// Dark pixels among the pixels along the projected segment `a`-`b`, between 30% and
/// 70% of its length (away from the visible edges at its ends), and the sample count.
fn dark_along(img: &image::RgbaImage, camera: &Camera, a: DVec3, b: DVec3) -> (usize, usize) {
    let pa = camera.project(a, viewport_px()).unwrap();
    let pb = camera.project(b, viewport_px()).unwrap();
    let n = (pa.distance(pb) * 0.4).round() as usize;
    let dark = (0..n)
        .filter(|&i| {
            let p = pa.lerp(pb, 0.3 + 0.4 * i as f64 / n as f64);
            luma(img.get_pixel(p.x.round() as u32, p.y.round() as u32)) < 150
        })
        .count();
    (dark, n)
}

#[test]
fn back_edges_draw_hidden_edges_dashed() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut style = face_style();
    style.profiles = None;
    let (mut viewport, camera) = model_edge_cube(&device, style.clone());
    viewport.set_face_style(FaceStyle::HiddenLine, false);
    let a = HIDDEN_EDGE - DVec3::X * HALF;
    let b = HIDDEN_EDGE + DVec3::X * HALF;
    let img = render(&device, &queue, &mut viewport, &camera, "back_edges_off");
    let (dark, n) = dark_along(&img, &camera, a, b);
    // It crosses the near vertical edge once.
    assert!(dark <= 2, "hidden edge covered ({dark}/{n})");

    style.back_edges = true;
    viewport.set_style(&device, style);
    let img = render(&device, &queue, &mut viewport, &camera, "back_edges_on");
    let (dark, n) = dark_along(&img, &camera, a, b);
    assert!(
        dark * 5 > n && dark * 5 < n * 4,
        "hidden edge dashed: {dark}/{n} dark"
    );
    // Visible edges stay solid.
    let near = pixel_at(&img, &camera, NEAR_EDGE);
    assert!(luma(&near) < 100, "visible edge, got {near:?}");
}

#[test]
fn edges_off_keeps_only_profiles() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut style = face_style();
    style.edges = false;
    let (mut viewport, camera) = model_edge_cube(&device, style.clone());
    let img = render(&device, &queue, &mut viewport, &camera, "edges_off");
    // The near vertical edge has both faces towards the camera: not a profile.
    let near = pixel_at(&img, &camera, NEAR_EDGE);
    assert!(luma(&near) > 100, "non-profile edge hidden, got {near:?}");
    // The top's far edge along X (y = 50, z = 50) is on the silhouette.
    let silhouette = DVec3::new(0.0, HALF, HALF);
    let (dark, n) = dark_along(
        &img,
        &camera,
        silhouette - DVec3::X * HALF,
        silhouette + DVec3::X * HALF,
    );
    assert!(dark * 10 > n * 9, "profile drawn: {dark}/{n}");

    // Profiles off too: nothing.
    style.profiles = None;
    viewport.set_style(&device, style);
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        "edges_profiles_off",
    );
    let (dark, n) = dark_along(
        &img,
        &camera,
        silhouette - DVec3::X * HALF,
        silhouette + DVec3::X * HALF,
    );
    assert_eq!(dark, 0, "no edges: {dark}/{n}");
}

/// Looking straight down at horizontal model lines without faces.
fn top_down() -> Camera {
    Camera {
        eye: DVec3::new(0.0, 0.0, 500.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
        two_point: false,
    }
}

fn flat_line(a: [f32; 3], b: [f32; 3]) -> Segment {
    let mut s = Segment::line(a, b, [0.0, 0.0, 0.0, 1.0], [1.0, 0.0]);
    s.flags = Segment::MODEL_EDGE;
    s
}

#[test]
fn extension_overshoots_line_ends() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let camera = top_down();
    let mut style = face_style();
    style.sky = None;
    style.ground = None;
    let mut viewport = Viewport::new(&device, style.clone());
    viewport.set_edges(&device, &[flat_line([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0])]);
    let end = camera
        .project(DVec3::new(50.0, 0.0, 0.0), viewport_px())
        .unwrap();
    // 4 pixels past the end, on the line (darkest of three rows: the line is 1 px).
    let past = |img: &image::RgbaImage| {
        (-1..=1)
            .map(|dy| luma(img.get_pixel(end.x as u32 + 4, (end.y.round() as i32 + dy) as u32)))
            .min()
            .unwrap()
    };
    let img = render(&device, &queue, &mut viewport, &camera, "extension_off");
    assert!(past(&img) > 200, "no overshoot: {}", past(&img));
    style.extension = Some(8.0);
    viewport.set_style(&device, style);
    let img = render(&device, &queue, &mut viewport, &camera, "extension_on");
    // Half covered at least (a 1 px line on a pixel boundary covers two rows by half).
    assert!(past(&img) < 160, "overshoot: {}", past(&img));
}

#[test]
fn depth_cue_draws_near_edges_wider() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let camera = top_down();
    let mut style = face_style();
    style.sky = None;
    style.ground = None;
    style.depth_cue = Some(6.0);
    let mut viewport = Viewport::new(&device, style);
    // Near line (z = 200) at y > 0, far line (z = -200) at y < 0.
    viewport.set_edges(
        &device,
        &[
            flat_line([-50.0, 30.0, 200.0], [50.0, 30.0, 200.0]),
            flat_line([-50.0, -60.0, -200.0], [50.0, -60.0, -200.0]),
        ],
    );
    let img = render(&device, &queue, &mut viewport, &camera, "depth_cue");
    let x = SIZE[0] / 2;
    let mid = SIZE[1] / 2;
    let dark =
        |range: std::ops::Range<u32>| range.filter(|&y| luma(img.get_pixel(x, y)) < 150).count();
    let (near, far) = (dark(0..mid), dark(mid..SIZE[1]));
    eprintln!("depth cue: near {near} px, far {far} px");
    assert!(near >= 5 && far <= 2, "near {near} px, far {far} px");
}

#[test]
fn fog_fades_faces_towards_fog_color() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut style = face_style();
    style.sky = None;
    style.ground = None;
    let background = style.background;
    let (mut viewport, camera) = face_style_cube(&device, style.clone(), RED, RED);
    let img = render(&device, &queue, &mut viewport, &camera, "fog_off");
    let clear = pixel_at(&img, &camera, FRONT_FACE);
    assert!(!close(&clear, background, 30), "no fog, got {clear:?}");

    // Fully fogged from the near side of the model: background color.
    style.fog = Some(mt_render::Fog {
        start: 0.0,
        end: 0.0,
        color: None,
    });
    viewport.set_style(&device, style.clone());
    let img = render(&device, &queue, &mut viewport, &camera, "fog_full");
    let px = pixel_at(&img, &camera, FRONT_FACE);
    assert!(close(&px, background, 2), "background, got {px:?}");

    // A given color, halfway through the fog at the cube's center depth.
    let green = [0.0, 1.0, 0.0];
    style.fog = Some(mt_render::Fog {
        start: 0.0,
        end: 1.0,
        color: Some(green),
    });
    viewport.set_style(&device, style);
    let img = render(&device, &queue, &mut viewport, &camera, "fog_green");
    let px = pixel_at(&img, &camera, FRONT_FACE);
    assert!(
        px[1] > clear[1] + 40 && px[0] < clear[0],
        "towards green: {px:?} from {clear:?}"
    );
}

#[test]
fn render_image_keeps_screen_targets_and_can_be_transparent() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let spec = Spec::builtin().unwrap().default_model;
    let camera = Camera::from_spec(&spec.camera);
    let mut viewport = Viewport::new(&device, Style::from_spec(&spec));
    viewport.render(&device, &queue, &camera, SIZE, 1.0);
    let img = viewport.render_image(&device, &queue, &camera, [300, 200], 1.0, false);
    assert_eq!(img.dimensions(), (300, 200));
    assert_eq!(img.get_pixel(2, 2)[3], 255);
    let clear = viewport.render_image(&device, &queue, &camera, [300, 200], 1.0, true);
    assert_eq!(clear.get_pixel(2, 2)[3], 0, "background left out");
    // The axes are still drawn.
    let viewport_px = DVec2::new(300.0, 200.0);
    let on_z = camera
        .project(DVec3::new(0.0, 0.0, 100.0), viewport_px)
        .unwrap();
    let px = clear.get_pixel(on_z.x.round() as u32, on_z.y.round() as u32);
    assert!(px[3] > 100 && px[2] > 150, "blue Z axis, got {px:?}");
    // The on-screen frame is untouched.
    let view = viewport.color_view().unwrap();
    assert_eq!(view.texture().width(), SIZE[0]);
}

// ---------------------------------------------------------------- Styles extras

/// Plain white background, no axes or ground.
fn plain_style() -> Style {
    Style {
        sky: None,
        background: [1.0, 1.0, 1.0],
        ..face_style()
    }
}

/// Horizontal quad from (x0, y0) to (x1, y1) at height z, facing up.
fn flat_quad(x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> (DVec3, [DVec3; 4]) {
    let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    (DVec3::Z, c.map(|(x, y)| DVec3::new(x, y, z)))
}

/// Pixels darker than `max_luma` in the rectangle (inclusive).
fn dark_in(img: &image::RgbaImage, x: [u32; 2], y: [u32; 2], max_luma: i32) -> usize {
    let mut n = 0;
    for yy in y[0]..=y[1] {
        for xx in x[0]..=x[1] {
            if luma(img.get_pixel(xx, yy)) < max_luma {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn endpoints_widen_line_ends_and_jitter_adds_a_stroke() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let camera = top_down();
    let mut style = plain_style();
    let mut viewport = Viewport::new(&device, style.clone());
    viewport.set_edges(&device, &[flat_line([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0])]);
    let a = camera
        .project(DVec3::new(-50.0, 0.0, 0.0), viewport_px())
        .unwrap();
    let b = camera
        .project(DVec3::new(50.0, 0.0, 0.0), viewport_px())
        .unwrap();
    let y = a.y.round() as u32;
    let rows = [y - 6, y + 6];
    let near_end =
        |img: &image::RgbaImage| dark_in(img, [b.x as u32 - 4, b.x as u32 - 2], rows, 200);
    let middle = |img: &image::RgbaImage| {
        let m = ((a.x + b.x) / 2.0) as u32;
        dark_in(img, [m - 1, m + 1], rows, 200)
    };
    let plain = render(&device, &queue, &mut viewport, &camera, "endpoints_off");
    assert_eq!(near_end(&plain), middle(&plain), "uniform line");
    style.endpoints = Some(8.0);
    viewport.set_style(&device, style.clone());
    let ends = render(&device, &queue, &mut viewport, &camera, "endpoints_on");
    assert!(
        near_end(&ends) > middle(&ends),
        "ends wider: {} vs {}",
        near_end(&ends),
        middle(&ends)
    );
    assert_eq!(middle(&ends), middle(&plain), "the middle stays");

    style.endpoints = None;
    style.jitter = true;
    viewport.set_style(&device, style);
    let jitter = render(&device, &queue, &mut viewport, &camera, "jitter_on");
    // Ink: how much darker than white the line's surroundings are.
    let ink = |img: &image::RgbaImage| {
        let mut sum = 0;
        for yy in y - 8..=y + 8 {
            for xx in a.x as u32 - 8..=b.x as u32 + 8 {
                sum += 255 - luma(img.get_pixel(xx, yy));
            }
        }
        sum
    };
    // Two offset strokes put down more ink than one.
    assert!(
        ink(&jitter) * 10 > ink(&plain) * 12,
        "{} vs {}",
        ink(&jitter),
        ink(&plain)
    );
}

#[test]
fn ground_hides_what_is_below_it_unless_transparent() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let spec = Spec::builtin().unwrap().default_model;
    let mut style = Style {
        axes: None,
        ..Style::from_spec(&spec)
    };
    let camera = Camera {
        eye: DVec3::new(0.0, -100.0, 500.0),
        target: DVec3::ZERO,
        fov_deg: 35.0,
        perspective: true,
        two_point: false,
    };
    let mut viewport = Viewport::new(&device, style.clone());
    // Red below the ground, green on it.
    let (mut vertices, mut indices) =
        quads_mesh(&[flat_quad(-100.0, -40.0, -20.0, 40.0, -10.0)], RED, RED);
    let green = [0.2, 0.8, 0.2, 1.0];
    let (gv, gi) = quads_mesh(&[flat_quad(20.0, -40.0, 100.0, 40.0, 0.0)], green, green);
    let base = vertices.len() as u32;
    vertices.extend(gv);
    indices.extend(gi.iter().map(|i| i + base));
    viewport.set_faces(&device, &vertices, &indices);
    let img = render(&device, &queue, &mut viewport, &camera, "ground_opaque");
    let below = pixel_at(&img, &camera, DVec3::new(-60.0, 0.0, -10.0));
    let on = pixel_at(&img, &camera, DVec3::new(60.0, 0.0, 0.0));
    assert!(
        close(&below, spec.ground.color, 6),
        "ground hides red: {below:?}"
    );
    assert!(on[1] > on[0] + 60, "a face on the ground shows: {on:?}");
    style.ground_opacity = 0.5;
    viewport.set_style(&device, style);
    let img = render(&device, &queue, &mut viewport, &camera, "ground_half");
    let below = pixel_at(&img, &camera, DVec3::new(-60.0, 0.0, -10.0));
    assert!(
        below[0] > below[1] + 40,
        "red shows through the translucent ground: {below:?}"
    );
}

#[test]
fn transparency_off_draws_translucent_faces_opaque() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut style = plain_style();
    let mut viewport = Viewport::new(&device, style.clone());
    let blue = [0.1, 0.1, 0.9, 0.3];
    let (v, i) = quads_mesh(&[flat_quad(-50.0, -50.0, 50.0, 50.0, 0.0)], blue, blue);
    viewport.set_faces(&device, &v, &i);
    let camera = top_down();
    let img = render(&device, &queue, &mut viewport, &camera, "transparency_on");
    let on = pixel_at(&img, &camera, DVec3::ZERO);
    style.transparency = mt_render::Transparency::Off;
    viewport.set_style(&device, style.clone());
    let img = render(&device, &queue, &mut viewport, &camera, "transparency_off");
    let off = pixel_at(&img, &camera, DVec3::ZERO);
    assert!(on[0] > 150, "mostly white through 30% blue: {on:?}");
    assert!(off[0] < 60 && off[2] > 150, "opaque blue: {off:?}");
    // Faster draws them unsorted, still blended.
    style.transparency = mt_render::Transparency::Faster;
    viewport.set_style(&device, style);
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        "transparency_faster",
    );
    assert_eq!(pixel_at(&img, &camera, DVec3::ZERO), on);
}

#[test]
fn ambient_occlusion_darkens_inside_corners() {
    ambient_occlusion_corner(4);
}

/// The occlusion pass reads a plain (not multisampled) depth texture and draws into
/// the result directly.
#[test]
fn ambient_occlusion_without_multisampling() {
    ambient_occlusion_corner(1);
}

fn ambient_occlusion_corner(samples: u32) {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut style = plain_style();
    let mut viewport = Viewport::with_samples(&device, style.clone(), samples);
    let gray = [0.8, 0.8, 0.8, 1.0];
    // A floor and a wall meeting along x = 0.
    let wall = (
        DVec3::NEG_X,
        [
            DVec3::new(0.0, -100.0, 0.0),
            DVec3::new(0.0, -100.0, 100.0),
            DVec3::new(0.0, 100.0, 100.0),
            DVec3::new(0.0, 100.0, 0.0),
        ],
    );
    let (v, i) = quads_mesh(
        &[flat_quad(-200.0, -100.0, 0.0, 100.0, 0.0), wall],
        gray,
        gray,
    );
    viewport.set_faces(&device, &v, &i);
    let camera = Camera {
        eye: DVec3::new(-300.0, -60.0, 250.0),
        target: DVec3::new(-40.0, 0.0, 0.0),
        fov_deg: 35.0,
        perspective: true,
        two_point: false,
    };
    let corner = DVec3::new(-2.0, 0.0, 0.0);
    let open = DVec3::new(-120.0, 0.0, 0.0);
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        &format!("ao_off_{samples}x"),
    );
    let (c0, o0) = (
        pixel_at(&img, &camera, corner),
        pixel_at(&img, &camera, open),
    );
    style.ambient_occlusion = Some(mt_render::AmbientOcclusion {
        intensity: 1.0,
        distance: 20.0,
        color: [0.0; 3],
    });
    viewport.set_style(&device, style);
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        &format!("ao_on_{samples}x"),
    );
    let (c1, o1) = (
        pixel_at(&img, &camera, corner),
        pixel_at(&img, &camera, open),
    );
    eprintln!("corner {c0:?} -> {c1:?}, open floor {o0:?} -> {o1:?}");
    assert!(luma(&c1) + 15 < luma(&c0), "the corner darkens");
    assert!((luma(&o1) - luma(&o0)).abs() <= 4, "open floor stays");
}

fn red_png() -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([230, 20, 20, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[test]
fn watermarks_draw_under_and_over_the_model() {
    watermarks_under_and_over(4);
}

#[test]
fn watermarks_without_multisampling() {
    watermarks_under_and_over(1);
}

fn watermarks_under_and_over(samples: u32) {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let image: std::sync::Arc<[u8]> = red_png().into();
    let mark = |overlay: bool, opacity: f32| mt_render::Watermark {
        image: image.clone(),
        overlay,
        opacity,
        placement: mt_render::WatermarkPlacement::Stretched { lock_aspect: false },
    };
    let mut style = plain_style();
    style.watermarks = vec![mark(false, 1.0)];
    let mut viewport = Viewport::with_samples(&device, style.clone(), samples);
    let gray = [0.6, 0.6, 0.6, 1.0];
    let (v, i) = quads_mesh(&[flat_quad(-50.0, -50.0, 50.0, 50.0, 0.0)], gray, gray);
    viewport.set_faces(&device, &v, &i);
    let camera = top_down();
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        &format!("watermark_under_{samples}x"),
    );
    let face = pixel_at(&img, &camera, DVec3::ZERO);
    let outside = *img.get_pixel(5, 5);
    assert!(
        close(&outside, [230.0 / 255.0, 20.0 / 255.0, 20.0 / 255.0], 3),
        "{outside:?}"
    );
    assert!(
        face[0].abs_diff(face[1]) < 5,
        "the face covers it: {face:?}"
    );
    style.watermarks = vec![mark(true, 0.5)];
    viewport.set_style(&device, style);
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera,
        &format!("watermark_over_{samples}x"),
    );
    let face = pixel_at(&img, &camera, DVec3::ZERO);
    assert!(face[0] > face[1] + 50, "red half over the face: {face:?}");
    assert!(img.get_pixel(5, 5)[1] > 100, "half over white");
}

#[test]
fn selection_dots_use_the_selection_color() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let mut style = plain_style();
    style.selection_color = [0.0, 1.0, 0.0];
    let mut viewport = Viewport::new(&device, style);
    let white = [1.0, 1.0, 1.0, 1.0];
    let (v, i) = quads_mesh(&[flat_quad(-50.0, -50.0, 50.0, 50.0, 0.0)], white, white);
    viewport.set_faces(&device, &v, &i);
    viewport.set_selected_faces(&device, &v, &i);
    let img = render(
        &device,
        &queue,
        &mut viewport,
        &camera_for_dots(),
        "selection_green",
    );
    let green = img
        .pixels()
        .filter(|p| p[1] > 200 && p[0] < 60 && p[2] < 60)
        .count();
    let blue = img
        .pixels()
        .filter(|p| p[2] > 200 && p[0] < 60 && p[1] < 60)
        .count();
    assert!(green > 100 && blue == 0, "green {green}, blue {blue}");
}

fn camera_for_dots() -> Camera {
    top_down()
}

#[test]
fn environment_skydome_replaces_the_sky() {
    let Some((device, queue)) = device() else {
        eprintln!("SKIPPED: no wgpu adapter");
        return;
    };
    let spec = Spec::builtin().unwrap().default_model;
    let camera = Camera::from_spec(&spec.camera);
    let mut viewport = Viewport::new(&device, Style::from_spec(&spec));
    let before = read_back(
        &device,
        &queue,
        &viewport.render(&device, &queue, &camera, SIZE, 1.0).clone(),
    );
    // A flat red panorama: every background pixel turns red.
    let env = mt_render::EnvImage {
        width: 8,
        height: 4,
        pixels: vec![[1.0, 0.0, 0.0]; 32],
    };
    viewport.set_environment(Some((
        std::sync::Arc::new(env),
        mt_render::EnvView::default(),
    )));
    let after = read_back(
        &device,
        &queue,
        &viewport.render(&device, &queue, &camera, SIZE, 1.0).clone(),
    );
    let top = after.get_pixel(5, 5);
    assert!(close(top, [1.0, 0.0, 0.0], 2), "{top:?}");
    assert_ne!(before.get_pixel(5, 5), top);
    // Exposure 0.5 darkens it (sRGB of 0.5 linear is about 188).
    viewport.set_environment(Some((
        std::sync::Arc::new(mt_render::EnvImage {
            width: 8,
            height: 4,
            pixels: vec![[1.0, 0.0, 0.0]; 32],
        }),
        mt_render::EnvView {
            skydome_exposure: 0.5,
            ..Default::default()
        },
    )));
    let dim = read_back(
        &device,
        &queue,
        &viewport.render(&device, &queue, &camera, SIZE, 1.0).clone(),
    );
    assert!(dim.get_pixel(5, 5)[0].abs_diff(188) <= 3, "{:?}", dim.get_pixel(5, 5));
    viewport.set_environment(None);
    let back = read_back(
        &device,
        &queue,
        &viewport.render(&device, &queue, &camera, SIZE, 1.0).clone(),
    );
    assert_eq!(back.get_pixel(5, 5), before.get_pixel(5, 5));
}
