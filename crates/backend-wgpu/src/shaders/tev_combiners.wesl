// TEV color combiner.
//
// In regular mode (bias != 3): result = (d (+/-) lerp(a, b, c) + bias) * scale
// In compare mode (bias == 3): `scale` and `sub` are repurposed:
//   `scale` selects the TevCompareMode (0=R8, 1=GR16, 2=BGR24, 3=RGB8)
//   `sub`   selects the operator (false=GT, true=EQ)
//   result = d + ((compare(a, b)) ? c : 0)  per the selected mode
// `do_clamp` always means: if true, clamp to [0,1]; if false, leave unclamped.
fn tev_combine_color(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>,
                     bias: u32, sub: bool, scale: u32, do_clamp: bool) -> vec3<f32> {
    var result: vec3<f32>;

    if bias == 3u {
        let ai = vec3<u32>(u32(round(clamp(a.r, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(a.g, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(a.b, 0.0, 1.0) * 255.0)));
        let bi = vec3<u32>(u32(round(clamp(b.r, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(b.g, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(b.b, 0.0, 1.0) * 255.0)));

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
        let lerp = a * (vec3(1.0) - c) + b * c;

        if sub {
            result = d - lerp;
        } else {
            result = d + lerp;
        }

        switch bias {
            case 1u: { result += vec3(0.5); }   // AddHalf
            case 2u: { result -= vec3(0.5); }   // SubHalf
            default: {}
        }

        switch scale {
            case 1u: { result *= 2.0; }     // Scale2
            case 2u: { result *= 4.0; }     // Scale4
            case 3u: { result *= 0.5; }     // Divide2
            default: {}
        }
    }

    if do_clamp {
        result = clamp(result, vec3(0.0), vec3(1.0));
    }

    return result;
}

// Scalar version for the alpha channel.
fn tev_combine_alpha(a: f32, b: f32, c: f32, d: f32,
                     a_rgb: vec3<f32>, b_rgb: vec3<f32>,
                     bias: u32, sub: bool, scale: u32, do_clamp: bool) -> f32 {
    var result: f32;

    if bias == 3u {
        let ai = vec3<u32>(u32(round(clamp(a_rgb.r, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(a_rgb.g, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(a_rgb.b, 0.0, 1.0) * 255.0)));
        let bi = vec3<u32>(u32(round(clamp(b_rgb.r, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(b_rgb.g, 0.0, 1.0) * 255.0)),
                           u32(round(clamp(b_rgb.b, 0.0, 1.0) * 255.0)));
        let aa = u32(round(clamp(a, 0.0, 1.0) * 255.0));
        let ba = u32(round(clamp(b, 0.0, 1.0) * 255.0));

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
        let lerp = a * (1.0 - c) + b * c;

        if sub {
            result = d - lerp;
        } else {
            result = d + lerp;
        }

        switch bias {
            case 1u: { result += 0.5; }
            case 2u: { result -= 0.5; }
            default: {}
        }

        switch scale {
            case 1u: { result *= 2.0; }
            case 2u: { result *= 4.0; }
            case 3u: { result *= 0.5; }
            default: {}
        }
    }

    if do_clamp {
        result = clamp(result, 0.0, 1.0);
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
