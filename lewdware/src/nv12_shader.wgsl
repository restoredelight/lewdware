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

// NV12: plane 0 = Y (R8Unorm), plane 1 = UV interleaved (Rg8Unorm, .r = U, .g = V)
@group(0) @binding(0) var t_y:  texture_2d<f32>;
@group(0) @binding(1) var t_uv: texture_2d<f32>;
@group(0) @binding(2) var s:    sampler;

struct WindowOptions {
    opacity: f32,
}
@group(1) @binding(0) var<uniform> options: WindowOptions;

fn gamma_decode(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// BT.709 limited range
@fragment
fn fs_nv12_limited(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw  = textureSample(t_y,  s, in.uv).r;
    let uv     = textureSample(t_uv, s, in.uv);

    let y  = (y_raw  - 16.0  / 255.0) * (255.0 / 219.0);
    let cb = (uv.r   - 128.0 / 255.0) * (255.0 / 224.0);
    let cr = (uv.g   - 128.0 / 255.0) * (255.0 / 224.0);

    let r = y + 1.57480 * cr;
    let g = y - 0.18732 * cb - 0.46812 * cr;
    let b = y + 1.85560 * cb;

    return vec4<f32>(
        gamma_decode(clamp(r, 0.0, 1.0)),
        gamma_decode(clamp(g, 0.0, 1.0)),
        gamma_decode(clamp(b, 0.0, 1.0)),
        options.opacity,
    );
}

// BT.709 full range
@fragment
fn fs_nv12_full(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw  = textureSample(t_y,  s, in.uv).r;
    let uv     = textureSample(t_uv, s, in.uv);

    let y  = y_raw;
    let cb = uv.r - 0.5;
    let cr = uv.g - 0.5;

    let r = y + 1.57480 * cr;
    let g = y - 0.18732 * cb - 0.46812 * cr;
    let b = y + 1.85560 * cb;

    return vec4<f32>(
        gamma_decode(clamp(r, 0.0, 1.0)),
        gamma_decode(clamp(g, 0.0, 1.0)),
        gamma_decode(clamp(b, 0.0, 1.0)),
        options.opacity,
    );
}
