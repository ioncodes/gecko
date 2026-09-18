use crate::cli::Action;

use std::path::Path;
use std::{fs, process};

use image::ipl::{IPL_ROM_SIZE, is_encoded, scramble};

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

    scramble(&mut data);

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
