mod cli;
mod disassembly;
mod dol;
mod dvd;
mod ipl;
mod setting;
mod sysconf;

use crate::cli::{Args, Command, DisasmArch};
use crate::disassembly::{disassemble_dsp, disassemble_ppc};

use clap::Parser;
use std::path::Path;
use std::{fs, process};

pub(crate) fn read_file_or_exit(file: &str) -> Vec<u8> {
    fs::read(file).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {}", file, e);
        process::exit(1);
    })
}

/// Resolve an output path: the explicit `output` if given, otherwise `default`
/// applied to the input's file name and placed alongside the input.
pub(crate) fn resolve_output(file: &str, output: Option<&str>, default: impl FnOnce(&str) -> String) -> String {
    if let Some(p) = output {
        return p.to_string();
    }

    let path = Path::new(file);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let out = default(&name);
    match path.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => dir.join(out).to_string_lossy().into_owned(),
        None => out,
    }
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
        Command::Sysconf { action, file, output } => {
            sysconf::process(file, output.as_deref(), *action);
        }
        Command::Setting { action, file, output } => {
            setting::process(file, output.as_deref(), *action);
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
