struct Uniforms {
    src_rect: vec4<f32>,
    dst_size: vec2<f32>,
    gamma: f32,
    filter_mode: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var efb_color: texture_2d<f32>;
@group(0) @binding(2) var efb_color_sampler: sampler;

fn fetch(pos: vec2<f32>) -> vec4<f32> {
    let dst_size = max(u.dst_size, vec2<f32>(1.0, 1.0));

    if (u.filter_mode == 1u) {
        let src_step = u.src_rect.zw / dst_size;
        let first = u.src_rect.xy + (pos - vec2<f32>(0.5)) * src_step;
        let coord = vec2<i32>(floor(first));
        return (
            textureLoad(efb_color, coord, 0) +
            textureLoad(efb_color, coord + vec2<i32>(1, 0), 0) +
            textureLoad(efb_color, coord + vec2<i32>(0, 1), 0) +
            textureLoad(efb_color, coord + vec2<i32>(1, 1), 0)
        ) * 0.25;
    }

    let src_pixel = u.src_rect.xy + (pos / dst_size) * u.src_rect.zw;
    let coord = vec2<i32>(src_pixel);
    return textureLoad(efb_color, coord, 0);
}
