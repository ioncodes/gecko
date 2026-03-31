use clap::Parser;
use disasm::dsp::GcDspInstruction;
use gecko::gamecube::GameCube;

#[derive(Parser)]
#[command(about = "DSP microcode tracer")]
struct Args {
    /// Path to a DSP binary (raw IRAM image, loaded at 0x0000)
    #[arg(long)]
    bin: String,

    /// Path to a DSP ROM image (loaded into IROM at 0x8000)
    #[arg(long)]
    rom: Option<String>,

    /// Path to a DRAM image (loaded at data memory 0x0000)
    #[arg(long)]
    dram: Option<String>,

    /// Path to a coefficient table (loaded at data memory 0x1000)
    #[arg(long)]
    coef: Option<String>,

    /// Start address (default: 0x0000)
    #[arg(long, default_value = "0", value_parser = parse_hex)]
    start: u16,

    /// Stop after N instructions
    #[arg(long)]
    max_steps: Option<u64>,

    /// Stop when PC reaches this address
    #[arg(long, value_parser = parse_hex)]
    until: Option<u16>,

    /// Print instruction trace
    #[arg(long, default_value_t = true)]
    trace: bool,
}

fn parse_hex(s: &str) -> Result<u16, String> {
    u16::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("invalid hex: {e}"))
}

fn load_into(dst: &mut [u8], path: &str) {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    let len = data.len().min(dst.len());
    dst[..len].copy_from_slice(&data[..len]);
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let mut emu = GameCube::new(0);

    // Load DSP binary into IRAM
    load_into(&mut *emu.dsp.iram, &args.bin);

    // Load optional ROM/DRAM/COEF
    if let Some(rom) = &args.rom {
        load_into(&mut *emu.dsp.irom, rom);
    }
    if let Some(dram) = &args.dram {
        load_into(&mut *emu.dsp.dram, dram);
    }
    if let Some(coef) = &args.coef {
        load_into(&mut *emu.dsp.coef, coef);
    }

    // Set start address and enable DSP
    emu.dsp.registers.pc = args.start;
    emu.dsp.csr.set_reset(false);
    emu.dsp.csr.set_halt(false);

    let mut steps: u64 = 0;

    loop {
        if emu.dsp.csr.halt() {
            eprintln!("DSP halted at PC={:04X} after {steps} steps", emu.dsp.registers.pc);
            break;
        }

        if let Some(max) = args.max_steps {
            if steps >= max {
                eprintln!("reached max steps ({max}) at PC={:04X}", emu.dsp.registers.pc);
                break;
            }
        }

        if let Some(until) = args.until {
            if emu.dsp.registers.pc == until {
                eprintln!("reached target PC={until:04X} after {steps} steps");
                break;
            }
        }

        let pc = emu.dsp.registers.pc;

        if args.trace {
            let w0 = emu.dsp.read_imem(pc);
            let w1 = emu.dsp.read_imem(pc.wrapping_add(1));
            let bytes = [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8];
            if let Some((insn, _)) = GcDspInstruction::decode(&bytes) {
                println!("{pc:04X}  {insn}");
            } else {
                println!("{pc:04X}  .word {w0:#06x}");
            }
        }

        emu.tick_dsp();
        steps += 1;
    }
}
