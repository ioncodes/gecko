struct Uniforms {
    src_rect: vec4<f32>,
    dst_size: vec2<f32>,
    gamma: f32,
    filter_mode: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var efb_color: texture_2d<f32>;
@group(0) @binding(2) var efb_color_sampler: sampler;

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

fn fetch(pos: vec2<f32>) -> vec4<f32> {
    let dst_size = max(u.dst_size, vec2<f32>(1.0, 1.0));
    let src_pixel = u.src_rect.xy + (pos / dst_size) * u.src_rect.zw;
    let coord = vec2<i32>(src_pixel);
    return textureLoad(efb_color, coord, 0);
}

fn luma_u8(s: vec4<f32>) -> u32 {
    let r = u32(round(s.r * 255.0));
    let g = u32(round(s.g * 255.0));
    let b = u32(round(s.b * 255.0));
    return min((299u * r + 587u * g + 114u * b) / 1000u, 255u);
}

fn expand3(v: u32) -> u32 { return (v << 5u) | (v << 2u) | (v >> 1u); }
fn expand4(v: u32) -> u32 { return (v << 4u) | v; }
fn expand5(v: u32) -> u32 { return (v << 3u) | (v >> 2u); }
fn expand6(v: u32) -> u32 { return (v << 2u) | (v >> 4u); }

@fragment
fn fs_rgba8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return fetch(p.xy);
}

@fragment
fn fs_rgba8_intensity(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = fetch(p.xy);
    let yf = f32(luma_u8(s)) / 255.0;
    return vec4<f32>(yf, yf, yf, s.a);
}

@fragment
fn fs_i8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let yf = f32(luma_u8(fetch(p.xy))) / 255.0;
    return vec4<f32>(yf, yf, yf, yf);
}

@fragment
fn fs_i4(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let q = (luma_u8(fetch(p.xy)) >> 4u) & 0xFu;
    let vf = f32(expand4(q)) / 255.0;
    return vec4<f32>(vf, vf, vf, vf);
}

@fragment
fn fs_ia8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = fetch(p.xy);
    let yf = f32(luma_u8(s)) / 255.0;
    return vec4<f32>(yf, yf, yf, s.a);
}

@fragment
fn fs_ia4(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = fetch(p.xy);
    let iq = (luma_u8(s) >> 4u) & 0xFu;
    let aq = (u32(round(s.a * 255.0)) >> 4u) & 0xFu;
    let ivf = f32(expand4(iq)) / 255.0;
    let avf = f32(expand4(aq)) / 255.0;
    return vec4<f32>(ivf, ivf, ivf, avf);
}

@fragment
fn fs_rgb565_intensity(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let y = luma_u8(fetch(p.xy));
    let r5 = (y >> 3u) & 0x1Fu;
    let g6 = (y >> 2u) & 0x3Fu;
    let b5 = (y >> 3u) & 0x1Fu;
    return vec4<f32>(f32(expand5(r5)) / 255.0, f32(expand6(g6)) / 255.0, f32(expand5(b5)) / 255.0, 1.0);
}

@fragment
fn fs_rgb565(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = fetch(p.xy);
    let r5 = (u32(round(s.r * 255.0)) >> 3u) & 0x1Fu;
    let g6 = (u32(round(s.g * 255.0)) >> 2u) & 0x3Fu;
    let b5 = (u32(round(s.b * 255.0)) >> 3u) & 0x1Fu;
    return vec4<f32>(f32(expand5(r5)) / 255.0, f32(expand6(g6)) / 255.0, f32(expand5(b5)) / 255.0, 1.0);
}

@fragment
fn fs_rgb5a3_intensity(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = fetch(p.xy);
    let y = luma_u8(s);
    let a = u32(round(s.a * 255.0));

    if (a == 255u) {
        let vf = f32(expand5((y >> 3u) & 0x1Fu)) / 255.0;
        return vec4<f32>(vf, vf, vf, 1.0);
    } else {
        let vf = f32(expand4((y >> 4u) & 0xFu)) / 255.0;
        let af = f32(expand3((a >> 5u) & 0x7u)) / 255.0;
        return vec4<f32>(vf, vf, vf, af);
    }
}

@fragment
fn fs_rgb5a3(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = fetch(p.xy);
    let r = u32(round(s.r * 255.0));
    let g = u32(round(s.g * 255.0));
    let b = u32(round(s.b * 255.0));
    let a = u32(round(s.a * 255.0));

    if (a == 255u) {
        let r8 = expand5((r >> 3u) & 0x1Fu);
        let g8 = expand5((g >> 3u) & 0x1Fu);
        let b8 = expand5((b >> 3u) & 0x1Fu);
        return vec4<f32>(f32(r8) / 255.0, f32(g8) / 255.0, f32(b8) / 255.0, 1.0);
    } else {
        let r8 = expand4((r >> 4u) & 0xFu);
        let g8 = expand4((g >> 4u) & 0xFu);
        let b8 = expand4((b >> 4u) & 0xFu);
        let a8 = expand3((a >> 5u) & 0x7u);
        return vec4<f32>(f32(r8) / 255.0, f32(g8) / 255.0, f32(b8) / 255.0, f32(a8) / 255.0);
    }
}

@fragment
fn fs_a8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let a = fetch(p.xy).a;
    return vec4<f32>(a, a, a, a);
}

@fragment
fn fs_r8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let r = fetch(p.xy).r;
    return vec4<f32>(r, r, r, r);
}

@fragment
fn fs_rg8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let s = fetch(p.xy);
    return vec4<f32>(s.g, s.g, s.g, s.r);
}
