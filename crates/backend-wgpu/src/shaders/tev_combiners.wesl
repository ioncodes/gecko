fn tev_wrap_rgb(value: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(vec3<i32>(round(value * 255.0)) & vec3<i32>(255)) / 255.0;
}

fn tev_wrap_alpha(value: f32) -> f32 {
    return f32(i32(round(value * 255.0)) & 255) / 255.0;
}

fn tev_regular(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>,
               bias: u32, sub: bool, scale: u32) -> vec4<f32> {
    let ai = vec4<i32>(round(a * 255.0));
    let bi = vec4<i32>(round(b * 255.0));
    let ci = vec4<i32>(round(c * 255.0));
    var di = vec4<i32>(round(d * 255.0));
    if bias == 1u { di += vec4<i32>(128); }
    if bias == 2u { di -= vec4<i32>(128); }

    let shift = select(scale, 0u, scale == 3u);
    let factor = ci + (ci >> vec4<u32>(7u));
    var lerp = ((ai << vec4<u32>(8u)) + (bi - ai) * factor) << vec4<u32>(shift);
    if scale != 3u { lerp += vec4<i32>(select(128, 127, sub)); }
    lerp >>= vec4<u32>(8u);
    var result = (di << vec4<u32>(shift)) + select(lerp, -lerp, sub);
    if scale == 3u { result >>= vec4<u32>(1u); }
    return vec4<f32>(result) / 255.0;
}

// TEV color combiner.
//
// In regular mode (bias != 3): result = (d (+/-) lerp(a, b, c) + bias) * scale
// In compare mode (bias == 3): `scale` and `sub` are repurposed:
//   `scale` selects the TevCompareMode (0=R8, 1=GR16, 2=BGR24, 3=RGB8)
//   `sub`   selects the operator (false=GT, true=EQ)
//   result = d + ((compare(a, b)) ? c : 0)  per the selected mode
// `do_clamp` selects [0,1] or the signed TEV range [-1024,1023] / 255.
fn tev_combine_color(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>,
                     bias: u32, sub: bool, scale: u32, do_clamp: bool) -> vec3<f32> {
    var result: vec3<f32>;

    if bias == 3u {
        let ai = vec3<u32>(round(a * 255.0));
        let bi = vec3<u32>(round(b * 255.0));

        // scale is the TevCompareMode selector.
        var mask: vec3<f32>;
        switch scale {
            case 0u: {
                // R8: scalar compare on the red channel, gate c.rgb.
                let hit = compare_u32(ai.r, bi.r, sub);
                mask = vec3<f32>(hit, hit, hit);
            }
            case 1u: {
                // GR16: r + g*256
                let pa = ai.r + ai.g * 256u;
                let pb = bi.r + bi.g * 256u;
                let hit = compare_u32(pa, pb, sub);
                mask = vec3<f32>(hit, hit, hit);
            }
            case 2u: {
                // BGR24: r + g*256 + b*65536
                let pa = ai.r + ai.g * 256u + ai.b * 65536u;
                let pb = bi.r + bi.g * 256u + bi.b * 65536u;
                let hit = compare_u32(pa, pb, sub);
                mask = vec3<f32>(hit, hit, hit);
            }
            default: {
                // RGB8: per-channel compare.
                mask = vec3<f32>(
                    compare_u32(ai.r, bi.r, sub),
                    compare_u32(ai.g, bi.g, sub),
                    compare_u32(ai.b, bi.b, sub),
                );
            }
        }

        result = d + c * mask;
    } else {
        result = tev_regular(vec4(a, 0.0), vec4(b, 0.0), vec4(c, 0.0), vec4(d, 0.0), bias, sub, scale).rgb;
    }

    if do_clamp {
        result = clamp(result, vec3(0.0), vec3(1.0));
    } else {
        result = clamp(result, vec3(-1024.0 / 255.0), vec3(1023.0 / 255.0));
    }

    return result;
}

// Scalar version for the alpha channel.
fn tev_combine_alpha(a: f32, b: f32, c: f32, d: f32,
                     a_rgb: vec3<f32>, b_rgb: vec3<f32>,
                     bias: u32, sub: bool, scale: u32, do_clamp: bool) -> f32 {
    var result: f32;

    if bias == 3u {
        let ai = vec3<u32>(round(a_rgb * 255.0));
        let bi = vec3<u32>(round(b_rgb * 255.0));
        let aa = u32(round(a * 255.0));
        let ba = u32(round(b * 255.0));

        var hit: f32;
        switch scale {
            case 0u: { hit = compare_u32(ai.r, bi.r, sub); }    // R8
            case 1u: {                                          // GR16
                let pa = ai.r + ai.g * 256u;
                let pb = bi.r + bi.g * 256u;
                hit = compare_u32(pa, pb, sub);
            }
            case 2u: {                                          // BGR24
                let pa = ai.r + ai.g * 256u + ai.b * 65536u;
                let pb = bi.r + bi.g * 256u + bi.b * 65536u;
                hit = compare_u32(pa, pb, sub);
            }
            default: { hit = compare_u32(aa, ba, sub); }        // A8
        }

        result = d + c * hit;
    } else {
        result = tev_regular(vec4(a), vec4(b), vec4(c), vec4(d), bias, sub, scale).x;
    }

    if do_clamp {
        result = clamp(result, 0.0, 1.0);
    } else {
        result = clamp(result, -1024.0 / 255.0, 1023.0 / 255.0);
    }

    return result;
}

fn compare_u32(a: u32, b: u32, eq: bool) -> f32 {
    if eq {
        if a == b { return 1.0; }
    } else {
        if a > b { return 1.0; }
    }

    return 0.0;
}
