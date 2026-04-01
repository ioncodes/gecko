use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum AsmToken<'a> {
    Mnemonic(&'a str),
    Gpr(u8),
    Fpr(u8),
    CrField(u8),
    Spr(&'a str),
    ImmSigned(i32),
    ImmUnsigned(u32),
    ImmHex(i64),
    Displacement(i32),
    BranchTarget(&'a str),
    Punct(char),
    Text(&'a str),
}

impl fmt::Display for AsmToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AsmToken::Mnemonic(s) => f.write_str(s),
            AsmToken::Gpr(n) => write!(f, "r{n}"),
            AsmToken::Fpr(n) => write!(f, "f{n}"),
            AsmToken::CrField(n) => write!(f, "cr{n}"),
            AsmToken::Spr(s) => f.write_str(s),
            AsmToken::ImmSigned(v) => write!(f, "{v}"),
            AsmToken::ImmUnsigned(v) => write!(f, "{v}"),
            AsmToken::ImmHex(v) if *v < 0 => write!(f, "-0x{:X}", -v),
            AsmToken::ImmHex(v) => write!(f, "0x{v:X}"),
            AsmToken::Displacement(v) => write!(f, "{v}"),
            AsmToken::BranchTarget(s) => f.write_str(s),
            AsmToken::Punct(c) => write!(f, "{c}"),
            AsmToken::Text(s) => f.write_str(s),
        }
    }
}

static SPR_NAMES: &[&str] = &[
    "xer", "lr", "ctr", "dsisr", "dar", "dec", "sdr1", "srr0", "srr1", "sprg0", "sprg1", "sprg2", "sprg3", "ear",
    "pvr", "ibat0u", "ibat0l", "ibat1u", "ibat1l", "ibat2u", "ibat2l", "ibat3u", "ibat3l", "dbat0u", "dbat0l",
    "dbat1u", "dbat1l", "dbat2u", "dbat2l", "dbat3u", "dbat3l", "gqr0", "gqr1", "gqr2", "gqr3", "gqr4", "gqr5", "gqr6",
    "gqr7", "hid0", "hid1", "hid2", "wpar", "dmau", "dmal", "ummcr0", "upmc1", "upmc2", "usia", "ummcr1", "upmc3",
    "upmc4", "usda", "mmcr0", "pmc1", "pmc2", "sia", "mmcr1", "pmc3", "pmc4", "sda", "l2cr", "ictc", "thrm1", "thrm2",
    "thrm3", "iabr", "dabr", "cr",
];

static SPR_MNEMONICS: &[&str] = &["mfspr", "mtspr", "mftb", "mftbu"];

#[derive(Clone)]
struct Scanner<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + offset).copied()
    }

    fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.src.len());
    }

    fn take_while(&mut self, pred: impl Fn(&u8) -> bool) -> &'a str {
        let start = self.pos;
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() && pred(&bytes[self.pos]) {
            self.pos += 1;
        }
        &self.src[start..self.pos]
    }

    fn span(&mut self, f: impl FnOnce(&mut Self)) -> &'a str {
        let start = self.pos;
        f(self);
        &self.src[start..self.pos]
    }

    fn take_char(&mut self) -> &'a str {
        let start = self.pos;
        let rest = &self.src[start..];
        let ch_len = rest.chars().next().map_or(0, char::len_utf8);
        self.pos += ch_len;
        &self.src[start..self.pos]
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        self.src.as_bytes()[self.pos..].starts_with(prefix)
    }
}

pub fn tokenize(s: &str) -> Vec<AsmToken<'_>> {
    let mut sc = Scanner::new(s);
    let mut tokens = Vec::new();

    let mnemonic = sc.take_while(|b| !b.is_ascii_whitespace());
    tokens.push(AsmToken::Mnemonic(mnemonic));

    if sc.is_empty() {
        return tokens;
    }

    let expect_spr = SPR_MNEMONICS.contains(&mnemonic);
    tokenize_operands(&mut sc, &mut tokens, expect_spr);
    tokens
}

fn tokenize_operands<'a>(sc: &mut Scanner<'a>, tokens: &mut Vec<AsmToken<'a>>, mut expect_spr: bool) {
    while let Some(b) = sc.peek() {
        let ch = b as char;

        // ── Whitespace / punctuation ─────────────────────────────────
        if ch.is_ascii_whitespace() || matches!(ch, ',' | '(' | ')' | '@' | '#' | '.') {
            tokens.push(AsmToken::Punct(ch));
            sc.skip(1);
            continue;
        }

        // ── Hex immediate: 0x... ─────────────────────────────────────
        if sc.starts_with(b"0x") {
            let slice = sc.span(|s| {
                s.skip(2);
                s.take_while(u8::is_ascii_hexdigit);
            });
            if is_standalone_operand(tokens) {
                tokens.push(AsmToken::BranchTarget(slice));
            } else {
                let val = i64::from_str_radix(&slice[2..], 16).unwrap_or(0);
                tokens.push(AsmToken::ImmHex(val));
            }
            continue;
        }

        // ── Negative: -0x... or -decimal ─────────────────────────────
        if b == b'-' && sc.peek_at(1).is_some() {
            // -0x...
            if sc.starts_with(b"-0x") {
                sc.skip(3);
                let hex_digits = sc.take_while(u8::is_ascii_hexdigit);
                let val = i64::from_str_radix(hex_digits, 16).unwrap_or(0);
                tokens.push(AsmToken::ImmHex(-val));
                continue;
            }

            // -decimal
            if sc.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
                sc.skip(1); // skip '-'
                let digits = sc.take_while(u8::is_ascii_digit);
                let val: i32 = digits.parse().unwrap_or(0);
                if sc.peek() == Some(b'(') {
                    tokens.push(AsmToken::Displacement(-val));
                } else {
                    tokens.push(AsmToken::ImmSigned(-val));
                }
                continue;
            }
        }

        // ── Positive decimal or displacement ─────────────────────────
        if b.is_ascii_digit() {
            let digits = sc.take_while(u8::is_ascii_digit);
            let val: u64 = digits.parse().unwrap_or(0);
            if sc.peek() == Some(b'(') {
                tokens.push(AsmToken::Displacement(val as i32));
            } else {
                tokens.push(AsmToken::ImmUnsigned(val as u32));
            }
            continue;
        }

        // ── Relative branch target: "+8", "+12" ─────────────────────
        if b == b'+' && sc.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            let slice = sc.span(|s| {
                s.skip(1);
                s.take_while(|b| b.is_ascii_digit());
            });
            tokens.push(AsmToken::BranchTarget(slice));
            continue;
        }

        // ── DSP register: $name or $name.sub ────────────────────────
        if b == b'$' && sc.peek_at(1).is_some_and(|b| b.is_ascii_alphabetic()) {
            let word = sc.span(|s| {
                s.skip(1); // skip '$'
                s.take_while(|b| b.is_ascii_alphanumeric() || *b == b'.' || *b == b'_');
            });
            tokens.push(AsmToken::Spr(word));
            continue;
        }

        // ── Word token: identifier ───────────────────────────────────
        if b.is_ascii_alphabetic() || b == b'_' {
            let word = sc.take_while(|b| b.is_ascii_alphanumeric() || *b == b'_');
            let tok = classify_word(word, expect_spr);
            if expect_spr && matches!(tok, AsmToken::Spr(_)) {
                expect_spr = false;
            }
            tokens.push(tok);
            continue;
        }

        // ── Fallback: emit single char as Text ──────────────────────
        tokens.push(AsmToken::Text(sc.take_char()));
    }
}

fn classify_word<'a>(word: &'a str, expect_spr: bool) -> AsmToken<'a> {
    if let Some(n) = parse_register(word, "r", 31) {
        return AsmToken::Gpr(n);
    }
    if let Some(n) = parse_register(word, "f", 31) {
        return AsmToken::Fpr(n);
    }
    if let Some(n) = parse_register(word, "cr", 7) {
        return AsmToken::CrField(n);
    }
    if SPR_NAMES.contains(&word) {
        return AsmToken::Spr(word);
    }
    if expect_spr && word.parse::<u16>().is_ok() {
        return AsmToken::Spr(word);
    }
    AsmToken::Text(word)
}

fn parse_register(word: &str, prefix: &str, max: u8) -> Option<u8> {
    let rest = word.strip_prefix(prefix)?;
    let n: u8 = rest.parse().ok().filter(|_| rest.bytes().all(|b| b.is_ascii_digit()))?;
    (n <= max).then_some(n)
}

fn is_standalone_operand(tokens: &[AsmToken<'_>]) -> bool {
    tokens.len() == 2 && matches!(tokens[0], AsmToken::Mnemonic(_)) && matches!(tokens[1], AsmToken::Punct(' '))
}
