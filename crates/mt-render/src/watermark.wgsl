// Watermarks (Styles > Watermark): images drawn under or over the model.

@group(0) @binding(0) var wm_texture: texture_2d<f32>;
@group(0) @binding(1) var wm_sampler: sampler;

struct WmVertex {
    // Clip-space xy.
    @location(0) pos: vec2<f32>,
    // Texture coordinates, (0, 0) the image's top left; past 1 repeats (tiled).
    @location(1) uv: vec2<f32>,
    @location(2) opacity: f32,
};

struct WmOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
};

@vertex
fn wm_vs(v: WmVertex) -> WmOut {
    var out: WmOut;
    out.pos = vec4<f32>(v.pos, 0.0, 1.0);
    out.uv = v.uv;
    out.opacity = v.opacity;
    return out;
}

@fragment
fn wm_fs(in: WmOut) -> @location(0) vec4<f32> {
    let c = textureSample(wm_texture, wm_sampler, in.uv);
    return vec4<f32>(c.rgb, c.a * in.opacity);
}
