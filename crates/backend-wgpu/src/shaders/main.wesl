import super::common::{FrameUniforms, DrawUniforms, VsIn, VsOut, frame, draw, tex0, tex1, tex2, tex3, tex4, tex5, tex6, tex7, samp0, samp1, samp2, samp3, samp4, samp5, samp6, samp7};
import super::tev_helpers::{extract_bits, select_texcoord, stq_to_uv, sample_texmap, swizzle_tev_color, tev_color_in, tev_alpha_in};
import super::tev_indirect::{IndirectSamples, IndirectResult, sample_indirect_textures, apply_indirect, texmap_size};
import super::tev_combiners::{tev_combine_color, tev_combine_alpha};
import super::alpha_test::{alpha_compare, alpha_combine};
import super::lighting::{compute_lighting_c0, compute_lighting_c1};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip_pos = draw.mvp * vec4<f32>(in.pos_view, 1.0);
    out.clip_pos.z = out.clip_pos.z + out.clip_pos.w;
    out.color = compute_lighting_c0(
        frame.color_ctrl0, frame.alpha_ctrl0,
        frame.ambient_color0, frame.material_color0,
        in.color, in.normal, in.pos_view,
    );
    out.color1 = compute_lighting_c1(
        frame.color_ctrl1, frame.alpha_ctrl1,
        frame.ambient_color1, frame.material_color1,
        in.color1, in.normal, in.pos_view,
    );

    @if(TEXCOORD_0_ENABLED) { out.uv0 = in.tex0; } @else { out.uv0 = vec3<f32>(0.0, 0.0, 1.0); }
    @if(TEXCOORD_1_ENABLED) { out.uv1 = in.tex1; } @else { out.uv1 = vec3<f32>(0.0, 0.0, 1.0); }
    @if(TEXCOORD_2_ENABLED) { out.uv2 = in.tex2; } @else { out.uv2 = vec3<f32>(0.0, 0.0, 1.0); }
    @if(TEXCOORD_3_ENABLED) { out.uv3 = in.tex3; } @else { out.uv3 = vec3<f32>(0.0, 0.0, 1.0); }
    @if(TEXCOORD_4_ENABLED) { out.uv4 = in.tex4; } @else { out.uv4 = vec3<f32>(0.0, 0.0, 1.0); }
    @if(TEXCOORD_5_ENABLED) { out.uv5 = in.tex5; } @else { out.uv5 = vec3<f32>(0.0, 0.0, 1.0); }
    @if(TEXCOORD_6_ENABLED) { out.uv6 = in.tex6; } @else { out.uv6 = vec3<f32>(0.0, 0.0, 1.0); }
    @if(TEXCOORD_7_ENABLED) { out.uv7 = in.tex7; } @else { out.uv7 = vec3<f32>(0.0, 0.0, 1.0); }
    
    return out;
}

struct TevState {
    regs: array<vec4<f32>, 4>,
    running_uv: vec2<f32>,
    last_color_dest: u32,
    last_alpha_dest: u32,
}

fn run_stage(
    stage: u32,
    in: VsOut,
    color0: vec4<f32>,
    presampled: IndirectSamples,
    state: ptr<function, TevState>,
) {
    let row = stage / 4u;
    let col = stage % 4u;
    let order = frame.tev_orders[row][col];
    let cenv = frame.tev_color_env[row][col];
    let aenv = frame.tev_alpha_env[row][col];
    let ind_cmd = frame.tev_indirect[row][col];

    let texmap_idx = order & 7u;
    let texcoord_idx = (order >> 3u) & 7u;
    let tex_enabled = ((order >> 6u) & 1u) != 0u;
    let ras_channel = (order >> 7u) & 7u;

    let stq = select_texcoord(in, texcoord_idx);
    let direct_uv = stq_to_uv(stq);
    let direct_size = texmap_size(texmap_idx);
    let indirect = apply_indirect(ind_cmd, direct_uv, direct_size, presampled, (*state).running_uv);
    (*state).running_uv = indirect.uv;

    var ras_color: vec4<f32>;
    switch ras_channel {
        case 0u: { ras_color = color0; }
        case 1u: { ras_color = in.color1; }
        case 2u: { ras_color = vec4(color0.a, color0.a, color0.a, color0.a); }
        case 3u: { ras_color = vec4(in.color1.a, in.color1.a, in.color1.a, in.color1.a); }
        case 4u: { ras_color = color0; }
        case 5u: { ras_color = in.color1; }
        case 6u: { ras_color = vec4(0.0, 0.0, 0.0, 0.0); }
        default: { ras_color = vec4(indirect.bump_alpha); }
    }

    var tex_color: vec4<f32>;
    if tex_enabled {
        tex_color = sample_texmap(texmap_idx, indirect.uv);
    } else {
        tex_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    ras_color = swizzle_tev_color(extract_bits(aenv, 0u, 2u), ras_color);
    tex_color = swizzle_tev_color(extract_bits(aenv, 2u, 2u), tex_color);

    let c_d = extract_bits(cenv, 0u, 4u);
    let c_c = extract_bits(cenv, 4u, 4u);
    let c_b = extract_bits(cenv, 8u, 4u);
    let c_a = extract_bits(cenv, 12u, 4u);
    let c_bias = extract_bits(cenv, 16u, 2u);
    let c_sub = extract_bits(cenv, 18u, 1u) != 0u;
    let c_clamp = extract_bits(cenv, 19u, 1u) != 0u;
    let c_scale = extract_bits(cenv, 20u, 2u);
    let c_dest = extract_bits(cenv, 22u, 2u);

    let konst_color = frame.tev_konst_colors[stage];

    let in_a = tev_color_in(c_a, tex_color, ras_color, (*state).regs, konst_color);
    let in_b = tev_color_in(c_b, tex_color, ras_color, (*state).regs, konst_color);
    let in_c = tev_color_in(c_c, tex_color, ras_color, (*state).regs, konst_color);
    let in_d = tev_color_in(c_d, tex_color, ras_color, (*state).regs, konst_color);

    let color_result = tev_combine_color(in_a, in_b, in_c, in_d, c_bias, c_sub, c_scale, c_clamp);
    (*state).regs[c_dest] = vec4(color_result, (*state).regs[c_dest].a);

    let a_d = extract_bits(aenv, 4u, 3u);
    let a_c = extract_bits(aenv, 7u, 3u);
    let a_b = extract_bits(aenv, 10u, 3u);
    let a_a = extract_bits(aenv, 13u, 3u);
    let a_bias = extract_bits(aenv, 16u, 2u);
    let a_sub = extract_bits(aenv, 18u, 1u) != 0u;
    let a_clamp = extract_bits(aenv, 19u, 1u) != 0u;
    let a_scale = extract_bits(aenv, 20u, 2u);
    let a_dest = extract_bits(aenv, 22u, 2u);

    let ain_a = tev_alpha_in(a_a, tex_color, ras_color, (*state).regs, konst_color);
    let ain_b = tev_alpha_in(a_b, tex_color, ras_color, (*state).regs, konst_color);
    let ain_c = tev_alpha_in(a_c, tex_color, ras_color, (*state).regs, konst_color);
    let ain_d = tev_alpha_in(a_d, tex_color, ras_color, (*state).regs, konst_color);

    let alpha_result = tev_combine_alpha(ain_a, ain_b, ain_c, ain_d, in_a, in_b, a_bias, a_sub, a_scale, a_clamp);
    (*state).regs[a_dest] = vec4((*state).regs[a_dest].rgb, alpha_result);

    (*state).last_color_dest = c_dest;
    (*state).last_alpha_dest = a_dest;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var state: TevState;
    state.regs[0] = frame.tev_color_reg0;
    state.regs[1] = frame.tev_color_reg1;
    state.regs[2] = frame.tev_color_reg2;
    state.regs[3] = frame.tev_color_reg3;
    state.running_uv = vec2<f32>(0.0, 0.0);
    state.last_color_dest = 0u;
    state.last_alpha_dest = 0u;

    let color0 = in.color;
    let presampled = sample_indirect_textures(in);

    @if(TEV_STAGE_1_ENABLED)  { run_stage( 0u, in, color0, presampled, &state); }
    @if(TEV_STAGE_2_ENABLED)  { run_stage( 1u, in, color0, presampled, &state); }
    @if(TEV_STAGE_3_ENABLED)  { run_stage( 2u, in, color0, presampled, &state); }
    @if(TEV_STAGE_4_ENABLED)  { run_stage( 3u, in, color0, presampled, &state); }
    @if(TEV_STAGE_5_ENABLED)  { run_stage( 4u, in, color0, presampled, &state); }
    @if(TEV_STAGE_6_ENABLED)  { run_stage( 5u, in, color0, presampled, &state); }
    @if(TEV_STAGE_7_ENABLED)  { run_stage( 6u, in, color0, presampled, &state); }
    @if(TEV_STAGE_8_ENABLED)  { run_stage( 7u, in, color0, presampled, &state); }
    @if(TEV_STAGE_9_ENABLED)  { run_stage( 8u, in, color0, presampled, &state); }
    @if(TEV_STAGE_10_ENABLED) { run_stage( 9u, in, color0, presampled, &state); }
    @if(TEV_STAGE_11_ENABLED) { run_stage(10u, in, color0, presampled, &state); }
    @if(TEV_STAGE_12_ENABLED) { run_stage(11u, in, color0, presampled, &state); }
    @if(TEV_STAGE_13_ENABLED) { run_stage(12u, in, color0, presampled, &state); }
    @if(TEV_STAGE_14_ENABLED) { run_stage(13u, in, color0, presampled, &state); }
    @if(TEV_STAGE_15_ENABLED) { run_stage(14u, in, color0, presampled, &state); }
    @if(TEV_STAGE_16_ENABLED) { run_stage(15u, in, color0, presampled, &state); }

    let color = vec4<f32>(state.regs[state.last_color_dest].rgb, state.regs[state.last_alpha_dest].a);

    @if(ALPHA_TEST_ENABLED) {
        let c0 = alpha_compare(color.a, frame.alpha_ref0, frame.alpha_comp0);
        let c1 = alpha_compare(color.a, frame.alpha_ref1, frame.alpha_comp1);
        if !alpha_combine(c0, c1, frame.alpha_op) {
            discard;
        }
    }

    return color;
}
