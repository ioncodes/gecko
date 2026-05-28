use crate::cli::Action;

use std::{fs, process};

const SYSCONF_SIZE: usize = 0x4000;
const FOOTER_OFFSET: usize = 0x3FFC;
const MAGIC: &[u8; 4] = b"SCv0";
const FOOTER: &[u8; 4] = b"SCed";

const TYPE_BIGARRAY: u8 = 1;
const TYPE_SMALLARRAY: u8 = 2;
const TYPE_BYTE: u8 = 3;
const TYPE_SHORT: u8 = 4;
const TYPE_LONG: u8 = 5;
const TYPE_LONGLONG: u8 = 6;
const TYPE_BOOL: u8 = 7;

struct Entry {
    name: String,
    value: Value,
}

enum Value {
    Byte(u8),
    Short(u16),
    Long(u32),
    LongLong(u64),
    Bool(bool),
    BigArray(Vec<u8>),
    SmallArray(Vec<u8>),
}

impl Value {
    fn type_code(&self) -> u8 {
        match self {
            Value::BigArray(_) => TYPE_BIGARRAY,
            Value::SmallArray(_) => TYPE_SMALLARRAY,
            Value::Byte(_) => TYPE_BYTE,
            Value::Short(_) => TYPE_SHORT,
            Value::Long(_) => TYPE_LONG,
            Value::LongLong(_) => TYPE_LONGLONG,
            Value::Bool(_) => TYPE_BOOL,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Value::BigArray(_) => "bigarray",
            Value::SmallArray(_) => "smallarray",
            Value::Byte(_) => "byte",
            Value::Short(_) => "short",
            Value::Long(_) => "long",
            Value::LongLong(_) => "longlong",
            Value::Bool(_) => "bool",
        }
    }

    /// Number of bytes this value occupies in the binary blob, including any
    /// length prefix.
    fn encoded_len(&self) -> usize {
        match self {
            Value::Byte(_) | Value::Bool(_) => 1,
            Value::Short(_) => 2,
            Value::Long(_) => 4,
            Value::LongLong(_) => 8,
            Value::BigArray(d) => 2 + d.len(),
            Value::SmallArray(d) => 1 + d.len(),
        }
    }

    fn write_into(&self, dst: &mut [u8]) -> usize {
        match self {
            Value::Byte(b) => {
                dst[0] = *b;
                1
            }
            Value::Bool(b) => {
                dst[0] = u8::from(*b);
                1
            }
            Value::Short(v) => {
                dst[0..2].copy_from_slice(&v.to_be_bytes());
                2
            }
            Value::Long(v) => {
                dst[0..4].copy_from_slice(&v.to_be_bytes());
                4
            }
            Value::LongLong(v) => {
                dst[0..8].copy_from_slice(&v.to_be_bytes());
                8
            }
            // Array length prefixes store length-minus-one, matching the SDK.
            Value::BigArray(data) => {
                dst[0..2].copy_from_slice(&((data.len() - 1) as u16).to_be_bytes());
                dst[2..2 + data.len()].copy_from_slice(data);
                2 + data.len()
            }
            Value::SmallArray(data) => {
                dst[0] = (data.len() - 1) as u8;
                dst[1..1 + data.len()].copy_from_slice(data);
                1 + data.len()
            }
        }
    }

    fn format(&self) -> String {
        match self {
            Value::Byte(b) => b.to_string(),
            Value::Short(v) => v.to_string(),
            Value::Long(v) => v.to_string(),
            Value::LongLong(v) => v.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::BigArray(d) | Value::SmallArray(d) => d.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }
}

fn parse_binary(data: &[u8]) -> Result<Vec<Entry>, String> {
    if data.len() < 6 {
        return Err("file too small to be a SYSCONF".into());
    }
    if &data[0..4] != MAGIC {
        eprintln!("warning: missing 'SCv0' magic (found {:02x?})", &data[0..4]);
    }
    if data.len() != SYSCONF_SIZE {
        eprintln!("warning: expected {SYSCONF_SIZE:#x} bytes, got {:#x}", data.len());
    }

    let count = u16::from_be_bytes([data[4], data[5]]) as usize;
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let off_pos = 6 + i * 2;
        let raw = data.get(off_pos..off_pos + 2).ok_or("offset table is truncated")?;
        let off = u16::from_be_bytes([raw[0], raw[1]]) as usize;
        entries.push(self::parse_entry(data, off)?);
    }
    Ok(entries)
}

fn parse_entry(data: &[u8], off: usize) -> Result<Entry, String> {
    let meta = *data.get(off).ok_or("entry offset out of range")?;
    let type_code = meta >> 5;
    let name_len = (meta & 0x1F) as usize + 1;

    let name_end = off + 1 + name_len;
    let name = data.get(off + 1..name_end).ok_or("entry name out of range")?;
    let name = String::from_utf8_lossy(name).into_owned();

    let value = self::parse_value(data, name_end, type_code)?;
    Ok(Entry { name, value })
}

fn parse_value(data: &[u8], pos: usize, type_code: u8) -> Result<Value, String> {
    let take = |start: usize, n: usize| -> Result<&[u8], String> {
        data.get(start..start + n)
            .ok_or_else(|| format!("value data out of range at {start:#x}"))
    };

    Ok(match type_code {
        TYPE_BYTE => Value::Byte(take(pos, 1)?[0]),
        TYPE_BOOL => Value::Bool(take(pos, 1)?[0] != 0),
        TYPE_SHORT => Value::Short(u16::from_be_bytes(take(pos, 2)?.try_into().unwrap())),
        TYPE_LONG => Value::Long(u32::from_be_bytes(take(pos, 4)?.try_into().unwrap())),
        TYPE_LONGLONG => Value::LongLong(u64::from_be_bytes(take(pos, 8)?.try_into().unwrap())),
        TYPE_BIGARRAY => {
            let len = u16::from_be_bytes(take(pos, 2)?.try_into().unwrap()) as usize + 1;
            Value::BigArray(take(pos + 2, len)?.to_vec())
        }
        TYPE_SMALLARRAY => {
            let len = take(pos, 1)?[0] as usize + 1;
            Value::SmallArray(take(pos + 1, len)?.to_vec())
        }
        other => return Err(format!("unknown entry type code {other}")),
    })
}

fn to_text(entries: &[Entry]) -> String {
    let name_width = entries.iter().map(|e| e.name.len()).max().unwrap_or(4).max(4);

    let mut out = String::new();
    out.push_str(&format!("# Wii SYSCONF ({} entries)\n", entries.len()));
    out.push_str("# format: <name> <type> <value>\n");
    out.push_str("# ints accept decimal or 0x-hex, bool is true/false, arrays are hex bytes\n");
    for entry in entries {
        out.push_str(&format!(
            "{:<name_width$}  {:<10}  {}\n",
            entry.name,
            entry.value.type_name(),
            entry.value.format(),
        ));
    }
    out
}

fn parse_text(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let lineno = idx + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        let mut tokens = line.split_whitespace();
        let name = tokens.next().ok_or_else(|| format!("line {lineno}: missing name"))?;
        let ty = tokens.next().ok_or_else(|| format!("line {lineno}: missing type"))?;
        let val = tokens.next().ok_or_else(|| format!("line {lineno}: missing value"))?;

        if name.is_empty() || name.len() > 32 {
            return Err(format!("line {lineno}: name must be 1..=32 bytes"));
        }

        let value = self::parse_value_text(ty, val).map_err(|e| format!("line {lineno}: {e}"))?;
        entries.push(Entry {
            name: name.to_string(),
            value,
        });
    }
    Ok(entries)
}

fn parse_value_text(ty: &str, val: &str) -> Result<Value, String> {
    match ty.to_ascii_lowercase().as_str() {
        "byte" => Ok(Value::Byte(self::parse_uint(val, u8::MAX as u64)? as u8)),
        "short" => Ok(Value::Short(self::parse_uint(val, u16::MAX as u64)? as u16)),
        "long" => Ok(Value::Long(self::parse_uint(val, u32::MAX as u64)? as u32)),
        "longlong" => Ok(Value::LongLong(self::parse_uint(val, u64::MAX)?)),
        "bool" => match val.to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(Value::Bool(true)),
            "false" | "0" => Ok(Value::Bool(false)),
            _ => Err(format!("invalid bool '{val}'")),
        },
        "bigarray" => Ok(Value::BigArray(self::parse_hex(val)?)),
        "smallarray" => Ok(Value::SmallArray(self::parse_hex(val)?)),
        other => Err(format!("unknown type '{other}'")),
    }
}

fn parse_uint(s: &str, max: u64) -> Result<u64, String> {
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        s.parse::<u64>()
    }
    .map_err(|e| format!("invalid integer '{s}': {e}"))?;

    if v > max {
        return Err(format!("value {s} exceeds the type maximum {max}"));
    }
    Ok(v)
}

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    if s.is_empty() {
        return Err("empty array".into());
    }
    if !s.len().is_multiple_of(2) {
        return Err(format!("hex string has an odd length ({})", s.len()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("invalid hex: {e}")))
        .collect()
}

fn to_binary(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut blob = vec![0u8; SYSCONF_SIZE];
    blob[0..4].copy_from_slice(MAGIC);
    blob[4..6].copy_from_slice(&(entries.len() as u16).to_be_bytes());

    let dir_off = 6;
    let mut entry_off = dir_off + entries.len() * 2 + 2;

    for (i, entry) in entries.iter().enumerate() {
        let name = entry.name.as_bytes();
        let entry_size = 1 + name.len() + entry.value.encoded_len();
        if entry_off + entry_size > FOOTER_OFFSET {
            return Err(format!("entries exceed SYSCONF capacity at '{}'", entry.name));
        }

        let dir_pos = dir_off + i * 2;
        blob[dir_pos..dir_pos + 2].copy_from_slice(&(entry_off as u16).to_be_bytes());

        blob[entry_off] = (entry.value.type_code() << 5) | ((name.len() - 1) as u8 & 0x1F);
        entry_off += 1;
        blob[entry_off..entry_off + name.len()].copy_from_slice(name);
        entry_off += name.len();
        entry_off += entry.value.write_into(&mut blob[entry_off..]);
    }

    let past_last = dir_off + entries.len() * 2;
    blob[past_last..past_last + 2].copy_from_slice(&(entry_off as u16).to_be_bytes());

    blob[FOOTER_OFFSET..FOOTER_OFFSET + 4].copy_from_slice(FOOTER);
    Ok(blob)
}

fn default_output(name: &str, action: Action) -> String {
    match action {
        Action::Decode => format!("{name}.txt"),
        Action::Encode => name
            .strip_suffix(".txt")
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
    let entries = self::parse_binary(data)?;
    let text = self::to_text(&entries);

    println!("Decoding: {file}");
    print!("{text}");

    fs::write(out_path, text.as_bytes()).map_err(|e| format!("failed to write {out_path}: {e}"))?;
    println!("  output (text): {out_path}");
    Ok(())
}

fn encode(file: &str, data: &[u8], out_path: &str) -> Result<(), String> {
    let text = std::str::from_utf8(data).map_err(|_| "input is not valid UTF-8 text".to_string())?;
    let entries = self::parse_text(text)?;
    let blob = self::to_binary(&entries)?;

    println!("Encoding: {file} ({} entries)", entries.len());

    fs::write(out_path, &blob).map_err(|e| format!("failed to write {out_path}: {e}"))?;
    println!("  output (binary): {out_path}");
    Ok(())
}
