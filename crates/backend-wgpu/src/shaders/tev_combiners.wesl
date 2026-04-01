// TEV color combiner: result = (d (+/-) ((1-c)*a + c*b) + bias) * scale
fn tev_combine_color(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>,
                     bias: u32, sub: bool, scale: u32, do_clamp: bool) -> vec3<f32> {
    let lerp = a * (vec3(1.0) - c) + b * c;

    var result: vec3<f32>;
    if sub {
        result = d - lerp;
    } else {
        result = d + lerp;
    }

    // Bias
    switch bias {
        case 1u: { result += vec3(0.5); }   // AddHalf
        case 2u: { result -= vec3(0.5); }   // SubHalf
        default: {}                         // Zero
    }

    // Scale
    switch scale {
        case 1u: { result *= 2.0; }     // Scale2
        case 2u: { result *= 4.0; }     // Scale4
        case 3u: { result *= 0.5; }     // Divide2
        default: {}                     // Scale1
    }

    if do_clamp {
        result = clamp(result, vec3(0.0), vec3(1.0));
    }

    return result;
}

// TEV alpha combiner: same formula as color but per-channel
fn tev_combine_alpha(a: f32, b: f32, c: f32, d: f32, bias: u32, sub: bool, scale: u32, do_clamp: bool) -> f32 {
    let lerp = a * (1.0 - c) + b * c;

    var result: f32;
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

    if do_clamp {
        result = clamp(result, 0.0, 1.0);
    }

    return result;
}
