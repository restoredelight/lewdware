struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
    );
    var uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    out.position = vec4<f32>(pos[in_vertex_index], 0.0, 1.0);
    out.uv = uvs[in_vertex_index];
    return out;
}

@group(0) @binding(0) var t_y:  texture_2d<f32>;
@group(0) @binding(1) var t_cb: texture_2d<f32>;
@group(0) @binding(2) var t_cr: texture_2d<f32>;
@group(0) @binding(3) var s_yuv: sampler;

struct WindowOptions {
    opacity: f32,
    // Non-zero when the surface is CompositeAlphaMode::PreMultiplied, in which case rgb must
    // be pre-scaled by alpha. Otherwise (PostMultiplied, or Opaque where alpha is ignored by
    // the compositor entirely) rgb is emitted straight.
    premultiply: u32,
}
@group(1) @binding(0) var<uniform> options: WindowOptions;

// Undo sRGB/BT.709 gamma so the sRGB surface can re-apply it correctly.
fn gamma_decode(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn premultiply(rgb: vec3<f32>, alpha: f32) -> vec4<f32> {
    if options.premultiply != 0u {
        return vec4<f32>(rgb * alpha, alpha);
    }
    return vec4<f32>(rgb, alpha);
}

// BT.709 limited range: Y in [16/255, 235/255], Cb/Cr in [16/255, 240/255]
@fragment
fn fs_yuv_limited(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw  = textureSample(t_y,  s_yuv, in.uv).r;
    let cb_raw = textureSample(t_cb, s_yuv, in.uv).r;
    let cr_raw = textureSample(t_cr, s_yuv, in.uv).r;

    let y  = (y_raw  - 16.0  / 255.0) * (255.0 / 219.0);
    let cb = (cb_raw - 128.0 / 255.0) * (255.0 / 224.0);
    let cr = (cr_raw - 128.0 / 255.0) * (255.0 / 224.0);

    let r = y + 1.57480 * cr;
    let g = y - 0.18732 * cb - 0.46812 * cr;
    let b = y + 1.85560 * cb;

    let alpha = options.opacity;
    let rgb = vec3<f32>(
        gamma_decode(clamp(r, 0.0, 1.0)),
        gamma_decode(clamp(g, 0.0, 1.0)),
        gamma_decode(clamp(b, 0.0, 1.0)),
    );
    return premultiply(rgb, alpha);
}

// BT.709 full range: Y in [0, 1], Cb/Cr in [0, 1] centred at 0.5
@fragment
fn fs_yuv_full(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw  = textureSample(t_y,  s_yuv, in.uv).r;
    let cb_raw = textureSample(t_cb, s_yuv, in.uv).r;
    let cr_raw = textureSample(t_cr, s_yuv, in.uv).r;

    let y  = y_raw;
    let cb = cb_raw - 0.5;
    let cr = cr_raw - 0.5;

    let r = y + 1.57480 * cr;
    let g = y - 0.18732 * cb - 0.46812 * cr;
    let b = y + 1.85560 * cb;

    let alpha = options.opacity;
    let rgb = vec3<f32>(
        gamma_decode(clamp(r, 0.0, 1.0)),
        gamma_decode(clamp(g, 0.0, 1.0)),
        gamma_decode(clamp(b, 0.0, 1.0)),
    );
    return premultiply(rgb, alpha);
}

// Packed-alpha YUV420p: top half = color, bottom half = alpha-as-luma.
// Always encoded full-range; Cb/Cr sampled from top half only.
@fragment
fn fs_yuv_packed_alpha(in: VertexOutput) -> @location(0) vec4<f32> {
    let color_uv = vec2<f32>(in.uv.x, in.uv.y * 0.5);
    let alpha_uv = vec2<f32>(in.uv.x, in.uv.y * 0.5 + 0.5);

    let y_raw     = textureSample(t_y,  s_yuv, color_uv).r;
    let cb_raw    = textureSample(t_cb, s_yuv, color_uv).r;
    let cr_raw    = textureSample(t_cr, s_yuv, color_uv).r;
    let alpha_raw = textureSample(t_y,  s_yuv, alpha_uv).r;

    let y  = y_raw;
    let cb = cb_raw - 0.5;
    let cr = cr_raw - 0.5;

    let r = y + 1.57480 * cr;
    let g = y - 0.18732 * cb - 0.46812 * cr;
    let b = y + 1.85560 * cb;

    let alpha = alpha_raw * options.opacity;
    let rgb = vec3<f32>(
        gamma_decode(clamp(r, 0.0, 1.0)),
        gamma_decode(clamp(g, 0.0, 1.0)),
        gamma_decode(clamp(b, 0.0, 1.0)),
    );
    return premultiply(rgb, alpha);
}
