import super::common::{FrameUniforms, DrawUniforms, VsIn, VsOut, frame, draw, tex0, tex1, tex2, tex3, tex4, tex5, tex6, tex7, samp0, samp1, samp2, samp3, samp4, samp5, samp6, samp7};
import super::tev_helpers::{extract_bits, select_texcoord, stq_to_uv, sample_texmap, swizzle_tev_color, tev_color_in, tev_alpha_in};
import super::tev_indirect::{IndirectSamples, IndirectResult, sample_indirect_textures, apply_indirect, texmap_size};
import super::tev_combiners::{tev_combine_color, tev_combine_alpha};
import super::alpha_test::{alpha_compare, alpha_combine};
import super::lighting::{compute_lighting_c0, compute_lighting_c1};

override gx_ubershader: bool = false;
override gx_tev_color_env_0: u32 = 0u;
override gx_tev_color_env_1: u32 = 0u;
override gx_tev_color_env_2: u32 = 0u;
override gx_tev_color_env_3: u32 = 0u;
override gx_tev_color_env_4: u32 = 0u;
override gx_tev_color_env_5: u32 = 0u;
override gx_tev_color_env_6: u32 = 0u;
override gx_tev_color_env_7: u32 = 0u;
override gx_tev_color_env_8: u32 = 0u;
override gx_tev_color_env_9: u32 = 0u;
override gx_tev_color_env_10: u32 = 0u;
override gx_tev_color_env_11: u32 = 0u;
override gx_tev_color_env_12: u32 = 0u;
override gx_tev_color_env_13: u32 = 0u;
override gx_tev_color_env_14: u32 = 0u;
override gx_tev_color_env_15: u32 = 0u;

override gx_tev_alpha_env_0: u32 = 0u;
override gx_tev_alpha_env_1: u32 = 0u;
override gx_tev_alpha_env_2: u32 = 0u;
override gx_tev_alpha_env_3: u32 = 0u;
override gx_tev_alpha_env_4: u32 = 0u;
override gx_tev_alpha_env_5: u32 = 0u;
override gx_tev_alpha_env_6: u32 = 0u;
override gx_tev_alpha_env_7: u32 = 0u;
override gx_tev_alpha_env_8: u32 = 0u;
override gx_tev_alpha_env_9: u32 = 0u;
override gx_tev_alpha_env_10: u32 = 0u;
override gx_tev_alpha_env_11: u32 = 0u;
override gx_tev_alpha_env_12: u32 = 0u;
override gx_tev_alpha_env_13: u32 = 0u;
override gx_tev_alpha_env_14: u32 = 0u;
override gx_tev_alpha_env_15: u32 = 0u;

override gx_tev_order_0: u32 = 0u;
override gx_tev_order_1: u32 = 0u;
override gx_tev_order_2: u32 = 0u;
override gx_tev_order_3: u32 = 0u;
override gx_tev_order_4: u32 = 0u;
override gx_tev_order_5: u32 = 0u;
override gx_tev_order_6: u32 = 0u;
override gx_tev_order_7: u32 = 0u;
override gx_tev_order_8: u32 = 0u;
override gx_tev_order_9: u32 = 0u;
override gx_tev_order_10: u32 = 0u;
override gx_tev_order_11: u32 = 0u;
override gx_tev_order_12: u32 = 0u;
override gx_tev_order_13: u32 = 0u;
override gx_tev_order_14: u32 = 0u;
override gx_tev_order_15: u32 = 0u;

override gx_tev_indirect_0: u32 = 0u;
override gx_tev_indirect_1: u32 = 0u;
override gx_tev_indirect_2: u32 = 0u;
override gx_tev_indirect_3: u32 = 0u;
override gx_tev_indirect_4: u32 = 0u;
override gx_tev_indirect_5: u32 = 0u;
override gx_tev_indirect_6: u32 = 0u;
override gx_tev_indirect_7: u32 = 0u;
override gx_tev_indirect_8: u32 = 0u;
override gx_tev_indirect_9: u32 = 0u;
override gx_tev_indirect_10: u32 = 0u;
override gx_tev_indirect_11: u32 = 0u;
override gx_tev_indirect_12: u32 = 0u;
override gx_tev_indirect_13: u32 = 0u;
override gx_tev_indirect_14: u32 = 0u;
override gx_tev_indirect_15: u32 = 0u;

override gx_tev_ksel_0: u32 = 0u;
override gx_tev_ksel_1: u32 = 0u;
override gx_tev_ksel_2: u32 = 0u;
override gx_tev_ksel_3: u32 = 0u;
override gx_tev_ksel_4: u32 = 0u;
override gx_tev_ksel_5: u32 = 0u;
override gx_tev_ksel_6: u32 = 0u;
override gx_tev_ksel_7: u32 = 0u;
override gx_indirect_refs: u32 = 0u;

override gx_color_ctrl_0: u32 = 0u;
override gx_color_ctrl_1: u32 = 0u;
override gx_alpha_ctrl_0: u32 = 0u;
override gx_alpha_ctrl_1: u32 = 0u;
override gx_alpha_comp_0: u32 = 7u;
override gx_alpha_comp_1: u32 = 7u;
override gx_alpha_op: u32 = 0u;
override gx_ztex_type: u32 = 0u;
override gx_ztex_op: u32 = 0u;

fn shader_state(specialized: u32, dynamic: u32) -> u32 {
    return select(specialized, dynamic, gx_ubershader);
}

fn dynamic_tev_color_env(stage: u32) -> u32 {
    return frame.tev_color_env[stage / 4u][stage % 4u];
}

fn dynamic_tev_alpha_env(stage: u32) -> u32 {
    return frame.tev_alpha_env[stage / 4u][stage % 4u];
}

fn dynamic_tev_order(stage: u32) -> u32 {
    return frame.tev_orders[stage / 4u][stage % 4u];
}

fn dynamic_tev_indirect(stage: u32) -> u32 {
    return frame.tev_indirect[stage / 4u][stage % 4u];
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip_pos = draw.mvp * vec4<f32>(in.pos_view, 1.0);
    out.clip_pos.z = out.clip_pos.z + out.clip_pos.w;
    out.color = compute_lighting_c0(
        shader_state(gx_color_ctrl_0, frame.color_ctrl0),
        shader_state(gx_alpha_ctrl_0, frame.alpha_ctrl0),
        frame.ambient_color0, frame.material_color0,
        in.color, in.normal, in.pos_view,
    );
    out.color1 = compute_lighting_c1(
        shader_state(gx_color_ctrl_1, frame.color_ctrl1),
        shader_state(gx_alpha_ctrl_1, frame.alpha_ctrl1),
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
    // Raw (pre-swap-table) texel of the last TEV stage that sampled a
    // texture. Feeds the Z-texture value in fs_main_ztex (hardware: the
    // PE's RawTexColor latch).
    last_raw_tex: vec4<f32>,
}

fn specialized_tev_ksel(idx: u32) -> u32 {
    var specialized: u32;
    switch idx {
        case 0u: { specialized = gx_tev_ksel_0; }
        case 1u: { specialized = gx_tev_ksel_1; }
        case 2u: { specialized = gx_tev_ksel_2; }
        case 3u: { specialized = gx_tev_ksel_3; }
        case 4u: { specialized = gx_tev_ksel_4; }
        case 5u: { specialized = gx_tev_ksel_5; }
        case 6u: { specialized = gx_tev_ksel_6; }
        default: { specialized = gx_tev_ksel_7; }
    }
    return shader_state(specialized, frame.tev_ksel[idx / 4u][idx % 4u]);
}

fn run_stage(
    stage: u32,
    order: u32,
    cenv: u32,
    aenv: u32,
    ind_cmd: u32,
    in: VsOut,
    color0: vec4<f32>,
    presampled: IndirectSamples,
    state: ptr<function, TevState>,
) {
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
        (*state).last_raw_tex = tex_color;
    } else {
        tex_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let ras_swap = extract_bits(aenv, 0u, 2u) * 2u;
    let tex_swap = extract_bits(aenv, 2u, 2u) * 2u;
    ras_color = swizzle_tev_color(
        specialized_tev_ksel(ras_swap),
        specialized_tev_ksel(ras_swap + 1u),
        ras_color,
    );
    tex_color = swizzle_tev_color(
        specialized_tev_ksel(tex_swap),
        specialized_tev_ksel(tex_swap + 1u),
        tex_color,
    );

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

struct TevResult {
    color: vec4<f32>,
    raw_tex: vec4<f32>,
}

fn run_tev(in: VsOut) -> TevResult {
    var state: TevState;
    state.regs[0] = frame.tev_color_reg0;
    state.regs[1] = frame.tev_color_reg1;
    state.regs[2] = frame.tev_color_reg2;
    state.regs[3] = frame.tev_color_reg3;
    state.running_uv = vec2<f32>(0.0, 0.0);
    state.last_color_dest = 0u;
    state.last_alpha_dest = 0u;
    state.last_raw_tex = vec4<f32>(0.0, 0.0, 0.0, 0.0);

    let color0 = in.color;
    let presampled = sample_indirect_textures(in, shader_state(gx_indirect_refs, frame.indirect_refs));

    @if(TEV_STAGE_1_ENABLED)  { run_stage( 0u, shader_state(gx_tev_order_0,  dynamic_tev_order( 0u)), shader_state(gx_tev_color_env_0,  dynamic_tev_color_env( 0u)), shader_state(gx_tev_alpha_env_0,  dynamic_tev_alpha_env( 0u)), shader_state(gx_tev_indirect_0,  dynamic_tev_indirect( 0u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_2_ENABLED)  { run_stage( 1u, shader_state(gx_tev_order_1,  dynamic_tev_order( 1u)), shader_state(gx_tev_color_env_1,  dynamic_tev_color_env( 1u)), shader_state(gx_tev_alpha_env_1,  dynamic_tev_alpha_env( 1u)), shader_state(gx_tev_indirect_1,  dynamic_tev_indirect( 1u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_3_ENABLED)  { run_stage( 2u, shader_state(gx_tev_order_2,  dynamic_tev_order( 2u)), shader_state(gx_tev_color_env_2,  dynamic_tev_color_env( 2u)), shader_state(gx_tev_alpha_env_2,  dynamic_tev_alpha_env( 2u)), shader_state(gx_tev_indirect_2,  dynamic_tev_indirect( 2u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_4_ENABLED)  { run_stage( 3u, shader_state(gx_tev_order_3,  dynamic_tev_order( 3u)), shader_state(gx_tev_color_env_3,  dynamic_tev_color_env( 3u)), shader_state(gx_tev_alpha_env_3,  dynamic_tev_alpha_env( 3u)), shader_state(gx_tev_indirect_3,  dynamic_tev_indirect( 3u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_5_ENABLED)  { run_stage( 4u, shader_state(gx_tev_order_4,  dynamic_tev_order( 4u)), shader_state(gx_tev_color_env_4,  dynamic_tev_color_env( 4u)), shader_state(gx_tev_alpha_env_4,  dynamic_tev_alpha_env( 4u)), shader_state(gx_tev_indirect_4,  dynamic_tev_indirect( 4u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_6_ENABLED)  { run_stage( 5u, shader_state(gx_tev_order_5,  dynamic_tev_order( 5u)), shader_state(gx_tev_color_env_5,  dynamic_tev_color_env( 5u)), shader_state(gx_tev_alpha_env_5,  dynamic_tev_alpha_env( 5u)), shader_state(gx_tev_indirect_5,  dynamic_tev_indirect( 5u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_7_ENABLED)  { run_stage( 6u, shader_state(gx_tev_order_6,  dynamic_tev_order( 6u)), shader_state(gx_tev_color_env_6,  dynamic_tev_color_env( 6u)), shader_state(gx_tev_alpha_env_6,  dynamic_tev_alpha_env( 6u)), shader_state(gx_tev_indirect_6,  dynamic_tev_indirect( 6u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_8_ENABLED)  { run_stage( 7u, shader_state(gx_tev_order_7,  dynamic_tev_order( 7u)), shader_state(gx_tev_color_env_7,  dynamic_tev_color_env( 7u)), shader_state(gx_tev_alpha_env_7,  dynamic_tev_alpha_env( 7u)), shader_state(gx_tev_indirect_7,  dynamic_tev_indirect( 7u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_9_ENABLED)  { run_stage( 8u, shader_state(gx_tev_order_8,  dynamic_tev_order( 8u)), shader_state(gx_tev_color_env_8,  dynamic_tev_color_env( 8u)), shader_state(gx_tev_alpha_env_8,  dynamic_tev_alpha_env( 8u)), shader_state(gx_tev_indirect_8,  dynamic_tev_indirect( 8u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_10_ENABLED) { run_stage( 9u, shader_state(gx_tev_order_9,  dynamic_tev_order( 9u)), shader_state(gx_tev_color_env_9,  dynamic_tev_color_env( 9u)), shader_state(gx_tev_alpha_env_9,  dynamic_tev_alpha_env( 9u)), shader_state(gx_tev_indirect_9,  dynamic_tev_indirect( 9u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_11_ENABLED) { run_stage(10u, shader_state(gx_tev_order_10, dynamic_tev_order(10u)), shader_state(gx_tev_color_env_10, dynamic_tev_color_env(10u)), shader_state(gx_tev_alpha_env_10, dynamic_tev_alpha_env(10u)), shader_state(gx_tev_indirect_10, dynamic_tev_indirect(10u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_12_ENABLED) { run_stage(11u, shader_state(gx_tev_order_11, dynamic_tev_order(11u)), shader_state(gx_tev_color_env_11, dynamic_tev_color_env(11u)), shader_state(gx_tev_alpha_env_11, dynamic_tev_alpha_env(11u)), shader_state(gx_tev_indirect_11, dynamic_tev_indirect(11u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_13_ENABLED) { run_stage(12u, shader_state(gx_tev_order_12, dynamic_tev_order(12u)), shader_state(gx_tev_color_env_12, dynamic_tev_color_env(12u)), shader_state(gx_tev_alpha_env_12, dynamic_tev_alpha_env(12u)), shader_state(gx_tev_indirect_12, dynamic_tev_indirect(12u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_14_ENABLED) { run_stage(13u, shader_state(gx_tev_order_13, dynamic_tev_order(13u)), shader_state(gx_tev_color_env_13, dynamic_tev_color_env(13u)), shader_state(gx_tev_alpha_env_13, dynamic_tev_alpha_env(13u)), shader_state(gx_tev_indirect_13, dynamic_tev_indirect(13u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_15_ENABLED) { run_stage(14u, shader_state(gx_tev_order_14, dynamic_tev_order(14u)), shader_state(gx_tev_color_env_14, dynamic_tev_color_env(14u)), shader_state(gx_tev_alpha_env_14, dynamic_tev_alpha_env(14u)), shader_state(gx_tev_indirect_14, dynamic_tev_indirect(14u)), in, color0, presampled, &state); }
    @if(TEV_STAGE_16_ENABLED) { run_stage(15u, shader_state(gx_tev_order_15, dynamic_tev_order(15u)), shader_state(gx_tev_color_env_15, dynamic_tev_color_env(15u)), shader_state(gx_tev_alpha_env_15, dynamic_tev_alpha_env(15u)), shader_state(gx_tev_indirect_15, dynamic_tev_indirect(15u)), in, color0, presampled, &state); }

    let color = vec4<f32>(state.regs[state.last_color_dest].rgb, state.regs[state.last_alpha_dest].a);

    @if(ALPHA_TEST_ENABLED) {
        let c0 = alpha_compare(color.a, frame.alpha_ref0, shader_state(gx_alpha_comp_0, frame.alpha_comp0));
        let c1 = alpha_compare(color.a, frame.alpha_ref1, shader_state(gx_alpha_comp_1, frame.alpha_comp1));
        if !alpha_combine(c0, c1, shader_state(gx_alpha_op, frame.alpha_op)) {
            discard;
        }
    }

    var result: TevResult;
    result.color = color;
    result.raw_tex = state.last_raw_tex;
    return result;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return run_tev(in).color;
}

struct FsOutZtex {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_main_ztex(in: VsOut) -> FsOutZtex {
    let r = run_tev(in);
    let texel = vec4<u32>(round(r.raw_tex * 255.0));
    var z: u32 = frame.ztex_bias;
    switch shader_state(gx_ztex_type, frame.ztex_type) {
        case 0u: { z += texel.a; }
        case 1u: { z += (texel.a << 8u) | texel.r; }
        default: { z += (texel.r << 16u) | (texel.g << 8u) | texel.b; }
    }
    if shader_state(gx_ztex_op, frame.ztex_op) == 1u {
        z += u32(round(in.clip_pos.z * 16777215.0));
    }
    z = z & 0x00FFFFFFu;

    var out_: FsOutZtex;
    out_.color = r.color;
    out_.depth = f32(z) / 16777215.0;
    return out_;
}
