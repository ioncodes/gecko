import super::common::{VsOut, frame, tex0, tex1, tex2, tex3, tex4, tex5, tex6, tex7, samp0, samp1, samp2, samp3, samp4, samp5, samp6, samp7};

// Index into a packed array<vec4<u32>, 4> by flat index
fn read_packed(arr: array<vec4<u32>, 4>, idx: u32) -> u32 {
    let vi = idx / 4u;
    let ci = idx % 4u;
    return arr[vi][ci];
}

fn extract_bits(val: u32, lo: u32, width: u32) -> u32 {
    return (val >> lo) & ((1u << width) - 1u);
}

fn select_color_channel(color: vec4<f32>, channel: u32) -> f32 {
    switch channel {
        case 0u: { return color.r; }
        case 1u: { return color.g; }
        case 2u: { return color.b; }
        default: { return color.a; }
    }
}

fn swizzle_tev_color(swap_table: u32, color: vec4<f32>) -> vec4<f32> {
    let rg = read_packed(frame.tev_ksel, swap_table * 2u);
    let ba = read_packed(frame.tev_ksel, swap_table * 2u + 1u);
    let r = extract_bits(rg, 0u, 2u);
    let g = extract_bits(rg, 2u, 2u);
    let b = extract_bits(ba, 0u, 2u);
    let a = extract_bits(ba, 2u, 2u);

    return vec4<f32>(
        select_color_channel(color, r),
        select_color_channel(color, g),
        select_color_channel(color, b),
        select_color_channel(color, a),
    );
}

fn select_texcoord(in: VsOut, idx: u32) -> vec3<f32> {
    switch idx {
        case 0u: { return in.uv0; }
        case 1u: { return in.uv1; }
        case 2u: { return in.uv2; }
        case 3u: { return in.uv3; }
        case 4u: { return in.uv4; }
        case 5u: { return in.uv5; }
        case 6u: { return in.uv6; }
        case 7u: { return in.uv7; }
        default: { return in.uv0; }
    }
}

// Perform the perspective divide on 3-component (s, t, q) texcoords.
// For non-projective texcoords q == 1 and the divide is a no-op.
// The q == 0 special case was already handled on the CPU (values are
// clamped, so we use them directly).
fn stq_to_uv(stq: vec3<f32>) -> vec2<f32> {
    if abs(stq.z) < 0.00001 {
        return stq.xy;
    }
    return stq.xy / stq.z;
}

fn sample_texmap(texmap: u32, uv: vec2<f32>) -> vec4<f32> {
    switch texmap {
        case 0u: { return textureSample(tex0, samp0, uv); }
        case 1u: { return textureSample(tex1, samp1, uv); }
        case 2u: { return textureSample(tex2, samp2, uv); }
        case 3u: { return textureSample(tex3, samp3, uv); }
        case 4u: { return textureSample(tex4, samp4, uv); }
        case 5u: { return textureSample(tex5, samp5, uv); }
        case 6u: { return textureSample(tex6, samp6, uv); }
        case 7u: { return textureSample(tex7, samp7, uv); }
        default: { return textureSample(tex0, samp0, uv); }
    }
}

// TevColorIn: 4-bit selector, 16 variants
fn tev_color_in(sel: u32, tex_color: vec4<f32>, ras_color: vec4<f32>, regs: array<vec4<f32>, 4>, konst_color: vec4<f32>) -> vec3<f32> {
    switch sel {
        case 0u:  { return regs[0].rgb; }           // PrevColor
        case 1u:  { return vec3(regs[0].a); }       // PrevAlpha
        case 2u:  { return regs[1].rgb; }           // Reg0Color
        case 3u:  { return vec3(regs[1].a); }       // Reg0Alpha
        case 4u:  { return regs[2].rgb; }           // Reg1Color
        case 5u:  { return vec3(regs[2].a); }       // Reg1Alpha
        case 6u:  { return regs[3].rgb; }           // Reg2Color
        case 7u:  { return vec3(regs[3].a); }       // Reg2Alpha
        case 8u:  { return tex_color.rgb; }         // TexColor
        case 9u:  { return vec3(tex_color.a); }     // TexAlpha
        case 10u: { return ras_color.rgb; }         // RasColor
        case 11u: { return vec3(ras_color.a); }     // RasAlpha
        case 12u: { return vec3(1.0); }             // One
        case 13u: { return vec3(0.5); }             // Half
        case 14u: { return konst_color.rgb; }       // Konst
        case 15u: { return vec3(0.0); }             // Zero
        default:  { return vec3(0.0); }
    }
}

// TevAlphaIn: 3-bit selector, 8 variants
fn tev_alpha_in(sel: u32, tex_color: vec4<f32>, ras_color: vec4<f32>, regs: array<vec4<f32>, 4>, konst_color: vec4<f32>) -> f32 {
    switch sel {
        case 0u: { return regs[0].a; }      // PrevAlpha
        case 1u: { return regs[1].a; }      // Reg0Alpha
        case 2u: { return regs[2].a; }      // Reg1Alpha
        case 3u: { return regs[3].a; }      // Reg2Alpha
        case 4u: { return tex_color.a; }    // TexAlpha
        case 5u: { return ras_color.a; }    // RasAlpha
        case 6u: { return konst_color.a; }  // Konst
        case 7u: { return 0.0; }            // Zero
        default: { return 0.0; }
    }
}
