use crate::cli::Action;

use std::path::Path;
use std::{fs, process};

const IPL_ROM_SIZE: usize = 0x200000;
const SCRAMBLE_START: usize = 0x100;
const SCRAMBLE_SIZE: usize = 0x1AFE00;

fn decode(data: &mut [u8]) {
    let region = &mut data[SCRAMBLE_START..SCRAMBLE_START + SCRAMBLE_SIZE];

    let mut acc: u8 = 0;
    let mut nacc: u8 = 0;

    let mut t: u16 = 0x2953;
    let mut u: u16 = 0xD9C2;
    let mut v: u16 = 0x3FF1;

    let mut x: u8 = 1;

    let mut it = 0;
    while it < region.len() {
        let t0 = (t & 1) as u8;
        let t1 = ((t >> 1) & 1) as u8;
        let u0 = (u & 1) as u8;
        let u1 = ((u >> 1) & 1) as u8;
        let v0 = (v & 1) as u8;

        x ^= t1 ^ v0;
        x ^= u0 | u1;
        x ^= (t0 ^ u1 ^ v0) & (t0 ^ u0);

        if t0 == u0 {
            v >>= 1;
            if v0 != 0 {
                v ^= 0xB3D0;
            }
        }

        if t0 == 0 {
            u >>= 1;
            if u0 != 0 {
                u ^= 0xFB10;
            }
        }

        t >>= 1;
        if t0 != 0 {
            t ^= 0xA740;
        }

        nacc += 1;
        acc = acc.wrapping_shl(1) + x;
        if nacc == 8 {
            region[it] ^= acc;
            nacc = 0;
            it += 1;
        }
    }
}

fn is_encoded(data: &[u8]) -> bool {
    if data.len() < 0x104 {
        return true;
    }
    let w = u32::from_be_bytes(data[0x100..0x104].try_into().unwrap());
    // Decoded BS1 starts with `lis r4, 0x0011`
    w != 0x3C800011
}

pub fn process(file: &str, output: Option<&str>, action: Action) {
    let mut data = fs::read(file).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {}", file, e);
        process::exit(1);
    });
    assert_eq!(data.len(), IPL_ROM_SIZE);

    let encoded = is_encoded(&data);

    match action {
        Action::Encode if encoded => eprintln!("warning: ROM appears already encoded"),
        Action::Decode if !encoded => eprintln!("warning: ROM appears already decoded"),
        _ => {}
    }

    let copyright = String::from_utf8_lossy(&data[..0x56]);
    let action_label = match action {
        Action::Decode => "Decoding",
        Action::Encode => "Encoding",
    };
    println!("{action_label}: {file}");
    println!("  {copyright}");

    decode(&mut data);

    let out_path = match output {
        Some(p) => p.to_string(),
        None => {
            let p = Path::new(file);
            let stem = p.file_stem().unwrap_or_default().to_string_lossy();
            let ext = p
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let suffix = match action {
                Action::Encode => ".encoded",
                Action::Decode => ".decoded",
            };
            format!("{}{}{}", stem, suffix, ext)
        }
    };

    fs::write(&out_path, &data).unwrap_or_else(|e| {
        eprintln!("failed to write {}: {}", out_path, e);
        process::exit(1);
    });

    let state = if is_encoded(&data) { "encoded" } else { "decoded" };
    println!("  output ({state}): {out_path}");
}
