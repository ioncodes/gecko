import super::common::{FrameUniforms, DrawUniforms, VsIn, VsOut, frame, draw, tex0, tex1, tex2, tex3, tex4, tex5, tex6, tex7, samp0, samp1, samp2, samp3, samp4, samp5, samp6, samp7};
import super::tev_helpers::{read_packed, extract_bits, select_texcoord, sample_texmap, tev_color_in, tev_alpha_in};
import super::tev_combiners::{tev_combine_color, tev_combine_alpha};
import super::alpha_test::{alpha_compare, alpha_combine};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip_pos = draw.mvp * vec4<f32>(in.position, 1.0);
    // Remap depth: GameCube/OpenGL uses [-1,1], wgpu uses [0,1]
    out.clip_pos.z = out.clip_pos.z * 0.5 + out.clip_pos.w * 0.5;
    out.color = in.color;
    out.uv0 = in.tex0;
    out.uv1 = in.tex1;
    out.uv2 = in.tex2;
    out.uv3 = in.tex3;
    out.uv4 = in.tex4;
    out.uv5 = in.tex5;
    out.uv6 = in.tex6;
    out.uv7 = in.tex7;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Initialize TEV registers from uniforms
    var regs: array<vec4<f32>, 4>;
    regs[0] = frame.tev_color_reg0;
    regs[1] = frame.tev_color_reg1;
    regs[2] = frame.tev_color_reg2;
    regs[3] = frame.tev_color_reg3;

    let ras_color = in.color;
    let num_stages = frame.num_tev_stages;

    var last_color_dest = 0u;
    var last_alpha_dest = 0u;

    for (var stage = 0u; stage < 16u; stage++) {
        if stage >= num_stages {
            break;
        }

        // Resolve per-stage texture from TevOrder
        // TevStageOrder bit layout: texmap(0-2), texcoord(3-5), tex_enable(6)
        let order = read_packed(frame.tev_orders, stage);
        let texmap_idx = order & 7u;
        let texcoord_idx = (order >> 3u) & 7u;
        let tex_enabled = ((order >> 6u) & 1u) != 0u;

        var tex_color: vec4<f32>;
        if tex_enabled {
            let uv = select_texcoord(in, texcoord_idx);
            tex_color = sample_texmap(texmap_idx, uv);
        } else {
            tex_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
        }

        // Combine colors
        let cenv = read_packed(frame.tev_color_env, stage);

        // TevColorEnv bit layout (LSB0):
        //   [3:0]=d  [7:4]=c  [11:8]=b  [15:12]=a
        //   [17:16]=bias  [18]=sub  [19]=clamp  [21:20]=scale  [23:22]=dest
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

        let in_a = tev_color_in(c_a, tex_color, ras_color, regs, konst_color);
        let in_b = tev_color_in(c_b, tex_color, ras_color, regs, konst_color);
        let in_c = tev_color_in(c_c, tex_color, ras_color, regs, konst_color);
        let in_d = tev_color_in(c_d, tex_color, ras_color, regs, konst_color);

        let color_result = tev_combine_color(in_a, in_b, in_c, in_d, c_bias, c_sub, c_scale, c_clamp);
        // Write RGB to dest, preserve alpha
        regs[c_dest] = vec4(color_result, regs[c_dest].a);

        // Combine alpha
        let aenv = read_packed(frame.tev_alpha_env, stage);

        // TevAlphaEnv bit layout (LSB0):
        //   [6:4]=d  [9:7]=c  [12:10]=b  [15:13]=a
        //   [17:16]=bias  [18]=sub  [19]=clamp  [21:20]=scale  [23:22]=dest
        let a_d = extract_bits(aenv, 4u, 3u);
        let a_c = extract_bits(aenv, 7u, 3u);
        let a_b = extract_bits(aenv, 10u, 3u);
        let a_a = extract_bits(aenv, 13u, 3u);
        let a_bias = extract_bits(aenv, 16u, 2u);
        let a_sub = extract_bits(aenv, 18u, 1u) != 0u;
        let a_clamp = extract_bits(aenv, 19u, 1u) != 0u;
        let a_scale = extract_bits(aenv, 20u, 2u);
        let a_dest = extract_bits(aenv, 22u, 2u);

        let ain_a = tev_alpha_in(a_a, tex_color, ras_color, regs, konst_color);
        let ain_b = tev_alpha_in(a_b, tex_color, ras_color, regs, konst_color);
        let ain_c = tev_alpha_in(a_c, tex_color, ras_color, regs, konst_color);
        let ain_d = tev_alpha_in(a_d, tex_color, ras_color, regs, konst_color);

        let alpha_result = tev_combine_alpha(ain_a, ain_b, ain_c, ain_d, a_bias, a_sub, a_scale, a_clamp);

        // Write alpha to dest, preserve RGB
        regs[a_dest] = vec4(regs[a_dest].rgb, alpha_result);

        last_color_dest = c_dest;
        last_alpha_dest = a_dest;
    }

    // Output from the dest registers of the last active TEV stage (not always TEVPREV)
    let color = vec4<f32>(regs[last_color_dest].rgb, regs[last_alpha_dest].a);

    let c0 = alpha_compare(color.a, frame.alpha_ref0, frame.alpha_comp0);
    let c1 = alpha_compare(color.a, frame.alpha_ref1, frame.alpha_comp1);
    if !alpha_combine(c0, c1, frame.alpha_op) {
        discard;
    }

    return color;
}
