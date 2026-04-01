struct FrameUniforms {
    tev_color_reg0: vec4<f32>,
    tev_color_reg1: vec4<f32>,
    tev_color_reg2: vec4<f32>,
    tev_color_reg3: vec4<f32>,
    tev_konst_colors: array<vec4<f32>, 16>,
    tev_color_env: array<vec4<u32>, 4>,
    tev_alpha_env: array<vec4<u32>, 4>,
    tev_orders: array<vec4<u32>, 4>,
    num_tev_stages: u32,
    alpha_ref0: f32,
    alpha_ref1: f32,
    alpha_comp0: u32,
    alpha_comp1: u32,
    alpha_op: u32,
};

struct DrawUniforms {
    mvp: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> frame: FrameUniforms;

@group(0) @binding(1)
var<uniform> draw: DrawUniforms;

// 8 texture maps (texmap 0-7)
@group(0) @binding(2) var tex0: texture_2d<f32>;
@group(0) @binding(3) var tex1: texture_2d<f32>;
@group(0) @binding(4) var tex2: texture_2d<f32>;
@group(0) @binding(5) var tex3: texture_2d<f32>;
@group(0) @binding(6) var tex4: texture_2d<f32>;
@group(0) @binding(7) var tex5: texture_2d<f32>;
@group(0) @binding(8) var tex6: texture_2d<f32>;
@group(0) @binding(9) var tex7: texture_2d<f32>;

// 8 samplers (paired with texmaps)
@group(0) @binding(10) var samp0: sampler;
@group(0) @binding(11) var samp1: sampler;
@group(0) @binding(12) var samp2: sampler;
@group(0) @binding(13) var samp3: sampler;
@group(0) @binding(14) var samp4: sampler;
@group(0) @binding(15) var samp5: sampler;
@group(0) @binding(16) var samp6: sampler;
@group(0) @binding(17) var samp7: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) tex0: vec2<f32>,
    @location(3) tex1: vec2<f32>,
    @location(4) tex2: vec2<f32>,
    @location(5) tex3: vec2<f32>,
    @location(6) tex4: vec2<f32>,
    @location(7) tex5: vec2<f32>,
    @location(8) tex6: vec2<f32>,
    @location(9) tex7: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) uv1: vec2<f32>,
    @location(3) uv2: vec2<f32>,
    @location(4) uv3: vec2<f32>,
    @location(5) uv4: vec2<f32>,
    @location(6) uv5: vec2<f32>,
    @location(7) uv6: vec2<f32>,
    @location(8) uv7: vec2<f32>,
};
