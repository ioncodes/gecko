import super::common::{VsOut, tex0, tex1, tex2, tex3, tex4, tex5, tex6, tex7, samp0, samp1, samp2, samp3, samp4, samp5, samp6, samp7};

// Read a single u32 from a packed array<vec4<u32>, 4> by flat index (0-15)
fn read_packed(arr: array<vec4<u32>, 4>, idx: u32) -> u32 {
    let vi = idx / 4u;
    let ci = idx % 4u;
    return arr[vi][ci];
}

// Extract `width` bits starting at bit `lo` from `val`
fn extract_bits(val: u32, lo: u32, width: u32) -> u32 {
    return (val >> lo) & ((1u << width) - 1u);
}

// Select texcoord by index (from TevOrder.texcoord)
fn select_texcoord(in: VsOut, idx: u32) -> vec2<f32> {
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

// Sample from texmap by index (from TevOrder.texmap)
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

// TEV color input selector (TevColorIn, 4-bit, 16 variants)
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

// TEV alpha input selector (TevAlphaIn, 3-bit, 8 variants)
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
