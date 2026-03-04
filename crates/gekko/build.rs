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
        .group("branch", ["bx", "bcx", "bclrx", "bcctrx"])
        .group(
            "alu",
            [
                "ori",
                "oris",
                "addx",
                "addi",
                "addis",
                "xori",
                "xoris",
                "andi_dot",
                "andis_dot",
                "subfx",
                "negx",
                "addcx",
                "subfcx",
                "addex",
                "subfex",
                "addzex",
                "subfzex",
                "addmex",
                "subfmex",
                "mullwx",
                "divwux",
                "divwx",
                "mulhwux",
                "mulhwx",
                "subfic",
                "addic",
                "addic_dot",
                "mulli",
            ],
        )
        .group("rotate", ["rlwinmx", "rlwimix", "rlwnmx"])
        .group("msr", ["mtmsr", "mfmsr", "rfi"])
        .group("spr", ["mtspr", "mfspr"])
        .group("segment", ["mtsr", "mfsr"])
        .group(
            "store_load",
            [
                "stw", "stwu", "sth", "sthu", "lwz", "lwzu", "lbz", "lbzu", "stb", "stbu", "lhz", "lhzu", "lha",
                "lhau", "lmw", "stmw", "lwzx", "lbzx", "lhzx", "lhax", "stwx", "stbx", "sthx", "lwzux", "lbzux",
                "lhzux", "lhaux", "stwux", "stbux", "sthux",
            ],
        )
        .group(
            "store_load_fp",
            [
                "lfd", "lfdu", "stfd", "stfdu", "lfs", "lfsu", "stfs", "stfsu", "lfsx", "lfsux", "lfdx", "lfdux",
                "stfsx", "stfsux", "stfdx", "stfdux",
            ],
        )
        .group("compare", ["cmp", "cmpi", "cmpli", "cmpl"])
        .group(
            "nop",
            [
                "isync", "sync", "eieio", "dcbf", "dcbi", "dcbst", "dcbt", "dcbtst", "dcba", "dcbz", "dcbz_l", "icbi",
                "tlbie", "tlbia", "tlbsync", "tlbld", "tlbli",
            ],
        )
        .group(
            "logical",
            [
                "andx", "orx", "xorx", "nandx", "norx", "andcx", "orcx", "eqvx", "slwx", "srwx", "srawx", "srawix",
                "cntlzwx", "extshx", "extsbx",
            ],
        )
        .group(
            "cr_ops",
            [
                "mtcrf", "mfcr", "crxor", "cror", "crand", "creqv", "crnor", "crnand", "crandc", "crorc", "mcrf",
            ],
        )
        .group(
            "fp_ops",
            [
                "mtfsfx", "mffsx", "mtfsb0x", "mtfsb1x", "mtfsfix", "mcrfs", "fmrx", "fnegx", "fabsx", "fnabsx",
                "frspx", "fctiwx", "fctiwzx", "fcmpu", "fcmpo", "faddx", "fsubx", "fmulx", "fdivx", "fmaddx", "fmsubx",
                "fnmaddx", "fnmsubx", "faddsx", "fsubsx", "fmulsx", "fdivsx", "fmaddsx", "fmsubsx", "fnmaddsx",
                "fnmsubsx",
            ],
        );

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
