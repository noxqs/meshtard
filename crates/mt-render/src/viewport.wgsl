// Viewport shaders: background (sky + ground), shaded faces and screen-space lines.
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
    // xyz: unit direction towards the light, w: ambient fraction.
    light: vec4<f32>,
    // x: perspective radians per pixel, y: parallel NDC depth per pixel (one is 0).
    line_depth: vec4<f32>,
    // xyz: camera eye (w = 1, perspective) or direction towards the camera (w = 0).
    eye: vec4<f32>,
    // x: profile width in logical points, y: 1 when profiles are drawn.
    profile: vec4<f32>,
    // World to shadow map clip space (orthographic, depth 0..1, regular Z).
    light_view_proj: mat4x4<f32>,
    // Section plane: points with dot(xyz, p) + w > 0 are clipped. (0, 0, 0, -1) when off.
    section: vec4<f32>,
    // xyz: unit direction towards the sun (zero below the horizon), w: 1 when shadow
    // lighting (sun + light/dark) replaces the style's shading.
    sun: vec4<f32>,
    // x: light, y: dark, z: 1 when faces receive shadows, w: 1 when the ground does.
    shadow: vec4<f32>,
    // x: world size of one shadow map texel, y: 1 / shadow map size.
    shadow_texel: vec4<f32>,
    // x: face coloring (0 material, 1 background color unshaded (hidden line),
    // 2 monochrome), y: opacity cap (< 1 in X-ray).
    face_style: vec4<f32>,
    mono_front: vec4<f32>,
    mono_back: vec4<f32>,
    // x: 1 when Edges is on, y: depth cue width in points (0 = off), z: extension in
    // points (0 = off), w: back edge dash in points.
    edge_style: vec4<f32>,
    // View depth of p: dot(xyz, p) + w.
    view_axis: vec4<f32>,
    // x, y: view depth of the model's near and far side; z, w: fog start and end depth.
    depth_range: vec4<f32>,
    // rgb: fog color, w: 1 to fade towards the background instead.
    fog_color: vec4<f32>,
    // x: endpoint length in points (0 = off), y: 1 for jitter, z: 1 when transparency
    // is off (faces opaque), w: ground opacity.
    extra: vec4<f32>,
    // x: sky gradient exponent (horizon blend).
    extra2: vec4<f32>,
    // Selected faces' dot color.
    select_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;
@group(2) @binding(0) var face_textures: texture_2d_array<f32>;
@group(2) @binding(1) var face_sampler: sampler;

// Environments (environment.rs): a linear RGB equirectangular panorama.
struct EnvParams {
    // x: 1 when drawn as the skydome, y: skydome exposure, z: rotation (radians),
    // w: 1 when used for reflections.
    a: vec4<f32>,
    // rgb: face tint from the average color, w: reflection exposure.
    b: vec4<f32>,
};
@group(3) @binding(0) var<uniform> env: EnvParams;
@group(3) @binding(1) var env_map: texture_2d<f32>;
@group(3) @binding(2) var env_sampler: sampler;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

// Panorama coordinates seen along `dir` (mirrors `dir_to_uv` in environment.rs).
fn env_uv(dir: vec3<f32>) -> vec2<f32> {
    let d = normalize(dir);
    let lon = atan2(d.y, d.x) - env.a.z;
    let u = fract(0.5 - lon / 6.2831853);
    let v = acos(clamp(d.z, -1.0, 1.0)) / 3.14159265;
    return vec2<f32>(u, v);
}

// The panorama's sRGB color along `dir` at `exposure`.
fn env_color(dir: vec3<f32>, exposure: f32) -> vec3<f32> {
    let c = textureSampleLevel(env_map, env_sampler, env_uv(dir), 0.0).rgb;
    return linear_to_srgb(c * exposure);
}

fn section_clipped(p: vec3<f32>) -> bool {
    return dot(g.section.xyz, p) + g.section.w > 0.0;
}

fn view_depth(p: vec3<f32>) -> f32 {
    return dot(g.view_axis.xyz, p) + g.view_axis.w;
}

// Fog amount at view depth `d` (mirrors `fog_amount` in viewport.rs). Off when the
// start is past the end (start 1, end 0).
fn fog_amount(d: f32) -> f32 {
    let start = g.depth_range.z;
    let end = g.depth_range.w;
    if (start > end) {
        return 0.0;
    }
    if (end <= start) {
        return select(0.0, 1.0, d >= end);
    }
    return clamp((d - start) / (end - start), 0.0, 1.0);
}

// `color` fogged for world point `p` seen at framebuffer position `frag` (y down).
fn apply_fog(color: vec3<f32>, p: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let f = fog_amount(view_depth(p));
    if (f <= 0.0) {
        return color;
    }
    var fog = g.fog_color.rgb;
    if (g.fog_color.w > 0.5) {
        let ndc = vec2<f32>(frag.x / g.viewport.x * 2.0 - 1.0, 1.0 - frag.y / g.viewport.y * 2.0);
        let origin = unproject(ndc, 1.0);
        fog = background_color(origin, normalize(unproject(ndc, 0.5) - origin));
    }
    return mix(color, fog, f);
}

// Fraction of the sun reaching `p` (1 = lit), 5x5 PCF over the shadow map. `n` is the
// visible side's normal; the sample point is pushed along it by a texel or two so a
// surface does not shadow itself.
fn sun_visibility(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let offset = p + n * (g.shadow_texel.x * 1.5);
    let c = g.light_view_proj * vec4<f32>(offset, 1.0);
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return 1.0;
    }
    // Receivers past the far plane (the ground under the model) are behind every caster.
    let depth = min(ndc.z, 1.0);
    var sum = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let o = vec2<f32>(f32(x), f32(y)) * g.shadow_texel.y;
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + o, depth);
        }
    }
    return sum / 25.0;
}

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

// Sky, ground or background color seen along the ray `origin` + t `dir` (no shadows).
fn background_color(origin: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    if (env.a.x > 0.5) {
        return env_color(dir, env.a.y);
    }
    let ground_on = g.ground.w > 0.5;
    let from_below = g.background.w > 0.5;
    let hits_ground = (origin.z > 0.0 && dir.z < 0.0) || (from_below && origin.z < 0.0 && dir.z > 0.0);
    var color = g.background.rgb;
    if (ground_on && hits_ground) {
        color = mix(color, g.ground.rgb, g.extra.w);
    } else {
        color = sky_color(dir);
    }
    return color;
}

// The sky (or background color without one) along `dir`, ignoring the ground.
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    if (g.sky_horizon.w > 0.5) {
        let t = pow(clamp(dir.z, 0.0, 1.0), max(g.extra2.x, 0.01));
        return mix(g.sky_horizon.rgb, g.sky_zenith.rgb, t);
    }
    return g.background.rgb;
}

// Whether the ray `origin` + t `dir` hits the drawn ground plane z = 0.
fn ray_hits_ground(origin: vec3<f32>, dir: vec3<f32>) -> bool {
    let from_below = g.background.w > 0.5;
    return g.ground.w > 0.5
        && ((origin.z > 0.0 && dir.z < 0.0) || (from_below && origin.z < 0.0 && dir.z > 0.0));
}

// Sky and background color. The ground is drawn by `ground_fs` after the opaque
// faces, so it can hide what is below it.
@fragment
fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
    let origin = unproject(in.ndc, 1.0);
    let dir = normalize(unproject(in.ndc, 0.5) - origin);
    // Environments > Use as Skydome: the panorama replaces sky, ground and background.
    if (env.a.x > 0.5) {
        return vec4<f32>(env_color(dir, env.a.y), 1.0);
    }
    if (ray_hits_ground(origin, dir)) {
        return vec4<f32>(g.background.rgb, 1.0);
    }
    return vec4<f32>(sky_color(dir), 1.0);
}

struct GroundOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// The ground plane z = 0 at its own depth (a hair farther, so faces lying on it win),
// blended with the ground opacity over the background and over geometry below it.
// Shadows on the ground, seen from above: unshadowed ground keeps its color; shadowed
// ground is scaled by dark / (dark + light * sun.z), the ratio the face shading would
// give a horizontal face.
@fragment
fn ground_fs(in: BgOut) -> GroundOut {
    let origin = unproject(in.ndc, 1.0);
    let dir = normalize(unproject(in.ndc, 0.5) - origin);
    // No ground plane under a skydome (the panorama shows its own ground).
    if (env.a.x > 0.5 || !ray_hits_ground(origin, dir)) {
        discard;
    }
    let p = origin + dir * (-origin.z / dir.z);
    var color = g.ground.rgb;
    if (g.shadow.w > 0.5 && origin.z > 0.0) {
        let lit = min(g.shadow.y + g.shadow.x * g.sun.z, 1.0);
        let dark = min(g.shadow.y, lit);
        let v = sun_visibility(p, vec3<f32>(0.0, 0.0, 1.0));
        color *= mix(dark / max(lit, 1e-4), 1.0, v);
    }
    let c = g.view_proj * vec4<f32>(p, 1.0);
    var out: GroundOut;
    out.color = vec4<f32>(apply_fog(color, p, in.pos.xy), g.extra.w);
    out.depth = clamp(c.z / c.w * (1.0 - 1e-5), 0.0, 1.0);
    return out;
}

// ---------------------------------------------------------------- faces

struct FaceVertex {
    @location(0) position: vec3<f32>,
    // Front normal; triangles wind counter-clockwise around it.
    @location(1) normal: vec3<f32>,
    @location(2) front: vec4<f32>,
    @location(3) back: vec4<f32>,
    // Front (xy) and back (zw) texture coordinates, in repeats, v up.
    @location(4) uv: vec4<f32>,
    // Front and back texture layer; 0xffffffff for none.
    @location(5) texture: vec2<u32>,
    // Front and back projective denominators: the coordinates are uv / q (1 when
    // affine), divided per pixel for distorted texture positions.
    @location(6) q: vec2<f32>,
};

struct FaceOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) front: vec4<f32>,
    @location(2) back: vec4<f32>,
    @location(3) world: vec3<f32>,
    @location(4) uv: vec4<f32>,
    @location(5) @interpolate(flat) texture: vec2<u32>,
    @location(6) q: vec2<f32>,
    // 1 when the triangle is drawn mirrored (an instance's transform mirrors it):
    // its front is the side facing away.
    @location(7) @interpolate(flat) mirror: u32,
};

@vertex
fn face_vs(v: FaceVertex) -> FaceOut {
    var out: FaceOut;
    out.pos = g.view_proj * vec4<f32>(v.position, 1.0);
    out.normal = v.normal;
    out.front = v.front;
    out.back = v.back;
    out.world = v.position;
    out.uv = v.uv;
    out.texture = v.texture;
    out.q = v.q;
    out.mirror = 0u;
    return out;
}

@fragment
fn face_fs(in: FaceOut, @builtin(front_facing) facing: bool) -> @location(0) vec4<f32> {
    let front = facing != (in.mirror != 0u);
    // Sampled before any branching (implicit derivatives need uniform control flow).
    // Image rows run top down, SketchUp's v up.
    let layer = select(in.texture.y, in.texture.x, front);
    let uv = select(in.uv.zw / in.q.y, in.uv.xy / in.q.x, front);
    let texel = textureSample(
        face_textures, face_sampler, vec2<f32>(uv.x, 1.0 - uv.y), i32(min(layer, 2147483647u))
    );
    if (section_clipped(in.world)) {
        discard;
    }
    var color = select(in.back, in.front, front);
    let textured = g.face_style.z > 0.5 && f32(layer) < g.face_style.w;
    if (textured) {
        // Cut-out textures (alpha) on opaque materials are alpha-tested; translucent
        // materials blend with the texture's alpha too.
        if (color.a >= 1.0 && texel.a < 0.5) {
            discard;
        }
        color = vec4<f32>(texel.rgb, select(color.a * texel.a, 1.0, color.a >= 1.0));
    }
    // Face Settings > Enable transparency off: materials draw opaque.
    if (g.extra.z > 0.5) {
        color.a = 1.0;
    }
    color.a = min(color.a, g.face_style.y);
    if (g.face_style.x > 1.5) {
        color = vec4<f32>(select(g.mono_back.rgb, g.mono_front.rgb, front), color.a);
    } else if (g.face_style.x > 0.5) {
        return vec4<f32>(apply_fog(g.background.rgb, in.world, in.pos.xy), color.a);
    }
    let n = normalize(select(-in.normal, in.normal, front));
    var shade: f32;
    if (g.sun.w > 0.5) {
        // Draft of SketchUp's shadow lighting: ambient "dark" plus "light" times the
        // sun's diffuse term, the latter only where the sun reaches.
        let diffuse = max(0.0, dot(n, g.sun.xyz));
        var v = 1.0;
        if (g.shadow.z > 0.5 && diffuse > 0.0) {
            v = sun_visibility(in.world, n);
        }
        shade = min(g.shadow.y + g.shadow.x * diffuse * v, 1.0);
    } else {
        let ambient = g.light.w;
        shade = ambient + (1.0 - ambient) * max(0.0, dot(n, g.light.xyz));
    }
    var rgb = color.rgb * shade;
    if (env.a.w > 0.5) {
        // Use for Reflections (draft): tint by the environment's average color and
        // mix in the reflected panorama at grazing angles (Schlick Fresnel, half
        // strength). Meshtard has no glossy materials, so every face reflects alike.
        let view = select(g.eye.xyz, g.eye.xyz - in.world, g.eye.w > 0.5);
        let v = normalize(view);
        let fresnel = 0.04 + 0.96 * pow(1.0 - clamp(abs(dot(n, v)), 0.0, 1.0), 5.0);
        let r = reflect(-v, n);
        rgb = mix(rgb * env.b.rgb, env_color(r, env.b.w), fresnel * 0.5);
    }
    return vec4<f32>(apply_fog(rgb, in.world, in.pos.xy), color.a);
}

// Section fill: the color as given, unlit and not clipped.
@fragment
fn fill_fs(in: FaceOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.front.rgb, 1.0);
}

// SketchUp marks selected faces with a grid of blue dots.
@fragment
fn select_fs(in: FaceOut) -> @location(0) vec4<f32> {
    if (section_clipped(in.world)) {
        discard;
    }
    let p = vec2<u32>(in.pos.xy);
    if (p.x % 3u != 0u || p.y % 3u != 0u) {
        discard;
    }
    return vec4<f32>(g.select_color.rgb, 1.0);
}

// ---------------------------------------------------------------- shadow map

// Depth from the sun. Faces less than 70% opaque cast no shadow (SketchUp's rule as
// commonly documented; unverified) and clipped-away geometry casts none either.
@vertex
fn shadow_vs(v: FaceVertex) -> FaceOut {
    var out: FaceOut;
    out.pos = g.light_view_proj * vec4<f32>(v.position, 1.0);
    out.normal = v.normal;
    out.front = v.front;
    out.back = v.back;
    out.world = v.position;
    out.uv = v.uv;
    out.texture = v.texture;
    out.q = v.q;
    out.mirror = 0u;
    return out;
}

@fragment
fn shadow_fs(in: FaceOut) {
    if (section_clipped(in.world) || max(in.front.a, in.back.a) < 0.7) {
        discard;
    }
}

// ---------------------------------------------------------------- lines

struct Segment {
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) color: vec4<f32>,
    // x: width in logical points, y: dash length in logical points (0 = solid).
    @location(3) style: vec2<f32>,
    // Front normals of the two faces using the edge.
    @location(4) n1: vec3<f32>,
    @location(5) n2: vec3<f32>,
    // 0: never a profile, 1: profile on the silhouette, 2: always a profile.
    @location(6) profile: f32,
    // 1: model edge (the Edge Style settings apply).
    @location(7) flags: u32,
};

// Whether the segment is drawn as a profile now (profiles on and it is one).
fn is_profile(s: Segment) -> bool {
    if (g.profile.y < 0.5 || s.profile < 0.5) {
        return false;
    }
    if (s.profile > 1.5) {
        return true;
    }
    let mid = (s.a + s.b) * 0.5;
    let to_eye = select(g.eye.xyz, g.eye.xyz - mid, g.eye.w > 0.5);
    return dot(s.n1, to_eye) * dot(s.n2, to_eye) <= 0.0;
}

// Depth cue: `max_width` at the model's near side to 1 at its far side (mirrors
// `depth_cue_width` in viewport.rs).
fn depth_cue_width(d: f32, max_width: f32) -> f32 {
    let t = clamp((d - g.depth_range.x) / max(g.depth_range.y - g.depth_range.x, 1e-6), 0.0, 1.0);
    return max_width + (1.0 - max_width) * t;
}

// Line width in logical points: widened to the profile width on profile edges; on
// model edges set by the depth cue (at the edge's midpoint) when that is on.
fn line_width(s: Segment, profile: bool) -> f32 {
    var w = s.style.x;
    if (s.flags == 1u && g.edge_style.y > 0.0) {
        w = depth_cue_width(view_depth((s.a + s.b) * 0.5), g.edge_style.y);
    }
    return select(w, max(w, g.profile.x), profile);
}

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Segment start in framebuffer pixels (y down) and unit direction.
    @location(1) @interpolate(flat) start_dir: vec4<f32>,
    // x: width in pixels, y: dash length in pixels, z: 1 for a model edge, w: segment
    // length in pixels.
    @location(2) @interpolate(flat) style: vec4<f32>,
    // World position along the segment, for the section plane.
    @location(3) world: vec3<f32>,
};

// How far along p -> q to move p to put it on the near plane (0 when in front of it).
fn clip_near_t(p: vec4<f32>, q: vec4<f32>) -> f32 {
    let min_w = g.viewport.w;
    if (p.w >= min_w) {
        return 0.0;
    }
    return (min_w - p.w) / (q.w - p.w);
}

// Line quads have the edge's depth across their whole width, but a face through the
// edge seen at a grazing angle gets nearer within that width, which would stipple the
// edge. So the quad is moved towards the camera by LINE_PULL times its half width, in
// pixels of lateral size at its depth: enough for faces down to ~1/LINE_PULL radians
// (about 18 degrees) from edge-on. Measured in pixels, the pull is the same near and
// far, so hidden edges stay hidden unless within a few pixels' worth of depth behind
// a face (only right next to the vertex where they come out from behind it).
const LINE_PULL: f32 = 3.0;

fn pull_depth(c: vec4<f32>, half_px: f32) -> f32 {
    let px = half_px * LINE_PULL;
    // Reverse-Z perspective: ndc z = near / distance; moving towards the camera by a
    // fraction f of the distance divides the distance by (1 - f).
    let perspective = c.z / max(1.0 - px * g.line_depth.x, 0.5);
    // Parallel: ndc z is linear in distance.
    let z = perspective + px * g.line_depth.y * c.w;
    // Never in front of the near plane, where it would be clipped.
    return min(z, c.w);
}

@vertex
fn line_vs(@builtin(vertex_index) vi: u32, s: Segment) -> LineOut {
    return line_vertex(vi, s, false);
}

// Back edges: plain width (no profile widening), like SketchUp's thin dashed lines.
@vertex
fn back_line_vs(@builtin(vertex_index) vi: u32, s: Segment) -> LineOut {
    return line_vertex(vi, s, true);
}

// A pseudo-random number in 0..1 from a segment and a seed (Jitter).
fn hash(s: Segment, seed: f32) -> f32 {
    let k = dot(s.a + s.b * 1.37, vec3<f32>(12.9898, 78.233, 37.719)) + seed * 19.19;
    return fract(sin(k) * 43758.5453);
}

// Vertices 0..5 draw the segment's quad; 6..11 a second, jittered stroke of model
// edges when Jitter is on (else nothing).
fn line_vertex(vi_all: u32, s: Segment, back: bool) -> LineOut {
    var out: LineOut;
    let stroke = vi_all / 6u;
    let vi = vi_all % 6u;
    var ca = g.view_proj * vec4<f32>(s.a, 1.0);
    var cb = g.view_proj * vec4<f32>(s.b, 1.0);
    let model = s.flags == 1u;
    let profile = is_profile(s);
    // Behind the camera, or Edges off and not a profile.
    let hidden = model && g.edge_style.x < 0.5 && !profile;
    let jitter = model && g.extra.y > 0.5 && !back;
    if ((ca.w < g.viewport.w && cb.w < g.viewport.w) || hidden || (stroke > 0u && !jitter)) {
        out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let ta = clip_near_t(ca, cb);
    let tb = clip_near_t(cb, ca);
    let ca0 = ca;
    ca = mix(ca0, cb, ta);
    cb = mix(cb, ca0, tb);
    let wa = mix(s.a, s.b, ta);
    let wb = mix(s.b, s.a, tb);

    let size = g.viewport.xy;
    let ppp = g.viewport.z;
    var sa = (ca.xy / ca.w * 0.5 + 0.5) * size;
    var sb = (cb.xy / cb.w * 0.5 + 0.5) * size;
    // Extension (mirrors `extend_segment` in viewport.rs).
    let ext = g.edge_style.z * ppp;
    if (model && ext > 0.0 && length(sb - sa) > 1e-6) {
        let d = normalize(sb - sa);
        sa -= d * ext;
        sb += d * ext;
    }
    // Jitter: each stroke's ends moved a little across and along the line.
    if (jitter) {
        let d0 = sb - sa;
        if (length(d0) > 1e-6) {
            let d = normalize(d0);
            let n = vec2<f32>(-d.y, d.x);
            let amount = 1.5 * ppp;
            let k = f32(stroke) * 4.0;
            sa += (n * (hash(s, k) - 0.5) * 2.0 + d * (hash(s, k + 1.0) - 0.5) * 2.0) * amount;
            sb += (n * (hash(s, k + 2.0) - 0.5) * 2.0 + d * (hash(s, k + 3.0) - 0.5) * 2.0) * amount;
        }
    }
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
    let width = line_width(s, profile && !back);
    // Endpoints: model edge ends are drawn ENDPOINT_EXTRA points wider.
    let ends_on = model && g.extra.x > 0.0 && !back;
    let half = (width + select(0.0, ENDPOINT_EXTRA, ends_on)) * ppp * 0.5 + 1.0;
    let c = select(ca, cb, end > 0.5);
    let base = select(sa, sb, end > 0.5);
    let extend = select(-half, half, end > 0.5);
    let screen = base + normal * side * half + dir * extend;
    let ndc = screen / size * 2.0 - 1.0;
    out.pos = vec4<f32>(ndc * c.w, pull_depth(c, half), c.w);
    out.color = s.color;
    out.world = select(wa, wb, end > 0.5);
    // Framebuffer y runs down; everything above uses y up.
    out.start_dir = vec4<f32>(sa.x, size.y - sa.y, dir.x, -dir.y);
    out.style = vec4<f32>(width * ppp, s.style.y * ppp, select(0.0, 1.0, model), len);
    return out;
}

// How much wider (points) Endpoints draws the ends of model edges (unverified).
const ENDPOINT_EXTRA: f32 = 2.0;

// Model edges: clipped by the section plane.
@fragment
fn line_fs(in: LineOut) -> @location(0) vec4<f32> {
    if (section_clipped(in.world)) {
        discard;
    }
    return fogged_line(in, line_color(in, in.style.y));
}

// Back edges: the hidden parts of model edges (this pipeline only passes fragments
// behind the depth buffer), dashed.
@fragment
fn back_line_fs(in: LineOut) -> @location(0) vec4<f32> {
    if (in.style.z < 0.5 || section_clipped(in.world)) {
        discard;
    }
    return fogged_line(in, line_color(in, g.edge_style.w * g.viewport.z));
}

// Axes and other overlays: never clipped.
@fragment
fn overlay_line_fs(in: LineOut) -> @location(0) vec4<f32> {
    return line_color(in, in.style.y);
}

// Model edges fade into the fog like faces.
fn fogged_line(in: LineOut, c: vec4<f32>) -> vec4<f32> {
    if (in.style.z < 0.5) {
        return c;
    }
    return vec4<f32>(apply_fog(c.rgb, in.world, in.pos.xy), c.a);
}

fn line_color(in: LineOut, dash: f32) -> vec4<f32> {
    var width = in.style.x;
    let rel = in.pos.xy - in.start_dir.xy;
    let dir = in.start_dir.zw;
    let along = dot(rel, dir);
    let ends = g.extra.x * g.viewport.z;
    if (in.style.z > 0.5 && ends > 0.0 && (along < ends || along > in.style.w - ends)) {
        width += ENDPOINT_EXTRA * g.viewport.z;
    }
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

// ---------------------------------------------------------------- instances

// A mesh drawn once per instance (components placed many times): the instance's
// transform, column by column.
struct Inst {
    @location(8) c0: vec4<f32>,
    @location(9) c1: vec4<f32>,
    @location(10) c2: vec4<f32>,
    @location(11) c3: vec4<f32>,
};

fn inst_matrix(i: Inst) -> mat4x4<f32> {
    return mat4x4<f32>(i.c0, i.c1, i.c2, i.c3);
}

fn inst_point(m: mat4x4<f32>, p: vec3<f32>) -> vec3<f32> {
    return (m * vec4<f32>(p, 1.0)).xyz;
}

fn inst_det(m: mat4x4<f32>) -> f32 {
    return dot(m[0].xyz, cross(m[1].xyz, m[2].xyz));
}

// A normal placed by `m`: the inverse transpose's direction, which is the cofactor
// matrix's times the sign of the determinant. Zero stays zero.
fn inst_normal(m: mat4x4<f32>, n: vec3<f32>) -> vec3<f32> {
    let a = m[0].xyz;
    let b = m[1].xyz;
    let c = m[2].xyz;
    let cof = mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b));
    let w = cof * n * select(1.0, -1.0, inst_det(m) < 0.0);
    let l = length(w);
    return select(vec3<f32>(0.0), w / l, l > 1e-30);
}

fn inst_face(v: FaceVertex, i: Inst) -> FaceOut {
    let m = inst_matrix(i);
    let p = inst_point(m, v.position);
    var out: FaceOut;
    out.pos = g.view_proj * vec4<f32>(p, 1.0);
    out.normal = inst_normal(m, v.normal);
    out.front = v.front;
    out.back = v.back;
    out.world = p;
    out.uv = v.uv;
    out.texture = v.texture;
    out.q = v.q;
    // A mirroring transform turns the triangles' winding around.
    out.mirror = select(0u, 1u, inst_det(m) < 0.0);
    return out;
}

@vertex
fn inst_face_vs(v: FaceVertex, i: Inst) -> FaceOut {
    return inst_face(v, i);
}

@vertex
fn inst_shadow_vs(v: FaceVertex, i: Inst) -> FaceOut {
    var out = inst_face(v, i);
    out.pos = g.light_view_proj * vec4<f32>(out.world, 1.0);
    return out;
}

fn inst_segment(s: Segment, i: Inst) -> Segment {
    let m = inst_matrix(i);
    var o = s;
    o.a = inst_point(m, s.a);
    o.b = inst_point(m, s.b);
    o.n1 = inst_normal(m, s.n1);
    o.n2 = inst_normal(m, s.n2);
    return o;
}

// Instanced edges: the transform repeats for the 6 vertices of each segment's quad
// (one stroke: no Jitter).
@vertex
fn inst_line_vs(@builtin(vertex_index) vi: u32, s: Segment, i: Inst) -> LineOut {
    return line_vertex(vi % 6u, inst_segment(s, i), false);
}

@vertex
fn inst_back_line_vs(@builtin(vertex_index) vi: u32, s: Segment, i: Inst) -> LineOut {
    return line_vertex(vi % 6u, inst_segment(s, i), true);
}
