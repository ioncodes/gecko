use crate::cli::Action;

use std::{fs, process};

const SETTING_SIZE: usize = 0x100;
const SEED: u32 = 0x73B5_DBFA;

fn decode_blob(data: &[u8]) -> String {
    let content_len = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);

    let mut key = SEED;
    let mut out = String::new();
    for &b in &data[..content_len] {
        let plain = b ^ (key as u8);
        key = key.rotate_left(1);
        if plain != b'\r' {
            out.push(plain as char);
        }
    }
    out
}

struct Writer {
    buffer: [u8; SETTING_SIZE],
    position: usize,
    key: u32,
    overflowed: bool,
}

impl Writer {
    fn new() -> Self {
        Self {
            buffer: [0u8; SETTING_SIZE],
            position: 0,
            key: SEED,
            overflowed: false,
        }
    }

    fn write_line(&mut self, line: &str) {
        loop {
            let old_position = self.position;
            let old_key = self.key;
            for b in line.bytes() {
                self.write_byte(b);
            }

            if !self.buffer[old_position..self.position].contains(&0) {
                return;
            }

            self.position = old_position;
            self.key = old_key;
            self.write_byte(b'\n');
        }
    }

    fn write_byte(&mut self, b: u8) {
        if self.position >= self.buffer.len() {
            self.overflowed = true;
            return;
        }

        self.buffer[self.position] = b ^ (self.key as u8);
        self.position += 1;
        self.key = self.key.rotate_left(1);
    }
}

fn encode_text(text: &str) -> Result<Vec<u8>, String> {
    let mut writer = Writer::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if !line.contains('=') {
            return Err(format!("line {}: expected KEY=VALUE", idx + 1));
        }

        writer.write_line(&format!("{line}\r\n"));
    }

    if writer.overflowed {
        return Err(format!("content exceeds the {SETTING_SIZE:#x}-byte setting.txt size"));
    }

    Ok(writer.buffer.to_vec())
}

fn default_output(name: &str, action: Action) -> String {
    match action {
        Action::Decode => format!("{name}.decoded"),
        Action::Encode => name
            .strip_suffix(".decoded")
            .map(str::to_string)
            .unwrap_or_else(|| format!("{name}.bin")),
    }
}

pub fn process(file: &str, output: Option<&str>, action: Action) {
    let data = crate::read_file_or_exit(file);
    let out_path = crate::resolve_output(file, output, |name| self::default_output(name, action));

    let result = match action {
        Action::Decode => self::decode(file, &data, &out_path),
        Action::Encode => self::encode(file, &data, &out_path),
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

fn decode(file: &str, data: &[u8], out_path: &str) -> Result<(), String> {
    if data.len() != SETTING_SIZE {
        eprintln!("warning: expected {SETTING_SIZE:#x} bytes, got {:#x}", data.len());
    }

    let text = self::decode_blob(data);
    println!("Decoding: {file}");
    print!("{text}");

    fs::write(out_path, text.as_bytes()).map_err(|e| format!("failed to write {out_path}: {e}"))?;
    println!("  output (text): {out_path}");
    Ok(())
}

fn encode(file: &str, data: &[u8], out_path: &str) -> Result<(), String> {
    let text = std::str::from_utf8(data).map_err(|_| "input is not valid UTF-8 text".to_string())?;
    let blob = self::encode_text(text)?;

    println!("Encoding: {file}");
    fs::write(out_path, &blob).map_err(|e| format!("failed to write {out_path}: {e}"))?;
    println!("  output (binary): {out_path}");
    Ok(())
}
