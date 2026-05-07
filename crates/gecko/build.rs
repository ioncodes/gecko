fn main() {
    chipi_build::generate_bindings("gekko.bindings.chipi").expect("chipi codegen failed (gekko)");
    chipi_build::generate_bindings("dsp.bindings.chipi").expect("chipi codegen failed (dsp)");
    chipi_build::generate_bindings("wii_gekko.bindings.chipi").expect("chipi codegen failed (wii gekko)");
    chipi_build::generate_bindings("wii_dsp.bindings.chipi").expect("chipi codegen failed (wii dsp)");
    chipi_build::generate_bindings("gekko_jit.bindings.chipi").expect("chipi codegen failed (gekko jit)");
    chipi_build::generate_bindings("wii_gekko_jit.bindings.chipi").expect("chipi codegen failed (wii gekko jit)");
}
