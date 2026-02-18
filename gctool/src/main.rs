use disasm::dsp::DspInstruction;
use disasm::gekko::GekkoInstruction;

use clap::{Parser, ValueEnum};
use std::fs;
use std::process;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Dol,
    Dsp,
}

fn parse_offset(s: &str) -> Result<usize, String> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        s.parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())
    }
}

#[derive(Parser)]
#[command(about = "GameCube multi-tool", long_about = None)]
struct Args {
    #[arg(short, long, value_enum, default_value_t = Mode::Dol)]
    mode: Mode,
    file: String,
    #[arg(value_parser = parse_offset, default_value_t = 0)]
    offset: usize,
}

fn disassemble_dol(data: &[u8], start: usize) {
    let mut offset = start;
    while offset + 4 <= data.len() {
        let word = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
        let addr = offset as u32;

        match GekkoInstruction::decode(&data[offset..]) {
            Some((instr, _)) => println!("{:08X}  {:08X}  {}", addr, word, instr),
            None => println!("{:08X}  {:08X}  .long {:#010x}", addr, word, word),
        }

        offset += 4;
    }
}

fn disassemble_dsp(data: &[u8], start: usize) {
    let mut offset = start;
    while offset + 2 <= data.len() {
        let word = u16::from_be_bytes(data[offset..offset + 2].try_into().unwrap());
        let addr = (offset / 2) as u32;

        match DspInstruction::decode(&data[offset..]) {
            Some((instr, bytes_consumed)) => {
                let hex_parts: Vec<_> = data[offset..offset + bytes_consumed]
                    .chunks_exact(2)
                    .map(|c| format!("{:04x}", u16::from_be_bytes(c.try_into().unwrap())))
                    .collect();
                println!("{:04x}  {:9}  {}", addr, hex_parts.join(" "), instr);
                offset += bytes_consumed;
            }
            None => {
                println!("{:04x}  {:04x}      .word  {:#06x}", addr, word, word);
                offset += 2;
            }
        }
    }
}

fn main() {
    let args = Args::parse();

    let data = fs::read(&args.file).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {}", args.file, e);
        process::exit(1);
    });

    let min_size = match args.mode {
        Mode::Dol => 4,
        Mode::Dsp => 2,
    };

    if data.len() < args.offset + min_size {
        eprintln!("file too small for offset {:#x}", args.offset);
        process::exit(1);
    }

    match args.mode {
        Mode::Dol => disassemble_dol(&data, args.offset),
        Mode::Dsp => disassemble_dsp(&data, args.offset),
    }
}
