struct Uniforms {
    src_rect: vec4<f32>,
    dst_size: vec2<f32>,
    gamma: f32,
    filter_mode: u32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@group(0) @binding(1)
var efb_depth: texture_depth_multisampled_2d;

struct VsOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) f32 {
    let dst_size = max(u.dst_size, vec2<f32>(1.0, 1.0));
    let src_pixel = u.src_rect.xy + (position.xy / dst_size) * u.src_rect.zw;
    let coord = vec2<i32>(src_pixel);
    return textureLoad(efb_depth, coord, 0);
}
