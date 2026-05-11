import super::common::{FrameUniforms, frame};
import super::tev_helpers::extract_bits;

fn safe_div(n: f32, d: f32) -> f32 {
    if abs(d) < 1e-10 {
        return 0.0;
    }
    return n / d;
}

fn compute_light_factor(ctrl: u32, light_idx: u32, normal: vec3<f32>, pos: vec3<f32>) -> f32 {
    let cosatt = frame.light_cosatt[light_idx].xyz;
    let distatt = frame.light_distatt[light_idx].xyz;
    let light_pos = frame.light_pos[light_idx].xyz;
    let light_dir = frame.light_dir[light_idx].xyz;

    let attn_enable = extract_bits(ctrl, 9u, 1u) != 0u;
    let attn_select = extract_bits(ctrl, 10u, 1u) != 0u;
    let diff_fn = extract_bits(ctrl, 7u, 2u);

    var ldir = light_pos - pos;
    var attn: f32;

    if !attn_enable {
        ldir = normalize(ldir);
        attn = 1.0;
    } else if attn_select {
        let dist = length(ldir);
        ldir = normalize(ldir);
        let cos_angle = dot(light_dir, ldir);
        let angle_attn = max(cosatt.x + cosatt.y * cos_angle + cosatt.z * cos_angle * cos_angle, 0.0);
        attn = safe_div(angle_attn, distatt.x + distatt.y * dist + distatt.z * dist * dist);
    } else {
        let half_angle = normalize(light_pos);
        let norm_dot_half = dot(normal, half_angle);
        if norm_dot_half <= 0.0 {
            return 0.0;
        }
        let s = max(dot(normal, light_dir), 0.0);
        let att_len = vec3(1.0, s, s * s);
        attn = safe_div(max(dot(att_len, cosatt), 0.0), dot(att_len, distatt));
        ldir = half_angle;
    }

    let dif_attn = dot(ldir, normal);
    var diffuse: f32;
    switch diff_fn {
        case 0u: { diffuse = 1.0; }
        case 1u: { diffuse = dif_attn; }
        case 2u: { diffuse = max(dif_attn, 0.0); }
        default: { diffuse = 1.0; }
    }

    return attn * diffuse;
}

fn material_select(
    color_ctrl: u32, alpha_ctrl: u32,
    ambient: vec4<f32>, material: vec4<f32>,
    vertex_color: vec4<f32>,
) -> vec4<f32> {
    var mat_rgb: vec3<f32>;
    if extract_bits(color_ctrl, 0u, 1u) != 0u { mat_rgb = vertex_color.rgb; }
    else { mat_rgb = material.rgb; }
    var mat_a: f32;
    if extract_bits(alpha_ctrl, 0u, 1u) != 0u { mat_a = vertex_color.a; }
    else { mat_a = material.a; }
    return vec4(mat_rgb, mat_a);
}

fn compute_lighting_body(
    color_ctrl: u32, alpha_ctrl: u32,
    ambient: vec4<f32>, material: vec4<f32>,
    vertex_color: vec4<f32>, normal: vec3<f32>, pos: vec3<f32>,
) -> vec4<f32> {
    var mat_rgb: vec3<f32>;
    if extract_bits(color_ctrl, 0u, 1u) != 0u { mat_rgb = vertex_color.rgb; }
    else { mat_rgb = material.rgb; }

    var amb_rgb: vec3<f32>;
    if extract_bits(color_ctrl, 6u, 1u) != 0u { amb_rgb = vertex_color.rgb; }
    else { amb_rgb = ambient.rgb; }

    var mat_a: f32;
    if extract_bits(alpha_ctrl, 0u, 1u) != 0u { mat_a = vertex_color.a; }
    else { mat_a = material.a; }

    var amb_a: f32;
    if extract_bits(alpha_ctrl, 6u, 1u) != 0u { amb_a = vertex_color.a; }
    else { amb_a = ambient.a; }

    let color_lit_off = extract_bits(color_ctrl, 1u, 1u) == 0u;
    let alpha_lit_off = extract_bits(alpha_ctrl, 1u, 1u) == 0u;
    if color_lit_off && alpha_lit_off {
        return vec4(mat_rgb, mat_a);
    }

    var rgb_out = mat_rgb;
    if !color_lit_off {
        let mask_lo = extract_bits(color_ctrl, 2u, 4u);
        let mask_hi = extract_bits(color_ctrl, 11u, 4u);
        let light_mask = mask_lo | (mask_hi << 4u);

        var acc = amb_rgb;
        for (var i = 0u; i < 8u; i++) {
            if ((light_mask >> i) & 1u) == 0u {
                continue;
            }
            let lc = frame.light_colors[i];
            let factor = compute_light_factor(color_ctrl, i, normal, pos);
            acc += lc.rgb * factor;
        }
        rgb_out = mat_rgb * clamp(acc, vec3(0.0), vec3(1.0));
    }

    var a_out = mat_a;
    if !alpha_lit_off {
        let mask_lo = extract_bits(alpha_ctrl, 2u, 4u);
        let mask_hi = extract_bits(alpha_ctrl, 11u, 4u);
        let light_mask = mask_lo | (mask_hi << 4u);

        var acc_a = amb_a;
        for (var i = 0u; i < 8u; i++) {
            if ((light_mask >> i) & 1u) == 0u {
                continue;
            }
            let lc = frame.light_colors[i];
            let factor = compute_light_factor(alpha_ctrl, i, normal, pos);
            acc_a += lc.a * factor;
        }
        a_out = mat_a * clamp(acc_a, 0.0, 1.0);
    }

    return vec4(rgb_out, a_out);
}

@if(HAS_LIGHTING_C0)
fn compute_lighting_c0(
    color_ctrl: u32, alpha_ctrl: u32,
    ambient: vec4<f32>, material: vec4<f32>,
    vertex_color: vec4<f32>, normal: vec3<f32>, pos: vec3<f32>,
) -> vec4<f32> {
    return compute_lighting_body(color_ctrl, alpha_ctrl, ambient, material, vertex_color, normal, pos);
}
@else
fn compute_lighting_c0(
    color_ctrl: u32, alpha_ctrl: u32,
    ambient: vec4<f32>, material: vec4<f32>,
    vertex_color: vec4<f32>, normal: vec3<f32>, pos: vec3<f32>,
) -> vec4<f32> {
    return material_select(color_ctrl, alpha_ctrl, ambient, material, vertex_color);
}

@if(HAS_LIGHTING_C1)
fn compute_lighting_c1(
    color_ctrl: u32, alpha_ctrl: u32,
    ambient: vec4<f32>, material: vec4<f32>,
    vertex_color: vec4<f32>, normal: vec3<f32>, pos: vec3<f32>,
) -> vec4<f32> {
    return compute_lighting_body(color_ctrl, alpha_ctrl, ambient, material, vertex_color, normal, pos);
}
@else
fn compute_lighting_c1(
    color_ctrl: u32, alpha_ctrl: u32,
    ambient: vec4<f32>, material: vec4<f32>,
    vertex_color: vec4<f32>, normal: vec3<f32>, pos: vec3<f32>,
) -> vec4<f32> {
    return material_select(color_ctrl, alpha_ctrl, ambient, material, vertex_color);
}
