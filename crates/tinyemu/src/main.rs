mod fmt;
mod snaptshot;

use base64::Engine as _;
use colored::Colorize;
use disasm::gekko::GekkoInstruction;
use snaptshot::CpuSnapshot;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("Usage: debugger <path_to_rom>").clone();
    let is_debug = args.iter().any(|arg| arg == "--debug");
    let until_addr: Option<u32> = args
        .iter()
        .position(|arg| arg == "--until")
        .and_then(|i| args.get(i + 1))
        .map(|s| {
            u32::from_str_radix(s.trim_start_matches("0x"), 16)
                .expect("--until: invalid hex address")
        });

    tracing_subscriber::fmt()
        .without_time()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let mut gekko = gekko::gekko::Gekko::new(&path);
    let mut prev_snapshot = CpuSnapshot::from_cpu(&gekko.cpu);
    let mut current_addr = gekko.cpu.pc;
    let mut is_busyloop = false;

    loop {
        if !is_busyloop {
            let instr = GekkoInstruction::decode(gekko.mmio.virt_slice(gekko.cpu.pc, 4))
                .unwrap_or_else(|| {
                    dump_registers(&prev_snapshot, &prev_snapshot);
                    dump_memory(&gekko.mmio, gekko.cpu.read_gpr(1));

                    panic!("Failed to decode instruction at {:08X}", gekko.cpu.pc)
                })
                .0;

            if is_debug {
                dbg!(&instr);
            }

            let refs = fmt::gpr_refs(&instr);
            let comment = fmt::reg_comment(&prev_snapshot.gprs, &refs);

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

        gekko.run_until_event();

        let curr_snapshot = CpuSnapshot::from_cpu(&gekko.cpu);

        if current_addr == gekko.cpu.pc {
            if !is_busyloop {
                println!("{}", "Busyloop detected!".bright_red().bold());
                is_busyloop = true;
            }
        } else {
            is_busyloop = false;
        }

        current_addr = gekko.cpu.pc;

        if let Some(until) = until_addr {
            if gekko.cpu.pc == until {
                break;
            }
        }

        if is_debug && !is_busyloop {
            dump_registers(&curr_snapshot, &prev_snapshot);
        }

        prev_snapshot = curr_snapshot;
    }

    dump_mmio(&gekko.vi);

    println!("Render current XFB:");
    let pixels = gekko.render_xfb();
    let video_format = gekko.vi.dcr.video_format();
    render_kitty(&pixels, video_format.columns(), video_format.lines());
}

fn dump_registers(curr: &CpuSnapshot, prev: &CpuSnapshot) {
    let fmt_reg = |label: &str, val: u32, prev_val: u32| -> String {
        let value = format!("{:08X}", val);
        if val != prev_val {
            format!("{} {} ", label.yellow().bold(), value.bright_red().bold())
        } else {
            format!("{} {} ", label.dimmed(), value.dimmed())
        }
    };

    for row in 0..8 {
        let line: String = (0..4)
            .map(|col| {
                let i = row * 4 + col;
                fmt_reg(&format!("r{:<2}", i), curr.gprs[i], prev.gprs[i])
            })
            .collect();
        println!("{}", line.trim_end());
    }

    println!(
        "{}",
        format!(
            "{}{}",
            fmt_reg("lr ", curr.lr, prev.lr),
            fmt_reg("ctr", curr.ctr, prev.ctr)
        )
        .trim_end()
    );

    let cr_fields = [
        ("cr0", curr.cr.cr0(), prev.cr.cr0()),
        ("cr1", curr.cr.cr1(), prev.cr.cr1()),
        ("cr2", curr.cr.cr2(), prev.cr.cr2()),
        ("cr3", curr.cr.cr3(), prev.cr.cr3()),
        ("cr4", curr.cr.cr4(), prev.cr.cr4()),
        ("cr5", curr.cr.cr5(), prev.cr.cr5()),
        ("cr6", curr.cr.cr6(), prev.cr.cr6()),
        ("cr7", curr.cr.cr7(), prev.cr.cr7()),
    ];

    let fmt_cr_field = |label: &str,
                        val: gekko::cpu::condition::ConditionField,
                        prev_val: gekko::cpu::condition::ConditionField| {
        let flags = format!(
            "{}{}{}{}",
            if val.lt() { "L" } else { "·" },
            if val.gt() { "G" } else { "·" },
            if val.eq() { "Z" } else { "·" },
            if val.so() { "O" } else { "·" },
        );
        let text = format!("{}[{}] ", label, flags);
        if val.raw() != prev_val.raw() {
            format!("{}", text.bright_red().bold())
        } else {
            format!("{}", text.dimmed())
        }
    };

    let cr_line: String = cr_fields
        .iter()
        .map(|(label, val, prev_val)| fmt_cr_field(label, *val, *prev_val))
        .collect();
    println!("{}", cr_line.trim_end());

    println!();
}

fn dump_memory(mmio: &gekko::mmio::Mmio, addr: u32) {
    let aligned_addr = addr & !0xF;
    let start = aligned_addr.wrapping_sub(0x40);
    let data = mmio.virt_slice(start, 0x80);

    for (i, line) in data.chunks(16).enumerate() {
        let line_addr = start.wrapping_add((i as u32) * 16);
        let hex = line
            .chunks(4)
            .map(|chunk| {
                let word = u32::from_be_bytes(chunk.try_into().unwrap());
                format!("{:08X}", word)
            })
            .collect::<Vec<_>>()
            .join(" ");

        println!("{} {}", format!("{:08X}:", line_addr).blue().bold(), hex);
    }
}

fn dump_mmio(vi: &gekko::vi::Vi) {
    println!("Display Configuration: {:?}", vi.dcr);
    println!("Bottom Field Base: {:08X?}", vi.bfbl);
    println!("Top Field Base: {:08X?}", vi.tfbl);
    println!("XFB Address: {:08X}", vi.xfb_addr());
}

/// Render pixels (packed 0x00RRGGBB u32s) to the terminal via the Kitty
/// graphics protocol
///
/// Protocol: APC escape  \x1b_G<key=value,...>;<base64-payload>\x1b\\
///   a=T     – transmit + display immediately
///   f=32    – 32-bit RGBA pixels
///   s=W,v=H – dimensions
///   m=1     – more chunks follow; m=0 – last (or only) chunk
#[rustfmt::skip]
fn render_kitty(pixels: &[u32], width: usize, height: usize) {
    use std::io::Write as _;

    // Convert packed RGB to RGBA (alpha = 0xFF).
    let mut rgba: Vec<u8> = Vec::with_capacity(width * height * 4);
    for &px in pixels {
        rgba.push(((px >> 16) & 0xFF) as u8); // R
        rgba.push(((px >>  8) & 0xFF) as u8); // G
        rgba.push(( px        & 0xFF) as u8); // B
        rgba.push(0xFF);                      // A
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(&rgba);

    // Kitty protocol requires chunks of at most 4096 base64 characters
    const CHUNK: usize = 4096;
    let chunks: Vec<&str> = encoded
        .as_bytes()
        .chunks(CHUNK)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for (idx, chunk) in chunks.iter().enumerate() {
        let more = if idx + 1 < chunks.len() { 1 } else { 0 };
        if idx == 0 {
            // First chunk: include image metadata
            write!(
                out,
                "\x1b_Ga=T,f=32,s={},v={},m={};{}\x1b\\",
                width, height, more, chunk
            )
            .unwrap();
        } else {
            write!(out, "\x1b_Gm={};{}\x1b\\", more, chunk).unwrap();
        }
    }

    writeln!(out).unwrap();
}
