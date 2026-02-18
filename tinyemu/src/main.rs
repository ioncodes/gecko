use colored::Colorize;
use disasm::tokenizer::{self, AsmToken};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("Usage: debugger <path_to_rom>");
    let mut gekko = gekko::gekko::Gekko::new(&path);
    loop {
        let addr = gekko.cpu.pc;
        let (instr, _) = gekko.execute_instruction().unwrap();
        println!(
            "{}: {}",
            format!("{:08X}", addr).bold(),
            colorize_instr(&instr)
        );
    }
}

fn colorize(tok: &AsmToken<'_>) -> String {
    match tok {
        AsmToken::Mnemonic(s) => s.bold().cyan().to_string(),
        AsmToken::Gpr(n) => format!("r{n}").yellow().to_string(),
        AsmToken::Fpr(n) => format!("f{n}").magenta().to_string(),
        AsmToken::CrField(n) => format!("cr{n}").green().to_string(),
        AsmToken::Spr(s) => s.green().bold().to_string(),
        AsmToken::ImmSigned(v) => format!("{v}").blue().to_string(),
        AsmToken::ImmUnsigned(v) => format!("{v}").blue().to_string(),
        AsmToken::ImmHex(v) if *v < 0 => format!("-0x{:X}", -v).blue().to_string(),
        AsmToken::ImmHex(v) => format!("0x{v:X}").blue().to_string(),
        AsmToken::Displacement(v) => format!("{v}").blue().to_string(),
        AsmToken::BranchTarget(s) => s.bright_red().to_string(),
        AsmToken::Punct(_) | AsmToken::Text(_) => tok.to_string(),
    }
}

fn colorize_instr(instr: &disasm::gekko::GekkoInstruction) -> String {
    let text = format!("{}", instr);
    let tokens = tokenizer::tokenize(&text);
    tokens
        .into_iter()
        .map(|t| colorize(&t))
        .collect::<Vec<_>>()
        .join("")
}
