struct Uniforms {
    src_rect: vec4<f32>,
    dst_size: vec2<f32>,
    gamma: f32,
    filter_mode: u32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@group(0) @binding(1)
var efb_tex: texture_2d<f32>;

@group(0) @binding(2)
var efb_sampler: sampler;

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
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(efb_tex));
    let dst_size = max(u.dst_size, vec2<f32>(1.0, 1.0));

    if (u.filter_mode == 2u) {
        let src_step = u.src_rect.zw / dst_size;
        let first = u.src_rect.xy + (position.xy - vec2<f32>(0.5)) * src_step;
        let coord = vec2<i32>(floor(first));
        let half_step = vec2<i32>(src_step * 0.5);

        return (
            textureLoad(efb_tex, coord, 0) +
            textureLoad(efb_tex, coord + vec2<i32>(half_step.x, 0), 0) +
            textureLoad(efb_tex, coord + vec2<i32>(0, half_step.y), 0) +
            textureLoad(efb_tex, coord + half_step, 0)
        ) * 0.25;
    }

    let src_pixel = u.src_rect.xy + (position.xy / dst_size) * u.src_rect.zw;
    let uv = src_pixel / tex_size;

    var color = textureSample(efb_tex, efb_sampler, uv);
    if (abs(u.gamma - 1.0) > 0.001) {
        color = vec4<f32>(pow(max(color.rgb, vec3<f32>(0.0, 0.0, 0.0)), vec3<f32>(u.gamma)), color.a);
    }
    
    return color;
}
