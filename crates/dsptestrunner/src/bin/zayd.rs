use disasm::dsp::GcDspInstruction;
use gecko::flipper::dsp::Dsp;
use gecko::flipper::dsp::core::Registers;
use gecko::flipper::dsp::regs::ControlStatus;
use gecko::gamecube::GameCube;
use std::path::{Path, PathBuf};

const TEST_PC: u16 = 62;
const HALT_OPCODE: u16 = 0x0021;
const MAX_STEPS: u64 = 100_000;

const DSP_TESTS: &[&str] = &[
    "sanity",
    "abs",
    "add",
    "addarn",
    "addax",
    "addaxl",
    "addi",
    "addis",
    "addp",
    "addpaxz",
    "addr",
    "andc",
    "andcf",
    "andf",
    "andi",
    "andr",
    "asl",
    "asr",
    "asrn",
    "asrnr",
    "asrnrx",
    "asr16",
    "clr15",
    "clr",
    "clrl",
    "clrp",
    "cmp",
    "cmpaxh",
    "cmpi",
    "cmpis",
    "dar",
    "dec",
    "decm",
    "iar",
    "if_cc",
    "inc",
    "incm",
    "lsl",
    "lsl16",
    "lsr",
    "lsrn",
    "lsrnr",
    "lsrnrx",
    "lsr16",
    "lri",
    "lris",
    "lr_sr",
    "lrr_sr",
    "lrrd_sr",
    "lrri_sr",
    "lrrn_sr",
    "lrs_sr",
    "srr_lr",
    "srrd_lr",
    "srri_lr",
    "srrn_lr",
    "srs_lr",
    "srsh_lr",
    "m0",
    "m2",
    "madd",
    "maddc",
    "maddx",
    "mov",
    "movax",
    "movnp",
    "movp",
    "movpz",
    "movr",
    "mrr",
    "msub",
    "msubc",
    "msubx",
    "mul",
    "mulac",
    "mulaxh",
    "mulc",
    "mulcac",
    "mulcmv",
    "mulcmvz",
    "mulmv",
    "mulmvz",
    "mulx",
    "mulxac",
    "mulxmv",
    "mulxmvz",
    "neg",
    "not",
    "orc",
    "ori",
    "orr",
    "sbclr",
    "sbset",
    "set15",
    "set16",
    "set40",
    "sub",
    "subarn",
    "subax",
    "subp",
    "subr",
    "tst",
    "tstaxh",
    "tstprod",
    "xorc",
    "xori",
    "xorr",
    "bloop",
    "bloopi",
    "call_cc",
    "jmp_cc",
    "ret_cc",
    "ext_nop",
    "ext_dr",
    "ext_ir",
    "ext_nr",
    "ext_mv",
    "ext_s",
    "ext_sn",
    "ext_l",
    "ext_ln",
    "ext_ls",
    "ext_sl",
    "ext_lsn",
    "ext_sln",
    "ext_lsm",
    "ext_slm",
    "ext_lsnm",
    "ext_slnm",
    "ext_ld",
    "ext_ldax",
    "ext_ldn",
    "ext_ldaxn",
    "ext_ldm",
    "ext_ldaxm",
    "ext_ldnm",
    "ext_ldaxnm",
];

const DEFAULT_TESTS_DIR: &str = "submodules/beanwii/source/test/dsp/tests";

#[derive(Clone, Copy, Default)]
struct TestState {
    reg: [u16; 31],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Diff {
    Unchanged,
    ChangedFromPrevious,
    ChangedFromGolden,
    ChangedFromBoth,
}

struct DiffRow {
    reg: [Diff; 31],
}

impl DiffRow {
    fn any_failure(&self) -> bool {
        self.reg
            .iter()
            .any(|d| matches!(d, Diff::ChangedFromGolden | Diff::ChangedFromBoth))
    }
}

struct TestCase {
    instructions: Vec<u16>,
    expected: TestState,
    initial: TestState,
}

fn slot_to_dsp_reg(slot: usize) -> u8 {
    if slot < 18 { slot as u8 } else { (slot + 1) as u8 }
}

fn parse_test_file(bytes: &[u8]) -> Result<Vec<TestCase>, String> {
    if bytes.len() < 2 {
        return Err("file too small for header".into());
    }

    let instr_len_bytes = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
    if instr_len_bytes % 2 != 0 {
        return Err(format!("odd instruction_length {instr_len_bytes}"));
    }

    let per_case = instr_len_bytes + 31 * 2 * 2;
    let mut cases = Vec::new();
    let mut off = 2usize;
    while off + per_case <= bytes.len() {
        let mut instructions = Vec::with_capacity(instr_len_bytes / 2);

        for i in 0..instr_len_bytes / 2 {
            let lo = bytes[off + i * 2];
            let hi = bytes[off + i * 2 + 1];
            instructions.push(u16::from_le_bytes([lo, hi]));
        }

        off += instr_len_bytes;

        let mut expected = TestState::default();
        for slot in 0..31 {
            expected.reg[slot] = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
            off += 2;
        }

        let mut initial = TestState::default();
        for slot in 0..31 {
            initial.reg[slot] = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
            off += 2;
        }

        cases.push(TestCase {
            instructions,
            expected,
            initial,
        });
    }

    Ok(cases)
}

fn reset_dsp(dsp: &mut Dsp) {
    dsp.iram.fill(0);
    dsp.registers = Registers::default();
    dsp.csr = ControlStatus::default();
    dsp.csr.set_reset(false);
    dsp.csr.set_halt(false);
}

fn apply_initial_state(dsp: &mut Dsp, state: &TestState) {
    for slot in 0..31 {
        let dsp_reg = slot_to_dsp_reg(slot);
        // ST2 (DSP reg 14) is read-only on hardware: beanwii's set_reg drops
        // writes here, so the harness must too. Pushing would leave a stale
        // value on our loop_addr stack that read_actual_state would then pop.
        if dsp_reg == 14 {
            continue;
        }

        let mut val = state.reg[slot];
        if dsp_reg == 19 {
            // SR bit 8 cannot be set.
            val &= !0x0100;
        }

        // ALLOW_SIGN_EXTENSION=false: SXM effect on AC_MID writes is ignored,
        // matching beanwii's harness semantics.
        dsp.registers.write::<false>(dsp_reg, val);
    }
}

fn read_actual_state(dsp: &mut Dsp) -> TestState {
    let mut state = TestState::default();

    for slot in 0..31 {
        let dsp_reg = slot_to_dsp_reg(slot);

        // Mirror beanwii's get_reg(14): always reads 0 regardless of stack.
        if dsp_reg == 14 {
            state.reg[slot] = 0;
            continue;
        }

        state.reg[slot] = dsp.registers.read::<false>(dsp_reg);
    }

    state
}

fn load_instructions(dsp: &mut Dsp, instructions: &[u16]) {
    for (i, &w) in instructions.iter().enumerate() {
        let byte_off = (TEST_PC as usize + i) * 2;
        dsp.iram[byte_off..byte_off + 2].copy_from_slice(&w.to_be_bytes());
    }

    let halt_off = (TEST_PC as usize + instructions.len()) * 2;
    dsp.iram[halt_off..halt_off + 2].copy_from_slice(&HALT_OPCODE.to_be_bytes());
}

fn is_prod_test(name: &str) -> bool {
    name.starts_with("madd")
        || name == "movnp"
        || name == "movp"
        || name == "movpz"
        || name.starts_with("msub")
        || name.starts_with("mul")
        || name == "tstprod"
}

fn diff_state(previous: &TestState, golden: &TestState, actual: &TestState, ignore_flags: bool) -> DiffRow {
    let mut row = DiffRow {
        reg: [Diff::Unchanged; 31],
    };

    for slot in 0..31 {
        if ignore_flags && slot == 18 {
            row.reg[slot] = if previous.reg[slot] != actual.reg[slot] {
                Diff::ChangedFromPrevious
            } else {
                Diff::Unchanged
            };
            continue;
        }

        let g = golden.reg[slot];
        let a = actual.reg[slot];
        let p = previous.reg[slot];

        row.reg[slot] = if g != a && g != p {
            Diff::ChangedFromBoth
        } else if g != p {
            Diff::ChangedFromPrevious
        } else if g != a {
            Diff::ChangedFromGolden
        } else {
            Diff::Unchanged
        };
    }
    row
}

fn ansi_for(diff: Diff) -> &'static str {
    match diff {
        Diff::Unchanged => "",
        Diff::ChangedFromPrevious => "\x1b[36m",
        Diff::ChangedFromGolden => "\x1b[31m",
        Diff::ChangedFromBoth => "\x1b[1;31m",
    }
}

fn colorize(value: u16, diff: Diff) -> String {
    let color = ansi_for(diff);

    if color.is_empty() {
        format!("{value:04x}")
    } else {
        format!("{color}{value:04x}\x1b[0m")
    }
}

fn pretty_print(state: &TestState, diff: &DiffRow) {
    let c = |slot: usize| colorize(state.reg[slot], diff.reg[slot]);

    println!("\tAR: {} {} {} {}", c(0), c(1), c(2), c(3));
    println!("\tIX: {} {} {} {}", c(4), c(5), c(6), c(7));
    println!("\tWR: {} {} {} {}", c(8), c(9), c(10), c(11));
    println!("\tST: {} {} {} {}", c(12), c(13), c(14), c(15));
    println!(
        "\tAC0: {}:{}:{}  AC1: {}:{}:{}",
        c(16),
        c(29),
        c(27),
        c(17),
        c(30),
        c(28)
    );
    println!("\tAX0: {}:{}  AX1: {}:{}", c(25), c(23), c(26), c(24));
    println!("\tPROD: {}:{}:{}:{}", c(21), c(22), c(20), c(19));

    let sr_value = state.reg[18];
    let flags = (sr_value & 0xff) as u8;
    let sr_colored = colorize(sr_value, diff.reg[18]);

    fn flag(flags: u8, mask: u8, on: &'static str, off: &'static str) -> &'static str {
        if flags & mask != 0 { on } else { off }
    }

    println!(
        "\tSR {} [{} {} {} {} {} {} {} {}]",
        sr_colored,
        flag(flags, 0x80, "OS", "--"),
        flag(flags, 0x40, "LZ", "--"),
        flag(flags, 0x20, "TB", "--"),
        flag(flags, 0x10, "S32", "---"),
        flag(flags, 0x08, "S", "-"),
        flag(flags, 0x04, "AZ", "--"),
        flag(flags, 0x02, "O", "-"),
        flag(flags, 0x01, "C", "-"),
    );
}

fn format_instructions_hex(instructions: &[u16]) -> String {
    instructions
        .iter()
        .map(|w| format!("{w:04x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn disassemble_instructions(instructions: &[u16]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;

    while i < instructions.len() {
        let w0 = instructions[i];
        let w1 = if i + 1 < instructions.len() {
            instructions[i + 1]
        } else {
            0
        };

        let bytes = [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8];
        let pc = TEST_PC as usize + i;

        match GcDspInstruction::decode(&bytes) {
            Some((insn, byte_len)) => {
                out.push(format!("\t{:04x}: {insn}", pc));
                i += (byte_len / 2).max(1);
            }
            None => {
                out.push(format!("\t{:04x}: .word {:#06x}", pc, w0));
                i += 1;
            }
        }
    }

    out
}

struct RunResult {
    passed: u32,
    failed: u32,
}

fn run_one_test(emu: &mut GameCube, test_name: &str, path: &Path) -> Result<RunResult, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let cases = parse_test_file(&bytes)?;
    let ignore_flags = is_prod_test(test_name);

    let mut passed = 0u32;
    let mut failed = 0u32;

    for (idx, case) in cases.iter().enumerate() {
        reset_dsp(&mut emu.dsp);
        apply_initial_state(&mut emu.dsp, &case.initial);
        load_instructions(&mut emu.dsp, &case.instructions);
        emu.dsp.registers.pc = TEST_PC;

        let mut steps = 0u64;
        let mut runaway = false;
        while !emu.dsp.csr.halt() {
            if !emu.step_dsp_instruction() {
                break;
            }
            steps += 1;
            if steps >= MAX_STEPS {
                runaway = true;
                break;
            }
        }

        let actual = read_actual_state(&mut emu.dsp);
        let diff = diff_state(&case.initial, &case.expected, &actual, ignore_flags);

        if runaway || diff.any_failure() {
            failed += 1;
            println!("===== DSP Test {test_name} Failed =====");
            println!("Test case: {idx}");

            if runaway {
                println!(
                    "(runaway: >= {MAX_STEPS} steps without halt, pc={:04x})",
                    emu.dsp.registers.pc
                );
            }

            println!("Instructions: {}", format_instructions_hex(&case.instructions));

            for line in disassemble_instructions(&case.instructions) {
                println!("{line}");
            }

            println!("Initial state:");
            pretty_print(&case.initial, &diff);
            println!("Expected:");
            pretty_print(&case.expected, &diff);
            println!("Actual:");
            pretty_print(&actual, &diff);
            println!();
        } else {
            passed += 1;
        }
    }

    Ok(RunResult { passed, failed })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    let second = args.next();

    let (tests_dir, single_test): (PathBuf, Option<String>) = match (first, second) {
        (None, _) => (PathBuf::from(DEFAULT_TESTS_DIR), None),
        (Some(a), None) => {
            let p = PathBuf::from(&a);
            if p.is_dir() {
                (p, None)
            } else {
                (PathBuf::from(DEFAULT_TESTS_DIR), Some(a))
            }
        }
        (Some(dir), Some(name)) => (PathBuf::from(dir), Some(name)),
    };

    let mut emu = GameCube::new(0);

    let tests: Vec<&str> = if let Some(ref name) = single_test {
        vec![name.as_str()]
    } else {
        DSP_TESTS.to_vec()
    };

    let mut total_pass = 0u32;
    let mut total_fail = 0u32;
    let mut files_with_failures = Vec::new();

    for test in tests {
        let path = tests_dir.join(format!("{test}.bin"));
        if !path.exists() {
            eprintln!("skip {test}: {} not found", path.display());
            continue;
        }

        match run_one_test(&mut emu, test, &path) {
            Ok(r) => {
                let status = if r.failed == 0 { "OK" } else { "FAIL" };
                println!("{test}: {} passed / {} failed [{status}]", r.passed, r.failed);
                total_pass += r.passed;
                total_fail += r.failed;
                if r.failed > 0 {
                    files_with_failures.push(test.to_string());
                }
            }
            Err(e) => {
                eprintln!("{test}: error: {e}");
                total_fail += 1;
                files_with_failures.push(test.to_string());
            }
        }
    }

    println!();
    println!("=====================================");
    println!("Total: {total_pass} passed / {total_fail} failed");
    
    if !files_with_failures.is_empty() {
        println!("Files with failures: {}", files_with_failures.join(", "));
    }
}
