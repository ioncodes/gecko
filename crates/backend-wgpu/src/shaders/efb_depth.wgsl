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

fn sample_depth(pos: vec2<f32>) -> f32 {
    let dst_size = max(u.dst_size, vec2<f32>(1.0, 1.0));
    let src_pixel = u.src_rect.xy + (pos / dst_size) * u.src_rect.zw;
    return textureLoad(efb_depth, vec2<i32>(src_pixel), 0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) f32 {
    return sample_depth(position.xy);
}

const Z24_SCALE: f32 = 16777216.0; // 2^24

fn fetch(pos: vec2<f32>) -> vec4<f32> {
    let z24 = min(u32(sample_depth(pos) * Z24_SCALE), 0xFFFFFFu);
    let r = f32((z24 >> 16u) & 0xFFu) / 255.0;
    let g = f32((z24 >>  8u) & 0xFFu) / 255.0;
    let b = f32( z24         & 0xFFu) / 255.0;
    return vec4<f32>(r, g, b, 1.0);
}
