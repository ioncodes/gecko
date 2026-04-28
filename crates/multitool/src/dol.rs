use comfy_table::Table;
use comfy_table::presets::ASCII_MARKDOWN;
use image::dol::Dol;
use image::{Executable, Section};

fn section_table(sections: &[Section]) -> Table {
    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(vec!["idx", "offset", "vaddr", "size", "end"]);
    for (i, s) in sections.iter().enumerate() {
        table.add_row(vec![
            format!("{i}"),
            format!("0x{:08X}", s.offset),
            format!("0x{:08X}", s.vaddr),
            format!("0x{:08X}", s.size),
            format!("0x{:08X}", s.vaddr + s.size),
        ]);
    }
    table
}

pub fn info(data: Vec<u8>) {
    let dol = Dol::parse(data);

    println!("Text Sections ({}):", dol.text_sections().len());
    println!("{}\n", section_table(dol.text_sections()));

    println!("Data Sections ({}):", dol.data_sections().len());
    println!("{}\n", section_table(dol.data_sections()));

    let (bss_start, bss_size) = dol.bss();
    println!("Entry point: 0x{:08X}\n", dol.entry_point());
    println!(
        "BSS: 0x{:08X} - 0x{:08X} (size: 0x{:08X})",
        bss_start,
        bss_start + bss_size,
        bss_size
    );
}
