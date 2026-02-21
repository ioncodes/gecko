use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let spec = "../../rsrc/chipi-spec/gamecube/gekko.chipi";

    let builder = chipi::LutBuilder::new(spec)
        .handler_mod("crate::cpu::interpreter")
        .ctx_type("crate::gekko::Gekko")
        .instr_type("crate::cpu::semantics::Instruction")
        .group("branch", ["bx"])
        .group("alu", ["ori", "addi", "addis"]);

    // Always regenerate the LUT dispatch tables
    builder
        .build_lut(out_dir.join("gekko_lut.rs").to_str().unwrap())
        .expect("failed to generate Gekko LUT");

    // Generate interpreter stubs once
    let stubs = manifest_dir.join("src/cpu/interpreter.rs");
    if !stubs.exists() {
        builder
            .build_stubs(stubs.to_str().unwrap())
            .expect("failed to generate interpreter stubs");
    }

    println!("cargo:rerun-if-changed={spec}");
}
