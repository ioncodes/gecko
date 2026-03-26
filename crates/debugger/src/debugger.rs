use std::fs::File;
use std::io::{BufWriter, Write};

use disasm::gekko::GekkoInstruction;
use disasm::tokenizer::{self, AsmToken};
use gecko::gamecube::GameCube;

const TRACE_FILENAME: &str = "trace.log";

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum EmulatorState {
    Running,
    Paused,
    Step,
    RunUntilVsync,
    RunUntilAddress(u32),
}

pub struct DebuggerUi {
    pub emulator_state: EmulatorState,
    pub show_cpu: bool,
    pub show_gx_state: bool,
    pub show_mmio: bool,
    pub show_dvd: bool,
    pub show_exi: bool,
    pub show_irqs: bool,
    pub show_controls: bool,
    pub memory_base: u32,
    pub memory_addr_input: String,
    pub run_until_addr_input: String,
    pub dvd_cover_open: Option<bool>,
    pub trace_writer: Option<BufWriter<File>>,
}

impl Default for DebuggerUi {
    fn default() -> Self {
        DebuggerUi {
            emulator_state: EmulatorState::Paused,
            show_cpu: true,
            show_controls: true,
            show_gx_state: false,
            show_mmio: false,
            show_dvd: false,
            show_exi: false,
            show_irqs: false,
            memory_base: 0x8000_0000,
            memory_addr_input: "80000000".to_string(),
            run_until_addr_input: String::new(),
            dvd_cover_open: None,
            trace_writer: None,
        }
    }
}

impl DebuggerUi {
    pub fn is_tracing(&self) -> bool {
        self.trace_writer.is_some()
    }

    pub fn start_trace(&mut self) {
        let file = File::create(TRACE_FILENAME).expect("failed to create trace file");
        self.trace_writer = Some(BufWriter::new(file));
    }

    pub fn stop_trace(&mut self) {
        if let Some(mut w) = self.trace_writer.take() {
            let _ = w.flush();
        }
    }

    pub fn trace_step(&mut self, emulator: &GameCube) {
        if let Some(ref mut writer) = self.trace_writer {
            let pc = emulator.cpu.pc;
            let raw = emulator.mmio.virt_read_u32(pc);
            if let Some((instr, _)) = GekkoInstruction::decode(emulator.mmio.virt_slice(pc, 4)) {
                let text = format!("{}", instr);
                let comment = reg_comment(&text, &emulator.cpu.gprs, &emulator.cpu.fprs);
                const DISASM_COL: usize = 22;
                const COMMENT_COL: usize = 50;
                let pad = COMMENT_COL.saturating_sub(DISASM_COL + text.len());
                if comment.is_empty() {
                    let _ = writeln!(writer, "{:08X}  {:08X}  {}", pc, raw, text);
                } else {
                    let _ = writeln!(
                        writer,
                        "{:08X}  {:08X}  {}{}; {}",
                        pc,
                        raw,
                        text,
                        " ".repeat(pad),
                        comment
                    );
                }
            } else {
                let _ = writeln!(writer, "{:08X}  {:08X}  <unknown>", pc, raw);
            }
        }
    }
}

fn reg_comment(disasm_text: &str, gprs: &[u32; 32], fprs: &[f64; 32]) -> String {
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
