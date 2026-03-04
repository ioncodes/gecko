use colored::Colorize;
use disasm::tokenizer::{self, AsmToken};

pub fn colorize(tok: &AsmToken<'_>) -> String {
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

pub fn colorize_instr(instr: &disasm::gekko::GekkoInstruction) -> String {
    let text = format!("{}", instr);
    let tokens = tokenizer::tokenize(&text);
    tokens
        .into_iter()
        .map(|t| colorize(&t))
        .collect::<Vec<_>>()
        .join("")
}

pub fn gpr_refs(instr: &disasm::gekko::GekkoInstruction) -> Vec<u8> {
    let text = format!("{}", instr);
    let tokens = tokenizer::tokenize(&text);
    let mut seen = [false; 32];
    let mut refs = Vec::new();
    for tok in tokens {
        if let AsmToken::Gpr(n) = tok {
            let n = n as usize;
            if !seen[n] {
                seen[n] = true;
                refs.push(n as u8);
            }
        }
    }
    refs
}

pub fn fpr_refs(instr: &disasm::gekko::GekkoInstruction) -> Vec<u8> {
    let text = format!("{}", instr);
    let tokens = tokenizer::tokenize(&text);
    let mut seen = [false; 32];
    let mut refs = Vec::new();
    for tok in tokens {
        if let AsmToken::Fpr(n) = tok {
            let n = n as usize;
            if !seen[n] {
                seen[n] = true;
                refs.push(n as u8);
            }
        }
    }
    refs
}

pub fn reg_comment(gprs: &[u32; 32], gpr_refs: &[u8], fprs: &[f64; 32], fpr_refs: &[u8]) -> String {
    let mut parts: Vec<String> = gpr_refs
        .iter()
        .map(|&n| format!("r{}={:08X}", n, gprs[n as usize]))
        .collect();
    for &n in fpr_refs {
        parts.push(format!("f{}={:.6e}", n, fprs[n as usize]));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("; {}", parts.join(", ")).dimmed().to_string()
}

pub fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            len += 1;
        }
    }
    len
}
