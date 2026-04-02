use disasm::gekko::GekkoInstruction;
use disasm::tokenizer::{self, AsmToken};
use gecko::flipper::dsp::Dsp;
use gecko::flipper::dsp::core::Registers as DspRegisters;
use gecko::gamecube::GameCube;

const DISASM_COL: usize = 22;
const COMMENT_COL: usize = 50;

/// Format a single trace line for the current PC of the emulator.
///
/// Returns a string like `80003100  4E800020  blr                           ; lr=80001234`
pub fn format_trace_line(emulator: &GameCube) -> String {
    let pc = emulator.cpu.pc;
    let raw = emulator.mmio.virt_read_u32(pc);

    if let Some((instr, _)) = GekkoInstruction::decode(emulator.mmio.virt_slice(pc, 4)) {
        let text = format!("{}", instr);
        let comment = reg_comment(&text, &emulator.cpu.gprs, &emulator.cpu.fprs);
        let pad = COMMENT_COL.saturating_sub(DISASM_COL + text.len());
        if comment.is_empty() {
            format!("{:08X}  {:08X}  {}", pc, raw, text)
        } else {
            format!("{:08X}  {:08X}  {}{}; {}", pc, raw, text, " ".repeat(pad), comment)
        }
    } else {
        format!("{:08X}  {:08X}  <unknown>", pc, raw)
    }
}

/// Generate register-value comments for a disassembled instruction line.
///
/// Parses the disassembly text for register references and returns a comma-separated
/// list of their current values, e.g. `r3=00000001, r4=80003000`.
pub fn reg_comment(disasm_text: &str, gprs: &[u32; 32], fprs: &[f64; 32]) -> String {
    let tokens = tokenizer::tokenize(disasm_text);
    let mut parts = Vec::new();
    let mut gpr_seen = [false; 32];
    let mut fpr_seen = [false; 32];
    for tok in &tokens {
        match tok {
            AsmToken::Gpr(n) => {
                let n = *n as usize;
                if !gpr_seen[n] {
                    gpr_seen[n] = true;
                    parts.push(format!("r{}={:08X}", n, gprs[n]));
                }
            }
            AsmToken::Fpr(n) => {
                let n = *n as usize;
                if !fpr_seen[n] {
                    fpr_seen[n] = true;
                    parts.push(format!("f{}={:.6e}", n, fprs[n]));
                }
            }
            _ => {}
        }
    }
    parts.join(", ")
}

/// Format a single DSP trace line for the current PC.
pub fn format_dsp_trace_line(dsp: &Dsp) -> String {
    let pc = dsp.registers.pc;
    let w0 = dsp.read_imem(pc);
    let w1 = dsp.read_imem(pc.wrapping_add(1));
    let buf = [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8];

    match disasm::dsp::GcDspInstruction::decode(&buf) {
        Some((instr, byte_len)) => {
            let text = instr.to_string();
            let comment = dsp_reg_comment(&text, &dsp.registers);
            let raw = if byte_len == 2 {
                format!("{:04X}     ", w0)
            } else {
                format!("{:04X} {:04X}", w0, w1)
            };
            if comment.is_empty() {
                format!("{:04X}  {}  {}", pc, raw, text)
            } else {
                format!("{:04X}  {}  {:<30}; {}", pc, raw, text, comment)
            }
        }
        None => format!("{:04X}  {:04X}       <unknown>", pc, w0),
    }
}

fn dsp_reg_comment(disasm_text: &str, regs: &DspRegisters) -> String {
    let tokens = tokenizer::tokenize(disasm_text);
    let mut parts = Vec::new();
    let mut seen = Vec::new();
    for tok in &tokens {
        if let AsmToken::Spr(name) = tok {
            if seen.contains(name) {
                continue;
            }
            seen.push(name);
            if let Some(val) = dsp_reg_value(name, regs) {
                parts.push(val);
            }
        }
    }
    parts.join(", ")
}

fn dsp_reg_value(name: &str, regs: &DspRegisters) -> Option<String> {
    Some(match name {
        "$ar0" => format!("ar0={:04X}", regs.ar[0]),
        "$ar1" => format!("ar1={:04X}", regs.ar[1]),
        "$ar2" => format!("ar2={:04X}", regs.ar[2]),
        "$ar3" => format!("ar3={:04X}", regs.ar[3]),
        "$ix0" => format!("ix0={:04X}", regs.ix[0]),
        "$ix1" => format!("ix1={:04X}", regs.ix[1]),
        "$ix2" => format!("ix2={:04X}", regs.ix[2]),
        "$ix3" => format!("ix3={:04X}", regs.ix[3]),
        "$wr0" => format!("wr0={:04X}", regs.wr[0]),
        "$wr1" => format!("wr1={:04X}", regs.wr[1]),
        "$wr2" => format!("wr2={:04X}", regs.wr[2]),
        "$wr3" => format!("wr3={:04X}", regs.wr[3]),
        "$st0" => format!("st0={:04X}", regs.call_stack.top()),
        "$st1" => format!("st1={:04X}", regs.data_stack.top()),
        "$st2" => format!("st2={:04X}", regs.loop_addr.top()),
        "$st3" => format!("st3={:04X}", regs.loop_counter.top()),
        "$ac0.h" => format!("ac0.h={:04X}", regs.ac0_high),
        "$ac1.h" => format!("ac1.h={:04X}", regs.ac1_high),
        "$ac0.m" => format!("ac0.m={:04X}", regs.ac0_mid),
        "$ac1.m" => format!("ac1.m={:04X}", regs.ac1_mid),
        "$ac0.l" => format!("ac0.l={:04X}", regs.ac0_low),
        "$ac1.l" => format!("ac1.l={:04X}", regs.ac1_low),
        "$ac0" => format!("ac0={:010X}", regs.ac(0) as u64 & 0xFF_FFFF_FFFF),
        "$ac1" => format!("ac1={:010X}", regs.ac(1) as u64 & 0xFF_FFFF_FFFF),
        "$ax0.l" => format!("ax0.l={:04X}", regs.ax[0]),
        "$ax1.l" => format!("ax1.l={:04X}", regs.ax[1]),
        "$ax0.h" => format!("ax0.h={:04X}", regs.axh[0]),
        "$ax1.h" => format!("ax1.h={:04X}", regs.axh[1]),
        "$ax0" => format!("ax0={:08X}", (regs.axh[0] as u32) << 16 | regs.ax[0] as u32),
        "$ax1" => format!("ax1={:08X}", (regs.axh[1] as u32) << 16 | regs.ax[1] as u32),
        "$cr" => format!("cr={:04X}", regs.config),
        "$sr" => format!("sr={:04X}", u16::from(regs.status)),
        "$prod.l" => format!("prod.l={:04X}", regs.product_low),
        "$prod.m1" => format!("prod.m1={:04X}", regs.product_mid1),
        "$prod.h" => format!("prod.h={:04X}", regs.product_high),
        "$prod.m2" => format!("prod.m2={:04X}", regs.product_mid2),
        _ => return None,
    })
}
