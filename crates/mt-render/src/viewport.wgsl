// Viewport shaders: background (sky + ground) and screen-space lines.
// Colors are sRGB values written straight to an Rgba8Unorm target.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xy: viewport size in physical pixels, z: physical pixels per logical point,
    // w: clip-space w of the near plane.
    viewport: vec4<f32>,
    sky_zenith: vec4<f32>,
    // w: 1 when the sky is drawn.
    sky_horizon: vec4<f32>,
    // w: 1 when the ground is drawn.
    ground: vec4<f32>,
    // w: 1 when the ground is visible from below.
    background: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

// ---------------------------------------------------------------- background

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn bg_vs(@builtin(vertex_index) i: u32) -> BgOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    let ndc = uv * 2.0 - 1.0;
    var out: BgOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

fn unproject(ndc: vec2<f32>, depth: f32) -> vec3<f32> {
    let p = g.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    return p.xyz / p.w;
}

@fragment
fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
    let origin = unproject(in.ndc, 1.0);
    let dir = normalize(unproject(in.ndc, 0.5) - origin);

    let ground_on = g.ground.w > 0.5;
    let from_below = g.background.w > 0.5;
    let hits_ground = (origin.z > 0.0 && dir.z < 0.0) || (from_below && origin.z < 0.0 && dir.z > 0.0);
    if (ground_on && hits_ground) {
        return vec4<f32>(g.ground.rgb, 1.0);
    }
    if (g.sky_horizon.w > 0.5) {
        let t = pow(clamp(dir.z, 0.0, 1.0), 0.5);
        return vec4<f32>(mix(g.sky_horizon.rgb, g.sky_zenith.rgb, t), 1.0);
    }
    return vec4<f32>(g.background.rgb, 1.0);
}

// ---------------------------------------------------------------- lines

struct Segment {
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) color: vec4<f32>,
    // x: width in logical points, y: dash length in logical points (0 = solid).
    @location(3) style: vec2<f32>,
};

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Segment start in framebuffer pixels (y down) and unit direction.
    @location(1) @interpolate(flat) start_dir: vec4<f32>,
    // x: width in pixels, y: dash length in pixels.
    @location(2) @interpolate(flat) style: vec2<f32>,
};

// Pulls a clip-space point onto the near plane along the segment.
fn clip_near(p: vec4<f32>, q: vec4<f32>) -> vec4<f32> {
    let min_w = g.viewport.w;
    if (p.w >= min_w) {
        return p;
    }
    let t = (min_w - p.w) / (q.w - p.w);
    return mix(p, q, t);
}

@vertex
fn line_vs(@builtin(vertex_index) vi: u32, s: Segment) -> LineOut {
    var out: LineOut;
    var ca = g.view_proj * vec4<f32>(s.a, 1.0);
    var cb = g.view_proj * vec4<f32>(s.b, 1.0);
    if (ca.w < g.viewport.w && cb.w < g.viewport.w) {
        out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    // Clip `b` against the unclipped `a`.
    let ca0 = ca;
    ca = clip_near(ca, cb);
    cb = clip_near(cb, ca0);

    let size = g.viewport.xy;
    let ppp = g.viewport.z;
    let sa = (ca.xy / ca.w * 0.5 + 0.5) * size;
    let sb = (cb.xy / cb.w * 0.5 + 0.5) * size;
    var along = sb - sa;
    let len = length(along);
    var dir = vec2<f32>(1.0, 0.0);
    if (len > 1e-6) {
        dir = along / len;
    }
    let normal = vec2<f32>(-dir.y, dir.x);

    // Quad corners: 0..5 as two triangles over (end, side).
    let ends = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    let sides = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let end = ends[vi];
    let side = sides[vi];

    // Half width plus one pixel of feather for antialiasing.
    let half = s.style.x * ppp * 0.5 + 1.0;
    let c = select(ca, cb, end > 0.5);
    let base = select(sa, sb, end > 0.5);
    let extend = select(-half, half, end > 0.5);
    let screen = base + normal * side * half + dir * extend;
    let ndc = screen / size * 2.0 - 1.0;
    out.pos = vec4<f32>(ndc * c.w, c.z, c.w);
    out.color = s.color;
    // Framebuffer y runs down; everything above uses y up.
    out.start_dir = vec4<f32>(sa.x, size.y - sa.y, dir.x, -dir.y);
    out.style = vec2<f32>(s.style.x * ppp, s.style.y * ppp);
    return out;
}

@fragment
fn line_fs(in: LineOut) -> @location(0) vec4<f32> {
    let width = in.style.x;
    let dash = in.style.y;
    let rel = in.pos.xy - in.start_dir.xy;
    let dir = in.start_dir.zw;
    let along = dot(rel, dir);
    let across = dot(rel, vec2<f32>(-dir.y, dir.x));
    if (dash > 0.0 && fract(along / (2.0 * dash)) > 0.5) {
        discard;
    }
    let coverage = clamp(width * 0.5 + 0.5 - abs(across), 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
