// Screen-space ambient occlusion (Styles > Face > Ambient Occlusion): darkens faces
// where other geometry is close in front of them, from the depth of the opaque faces.
// A simple hemisphere estimate over a dozen depth samples; SketchUp's own method is not
// known.

struct Ao {
    inv_view_proj: mat4x4<f32>,
    // xy: viewport size in physical pixels.
    viewport: vec4<f32>,
    // x: intensity 0..1, y: occluder distance in world units, z: largest sampling
    // radius in pixels.
    params: vec4<f32>,
    // rgb: occlusion color.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ao: Ao;
@group(0) @binding(1) var depth_tex: texture_depth_multisampled_2d;

@vertex
fn ao_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn ndc_of(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(p.x / ao.viewport.x * 2.0 - 1.0, 1.0 - p.y / ao.viewport.y * 2.0);
}

fn unproject(ndc: vec2<f32>, depth: f32) -> vec3<f32> {
    let p = ao.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    return p.xyz / p.w;
}

fn depth_at(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(ao.viewport.xy);
    let q = clamp(p, vec2<i32>(0), size - 1);
    return textureLoad(depth_tex, q, 0);
}

// World position of pixel `p` (center), w = 0 where nothing was drawn.
fn world_at(p: vec2<i32>) -> vec4<f32> {
    let d = depth_at(p);
    return vec4<f32>(unproject(ndc_of(vec2<f32>(p) + 0.5), d), select(0.0, 1.0, d > 0.0));
}

@fragment
fn ao_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let d0 = depth_at(p);
    // Reverse Z: 0 is the cleared far plane (no face here).
    if (d0 <= 0.0) {
        discard;
    }
    let p0 = unproject(ndc_of(pos.xy), d0);
    // Normal from the neighbors on the side with the smaller depth step (avoids
    // smearing across silhouettes).
    let l = world_at(p - vec2<i32>(1, 0)).xyz;
    let r = world_at(p + vec2<i32>(1, 0)).xyz;
    let u = world_at(p - vec2<i32>(0, 1)).xyz;
    let dn = world_at(p + vec2<i32>(0, 1)).xyz;
    let dx = select(p0 - l, r - p0, length(r - p0) < length(p0 - l));
    let dy = select(p0 - u, dn - p0, length(dn - p0) < length(p0 - u));
    var n = normalize(cross(dx, dy));
    let toward_eye = unproject(ndc_of(pos.xy), 1.0) - p0;
    if (dot(n, toward_eye) < 0.0) {
        n = -n;
    }
    // Sampling radius: the occluder distance in pixels at this depth.
    let px_world = length(unproject(ndc_of(pos.xy + vec2<f32>(1.0, 0.0)), d0) - p0);
    let dist = ao.params.y;
    let radius = clamp(dist / max(px_world, 1e-6), 2.0, ao.params.z);
    // Interleaved gradient noise turns the sample spiral per pixel.
    let noise = fract(52.9829189 * fract(dot(pos.xy, vec2<f32>(0.06711056, 0.00583715))));
    var occlusion = 0.0;
    let count = 12;
    for (var i = 0; i < count; i++) {
        let fi = f32(i);
        let angle = (fi + noise) * 2.39996323;
        let rr = radius * (fi + 0.5 + noise * 0.5) / f32(count);
        let q = p + vec2<i32>(round(vec2<f32>(cos(angle), sin(angle)) * rr));
        let s = world_at(q);
        if (s.w < 0.5) {
            continue;
        }
        let v = s.xyz - p0;
        let len = length(v);
        if (len < 1e-6) {
            continue;
        }
        let facing = max(dot(n, v / len) - 0.15, 0.0);
        let falloff = clamp(1.0 - len / dist, 0.0, 1.0);
        occlusion += facing * falloff;
    }
    let amount = clamp(occlusion / f32(count) * 2.0 * ao.params.x, 0.0, 1.0);
    return vec4<f32>(ao.color.rgb, amount);
}
