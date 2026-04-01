fn alpha_compare(a: f32, ref_val: f32, func: u32) -> bool {
    switch func {
        case 0u: { return false; }          // Never
        case 1u: { return a < ref_val; }    // Less
        case 2u: { return a == ref_val; }   // Equal
        case 3u: { return a <= ref_val; }   // LessEqual
        case 4u: { return a > ref_val; }    // Greater
        case 5u: { return a != ref_val; }   // NotEqual
        case 6u: { return a >= ref_val; }   // GreaterEqual
        case 7u: { return true; }           // Always
        default: { return true; }
    }
}

fn alpha_combine(a: bool, b: bool, op: u32) -> bool {
    switch op {
        case 0u: { return a && b; }     // AND
        case 1u: { return a || b; }     // OR
        case 2u: { return a != b; }     // XOR
        case 3u: { return a == b; }     // XNOR
        default: { return true; }
    }
}
