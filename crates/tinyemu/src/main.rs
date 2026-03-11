mod dump;
mod fmt;
mod kitty;
mod snaptshot;

use clap::Parser;
use colored::Colorize;
use disasm::gekko::GekkoInstruction;
use snaptshot::CpuSnapshot;

#[derive(Parser)]
#[command(about = "Gekko CPU emulator / debugger")]
struct Args {
    /// Path to the ROM/DOL file
    rom: String,

    /// Print decoded instructions and register diffs after each step
    #[arg(long)]
    debug: bool,

    /// Stop emulation when PC reaches this address (hex, e.g. 0x80003A00)
    #[arg(long, value_parser = parse_hex_addr)]
    until: Option<u32>,

    /// Path to a companion ELF file for symbol names
    #[arg(long)]
    elf: Option<String>,

    /// Suppress all stdout output (tracing is unaffected)
    #[arg(long)]
    quiet: bool,
}

fn parse_hex_addr(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("invalid hex address: {e}"))
}

fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .without_time()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let rom_data = std::fs::read(&args.rom).expect("failed to read ROM");
    let dol = image::Dol::parse(rom_data);
    let mut gekko = gekko::gekko::Gekko::new(&dol);

    let symbols = args.elf.as_ref().map(|path| {
        let elf_data = std::fs::read(path).expect("failed to read ELF file");
        image::elf::parse_elf_symbols(&elf_data).expect("failed to parse ELF symbols")
    });

    run_emulator(&mut gekko, &args, symbols.as_ref());

    if !args.quiet {
        dump::vi(&gekko.vi);
        dump::exi(&gekko.exi);
        println!("Render current XFB:");
    }

    let pixels = gekko.render_xfb();
    let video_format = gekko.vi.dcr.video_format();
    kitty::render_xfb(&pixels, video_format.columns(), video_format.lines());
}

fn run_emulator(gekko: &mut gekko::gekko::Gekko, args: &Args, symbols: Option<&image::symbols::SymbolTable>) {
    let mut prev_snapshot = CpuSnapshot::from_cpu(&gekko.cpu);
    let mut prev_pc = gekko.cpu.pc;
    let mut in_busyloop = false;
    let mut current_func: Option<String> = None;

    loop {
        if !in_busyloop && !args.quiet {
            if let Some(symbols) = symbols {
                if let Some(sym) = symbols.lookup_exact(gekko.cpu.pc) {
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
            print_instruction(gekko, &prev_snapshot, args.debug);
        }

        gekko.step();

        let curr_snapshot = CpuSnapshot::from_cpu(&gekko.cpu);
        let curr_pc = gekko.cpu.pc;

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

        prev_pc = curr_pc;
        prev_snapshot = curr_snapshot;

        if args.until.is_some_and(|addr| curr_pc == addr) {
            break;
        }
    }
}

fn print_instruction(gekko: &gekko::gekko::Gekko, prev_snapshot: &CpuSnapshot, debug: bool) {
    let instr = GekkoInstruction::decode(gekko.mmio.virt_slice(gekko.cpu.pc, 4))
        .unwrap_or_else(|| {
            dump::registers(prev_snapshot, prev_snapshot);
            dump::memory(&gekko.mmio, gekko.cpu.read_gpr(1));
            panic!(
                "Failed to decode instruction at {:08X} => {:08X}",
                gekko.cpu.pc,
                gekko.mmio.virt_read_u32(gekko.cpu.pc)
            );
        })
        .0;

    if debug {
        dbg!(&instr);
    }

    let refs = fmt::gpr_refs(&instr);
    let fpr_refs = fmt::fpr_refs(&instr);
    let comment = fmt::reg_comment(&prev_snapshot.gprs, &refs, &prev_snapshot.fprs, &fpr_refs);
    let prefix = format!(
        "{}: {}",
        format!("{:08X}", gekko.cpu.pc).bold(),
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
