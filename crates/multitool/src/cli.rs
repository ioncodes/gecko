use clap::{Parser, Subcommand, ValueEnum};

pub fn parse_offset(s: &str) -> Result<usize, String> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        s.parse().map_err(|e: std::num::ParseIntError| e.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum DisasmArch {
    Ppc,
    Dsp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum IplAction {
    Decode,
    Encode,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show file header information
    Info {
        /// Input DOL file
        file: String,
    },
    /// Disassemble binary code
    Disasm {
        /// Target architecture
        #[arg(long, value_enum, default_value_t = DisasmArch::Ppc)]
        arch: DisasmArch,
        /// Start offset (hex or decimal)
        #[arg(long, value_parser = parse_offset)]
        offset: Option<usize>,
        /// Input file
        file: String,
    },
    /// Decode or encode a GameCube IPL
    Ipl {
        /// IPL transformation to apply
        #[arg(long, value_enum, default_value_t = IplAction::Decode)]
        action: IplAction,
        /// Input IPL file
        file: String,
        /// Output file path (defaults to <name>.encoded.bin or <name>.decoded.bin)
        output: Option<String>,
    },
    /// Dump GameCube/Wii disc image (ISO or RVZ) information
    Dvd {
        /// Input disc image (auto-detected: ISO, RVZ, or ZIP-wrapped)
        file: String,
        /// Extract files from the disc image to an "output" directory
        #[arg(long)]
        extract: bool,
    },
}

#[derive(Parser)]
#[command(about = "GameCube/Wii multi-tool", long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
