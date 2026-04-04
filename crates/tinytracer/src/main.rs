mod dump;
mod fmt;
mod kitty;
mod snaptshot;

use clap::{Parser, ValueEnum};
use colored::Colorize;
use disasm::dsp::GcDspInstruction;
use disasm::gekko::GekkoInstruction;
use snaptshot::CpuSnapshot;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TraceMode {
    Cpu,
    Dsp,
    Both,
}

impl TraceMode {
    fn trace_cpu(self) -> bool {
        matches!(self, TraceMode::Cpu | TraceMode::Both)
    }
    fn trace_dsp(self) -> bool {
        matches!(self, TraceMode::Dsp | TraceMode::Both)
    }
}

#[derive(Parser)]
#[command(about = "GameCube emulator")]
struct Args {
    /// Path to the DOL file
    #[arg(long)]
    dol: Option<String>,

    /// Path to an IPL file
    #[arg(long)]
    ipl: Option<String>,

    /// Path to a GameCube ISO file
    #[arg(long)]
    iso: Option<String>,

    /// Print decoded instructions and register diffs after each step
    #[arg(long)]
    debug: bool,

    /// Which processors to trace: cpu, dsp, or both
    #[arg(long, default_value = "cpu")]
    trace: TraceMode,

    /// Stop emulation when PC reaches this address (hex, e.g. 0x80003A00)
    #[arg(long, value_parser = parse_hex_addr)]
    until: Option<u32>,

    /// Path to a companion ELF file for symbol names
    #[arg(long)]
    elf: Option<String>,

    /// Suppress all stdout output (tracing is unaffected)
    #[arg(long)]
    quiet: bool,

    /// Disable ANSI escape codes
    #[arg(long)]
    no_ansi: bool,

    /// Path to a DSP IROM binary
    #[arg(long)]
    dsp: Option<String>,

    /// Path to a Lua script for scripting hooks
    #[cfg(feature = "scripting")]
    #[arg(long)]
    script: Option<String>,
}

fn parse_hex_addr(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("invalid hex address: {e}"))
}

fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .without_time()
        .with_ansi(!args.no_ansi)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let mut emulator = if let Some(dol_path) = &args.dol {
        let dol_data = std::fs::read(dol_path).expect("failed to read DOL");
        let dol = image::Dol::parse(dol_data);
        gecko::gamecube::GameCube::with_image(&dol)
    } else if let Some(ipl_path) = &args.ipl {
        let ipl_data = std::fs::read(ipl_path).expect("failed to read IPL");
        gecko::gamecube::GameCube::with_ipl(&ipl_data)
    } else {
        panic!("Either --dol or --ipl must be provided");
    };

    if let Some(iso_path) = &args.iso {
        if args.ipl.is_none() {
            panic!("--iso requires --ipl");
        }
        let iso_data = std::fs::read(iso_path).expect("failed to read ISO");
        let dvd = image::dvd::Dvd::parse(iso_data);
        emulator.insert_dvd(dvd);
    }

    if let Some(dsp_path) = &args.dsp {
        let dsp_data = std::fs::read(dsp_path).expect("failed to read DSP IROM");
        emulator.dsp.load_irom(&dsp_data);
    }

    #[cfg(feature = "scripting")]
    if let Some(path) = &args.script {
        let host = scripting::LuaHost::from_file(path).expect("failed to load script");
        emulator.set_hook_host(Box::new(host));
    }

    let symbols = args.elf.as_ref().map(|path| {
        let elf_data = std::fs::read(path).expect("failed to read ELF file");
        image::elf::parse_elf_symbols(&elf_data).expect("failed to parse ELF symbols")
    });

    run_emulator(&mut emulator, &args, symbols.as_ref());

    if !args.quiet {
        dump::vi(&emulator.vi);
        dump::exi(&emulator.exi);
        println!("Render current XFB:");
    }

    let pixels = emulator.render_xfb();
    let video_format = emulator.vi.dcr.video_format();
    kitty::render_xfb(&pixels, video_format.columns(), video_format.lines());
}

fn run_emulator(emulator: &mut gecko::gamecube::GameCube, args: &Args, symbols: Option<&image::symbols::SymbolTable>) {
    let mut prev_snapshot = CpuSnapshot::from_cpu(&emulator.cpu);
    let mut prev_pc = emulator.cpu.pc;
    let mut prev_dsp_pc = emulator.dsp.registers.pc;
    let mut in_busyloop = false;
    let mut current_func: Option<String> = None;
    let trace_cpu = args.trace.trace_cpu();
    let trace_dsp = args.trace.trace_dsp();
    let is_both = args.trace == TraceMode::Both;

    loop {
        if !in_busyloop && !args.quiet && trace_cpu {
            if let Some(symbols) = symbols {
                if let Some(sym) = symbols.lookup_exact(emulator.cpu.pc) {
                    if sym.kind == image::symbols::SymbolKind::Func {
                        let name = &sym.name;
                        let changed = current_func.as_ref() != Some(name);
                        if changed {
                            println!("{}", format!("{name}:").green().bold());
                            current_func = Some(name.clone());
                        }
                    }
                }
            }
            print_cpu_instruction(emulator, &prev_snapshot, args.debug, is_both);
        }

        emulator.step();

        // CPU trace
        let curr_snapshot = CpuSnapshot::from_cpu(&emulator.cpu);
        let curr_pc = emulator.cpu.pc;

        if trace_cpu {
            if curr_pc == prev_pc {
                if !in_busyloop && !args.quiet {
                    println!("{}", "Busyloop detected!".bright_red().bold());
                }
                in_busyloop = true;
            } else {
                in_busyloop = false;
            }

            if args.debug && !in_busyloop && !args.quiet {
                dump::registers(&curr_snapshot, &prev_snapshot);
            }
        }

        // DSP trace
        if trace_dsp && !args.quiet {
            let dsp_pc = emulator.dsp.registers.pc;
            if dsp_pc != prev_dsp_pc {
                print_dsp_instruction(emulator, prev_dsp_pc, is_both);
            }
            prev_dsp_pc = dsp_pc;
        }

        prev_pc = curr_pc;
        prev_snapshot = curr_snapshot;

        if args.until.is_some_and(|addr| curr_pc == addr) {
            break;
        }
    }
}

fn print_cpu_instruction(
    emulator: &gecko::gamecube::GameCube,
    prev_snapshot: &CpuSnapshot,
    debug: bool,
    prefix_tag: bool,
) {
    let instr = GekkoInstruction::decode(emulator.mmio.virt_slice(emulator.cpu.pc, 4))
        .unwrap_or_else(|| {
            dump::registers(prev_snapshot, prev_snapshot);
            dump::memory(&emulator.mmio, emulator.cpu.read_gpr(1));
            panic!(
                "Failed to decode instruction at {:08X} => {:08X}",
                emulator.cpu.pc,
                emulator.mmio.virt_read_u32(emulator.cpu.pc)
            );
        })
        .0;

    if debug {
        dbg!(&instr);
    }

    let refs = fmt::gpr_refs(&instr);
    let fpr_refs = fmt::fpr_refs(&instr);
    let comment = fmt::reg_comment(&prev_snapshot.gprs, &refs, &prev_snapshot.fprs, &fpr_refs);
    let tag = if prefix_tag {
        format!("{} ", "[cpu]".bold())
    } else {
        String::new()
    };
    let prefix = format!(
        "{}{}: {}",
        tag,
        format!("{:08X}", emulator.cpu.pc).bold(),
        fmt::colorize_instr(&instr)
    );

    const COMMENT_COL: usize = 50;
    let pad = COMMENT_COL.saturating_sub(fmt::visible_len(&prefix));

    if comment.is_empty() {
        println!("{}", prefix);
    } else {
        println!("{}{}{}", prefix, " ".repeat(pad), comment);
    }
}

fn print_dsp_instruction(emulator: &gecko::gamecube::GameCube, pc: u16, prefix_tag: bool) {
    let w0 = emulator.dsp.read_imem(pc);
    let w1 = emulator.dsp.read_imem(pc.wrapping_add(1));
    let bytes = [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8];
    let tag = if prefix_tag {
        format!("{} ", "[dsp]".bold().bright_blue())
    } else {
        String::new()
    };
    if let Some((insn, _)) = GcDspInstruction::decode(&bytes) {
        let text = format!("{}", insn);
        let comment = fmt::dsp_reg_comment(&text, &emulator.dsp.registers);
        let prefix = format!(
            "{}{}: {}",
            tag,
            format!("{:04X}", pc).bold(),
            fmt::colorize_dsp_instr(&insn)
        );
        const COMMENT_COL: usize = 50;
        let pad = COMMENT_COL.saturating_sub(fmt::visible_len(&prefix));
        if comment.is_empty() {
            println!("{}", prefix);
        } else {
            println!("{}{}{}", prefix, " ".repeat(pad), comment);
        }
    } else {
        println!("{}{}: .word {w0:#06x}", tag, format!("{:04X}", pc).bold());
    }
}
