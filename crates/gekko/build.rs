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
        .group("branch", ["bx", "bcx", "bclrx"])
        .group("alu", ["ori", "oris", "addx", "addi", "addis"])
        .group("rotate", ["rlwinmx"])
        .group("msr", ["mtmsr", "mfmsr", "rfi"])
        .group("spr", ["mtspr", "mfspr"])
        .group("store_load", ["stw", "stwu", "sth", "sthu", "lwz", "lwzu"])
        .group("compare", ["cmp", "cmpi"]);

    // Generate the instruction type with accessor methods
    builder
        .build_instr_type(out_dir.join("gekko_instr.rs").to_str().unwrap())
        .expect("failed to generate instruction type");

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
