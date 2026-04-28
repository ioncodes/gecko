mod cli;
mod disassembly;
mod dol;
mod dvd;
mod ipl;

use crate::cli::{Args, Command, DisasmArch};
use crate::disassembly::{disassemble_dsp, disassemble_ppc};

use clap::Parser;
use std::{fs, process};

fn read_file_or_exit(file: &str) -> Vec<u8> {
    fs::read(file).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {}", file, e);
        process::exit(1);
    })
}

fn main() {
    let args = Args::parse();

    match &args.command {
        Command::Info { file } => {
            dol::info(read_file_or_exit(file));
        }
        Command::Disasm { arch, offset, file } => {
            let data = read_file_or_exit(file);

            let start = offset.unwrap_or(0);
            let min_size = match arch {
                DisasmArch::Dsp => 2,
                DisasmArch::Ppc => 4,
            };
            if data.len() < start + min_size {
                eprintln!("file too small for offset {:#x}", start);
                process::exit(1);
            }

            match arch {
                DisasmArch::Dsp => disassemble_dsp(&data, start),
                DisasmArch::Ppc => disassemble_ppc(&data, start),
            }
        }
        Command::Ipl { action, file, output } => {
            ipl::process(file, output.as_deref(), *action);
        }
        Command::Dvd { file, extract } => {
            if *extract {
                dvd::extract(read_file_or_exit(file));
            } else {
                dvd::info(read_file_or_exit(file));
            }
        }
    }
}
