import super::common::{VsOut, FrameUniforms, frame, tex0, tex1, tex2, tex3, tex4, tex5, tex6, tex7, samp0, samp1, samp2, samp3, samp4, samp5, samp6, samp7};
import super::tev_helpers::{select_texcoord, stq_to_uv, sample_texmap, read_packed};

fn texmap_size(texmap: u32) -> vec2<f32> {
    switch texmap {
        case 0u: { return vec2<f32>(textureDimensions(tex0)); }
        case 1u: { return vec2<f32>(textureDimensions(tex1)); }
        case 2u: { return vec2<f32>(textureDimensions(tex2)); }
        case 3u: { return vec2<f32>(textureDimensions(tex3)); }
        case 4u: { return vec2<f32>(textureDimensions(tex4)); }
        case 5u: { return vec2<f32>(textureDimensions(tex5)); }
        case 6u: { return vec2<f32>(textureDimensions(tex6)); }
        default: { return vec2<f32>(textureDimensions(tex7)); }
    }
}

fn ind_quantize_and_bias(
    sample: vec3<f32>,
    format: u32,
    bias_s: u32,
    bias_t: u32,
    bias_u: u32,
) -> vec3<f32> {
    let bias_mask = vec3<f32>(f32(bias_s), f32(bias_t), f32(bias_u));
    switch format {
        case 0u: {
            let byte = floor(sample * 255.0);
            return byte - 128.0 * bias_mask;
        }
        case 1u: {
            let byte = floor(sample * 31.0);
            return byte + bias_mask;
        }
        case 2u: {
            let byte = floor(sample * 15.0);
            return byte + bias_mask;
        }
        default: {
            let byte = floor(sample * 7.0);
            return byte + bias_mask;
        }
    }
}

fn ind_bump_alpha_byte(sample_byte_u8: u32, format: u32) -> u32 {
    var shift: u32 = 0u;
    switch format {
        case 0u: { shift = 0u; }
        case 1u: { shift = 5u; }
        case 2u: { shift = 4u; }
        default: { shift = 3u; }
    }
    return (sample_byte_u8 << shift) & 0xF8u;
}

fn ind_wrap_texels(mode: u32) -> f32 {
    return 256.0 / exp2(f32(mode - 1u));
}

fn ind_wrap_axis(uv_coord: f32, mode: u32, tex_size: f32) -> f32 {
    if mode == 0u {
        return uv_coord;
    }
    if mode >= 6u {
        return 0.0;
    }
    let period = ind_wrap_texels(mode) / tex_size;
    return fract(uv_coord / period) * period;
}

struct IndirectSamples {
    samples: array<vec3<f32>, 4>,
}

fn sample_indirect_one(i: u32, in: VsOut, result: ptr<function, IndirectSamples>) {
    let shift = i * 6u;
    let texmap = (frame.indirect_refs >> shift) & 7u;
    let texcoord = (frame.indirect_refs >> (shift + 3u)) & 7u;
    let scale_pair = frame.indirect_scales[i / 2u];
    let scale_s: u32 = select(scale_pair.x, scale_pair.z, (i & 1u) == 1u);
    let scale_t: u32 = select(scale_pair.y, scale_pair.w, (i & 1u) == 1u);

    let stq = select_texcoord(in, texcoord);
    let direct_uv = stq_to_uv(stq);
    let sample_uv = vec2<f32>(
        direct_uv.x * exp2(-f32(scale_s)),
        direct_uv.y * exp2(-f32(scale_t)),
    );
    let sample = sample_texmap(texmap, sample_uv);
    (*result).samples[i] = sample.abg;
}

fn sample_indirect_textures(in: VsOut) -> IndirectSamples {
    var result: IndirectSamples;
    result.samples[0] = vec3<f32>(0.0);
    result.samples[1] = vec3<f32>(0.0);
    result.samples[2] = vec3<f32>(0.0);
    result.samples[3] = vec3<f32>(0.0);

    @if(IND_STAGE_0_ENABLED) { sample_indirect_one(0u, in, &result); }
    @if(IND_STAGE_1_ENABLED) { sample_indirect_one(1u, in, &result); }
    @if(IND_STAGE_2_ENABLED) { sample_indirect_one(2u, in, &result); }
    @if(IND_STAGE_3_ENABLED) { sample_indirect_one(3u, in, &result); }
    return result;
}

struct IndirectResult {
    uv: vec2<f32>,
    bump_alpha: f32,
}

fn apply_indirect(
    cmd: u32,
    direct_uv: vec2<f32>,
    direct_tex_size: vec2<f32>,
    presampled: IndirectSamples,
    previous_uv: vec2<f32>,
) -> IndirectResult {
    var result: IndirectResult;
    result.uv = direct_uv;
    result.bump_alpha = 0.0;

    @if(!IND_STAGE_0_ENABLED) {
        return result;
    }

    @if(IND_STAGE_0_ENABLED) {
        let source_stage = cmd & 3u;
        let format = (cmd >> 2u) & 3u;
        let bias_s = (cmd >> 4u) & 1u;
        let bias_t = (cmd >> 5u) & 1u;
        let bias_u = (cmd >> 6u) & 1u;
        let bump_source = (cmd >> 7u) & 3u;
        let matrix_index = (cmd >> 9u) & 3u;
        let matrix_mode = (cmd >> 11u) & 3u;
        let wrap_s = (cmd >> 13u) & 7u;
        let wrap_t = (cmd >> 16u) & 7u;
        let add_previous = ((cmd >> 20u) & 1u) != 0u;

        let sample = presampled.samples[source_stage];

        if bump_source != 0u {
            var raw_byte: f32 = 0.0;
            switch bump_source {
                case 1u: { raw_byte = sample.x; }
                case 2u: { raw_byte = sample.y; }
                case 3u: { raw_byte = sample.z; }
                default: {}
            }
            let byte_u8 = u32(floor(raw_byte * 255.0));
            result.bump_alpha = f32(ind_bump_alpha_byte(byte_u8, format)) / 255.0;
        }

        let has_offset = matrix_index != 0u || wrap_s != 0u || wrap_t != 0u || add_previous;
        if !has_offset {
            return result;
        }

        let quantized = ind_quantize_and_bias(sample, format, bias_s, bias_t, bias_u);

        var offset_uv = vec2<f32>(0.0);
        if matrix_index != 0u {
            let idx = matrix_index - 1u;
            let row0 = vec3<f32>(frame.indirect_matrices[idx * 2u].xyz) * (1.0 / 1024.0);
            let row1 = vec3<f32>(frame.indirect_matrices[idx * 2u + 1u].xyz) * (1.0 / 1024.0);
            let scale_exp = frame.indirect_matrices[idx * 2u].w;
            let inv_scale = exp2(-f32(scale_exp));
            switch matrix_mode {
                case 0u: {
                    offset_uv = vec2<f32>(dot(row0, quantized), dot(row1, quantized))
                              * inv_scale
                              / direct_tex_size;
                }
                case 1u: {
                    offset_uv = direct_uv * (quantized.x * (1.0 / 256.0) * inv_scale);
                }
                case 2u: {
                    offset_uv = direct_uv * (quantized.y * (1.0 / 256.0) * inv_scale);
                }
                default: {}
            }
        }

        let wrapped = vec2<f32>(
            ind_wrap_axis(direct_uv.x, wrap_s, direct_tex_size.x),
            ind_wrap_axis(direct_uv.y, wrap_t, direct_tex_size.y),
        );
        var final_uv = wrapped + offset_uv;
        if add_previous {
            final_uv = final_uv + previous_uv;
        }
        result.uv = final_uv;
        return result;
    }
}
