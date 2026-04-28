use image::Dvd;
use image::dvd::{DVD_APPLOADER_OFFSET, FstNode};
use owo_colors::OwoColorize;
use termtree::Tree;

pub fn info(data: Vec<u8>) {
    let dvd = image::load_dvd(data);
    let header = dvd.header();
    let apploader = dvd.apploader();

    let game_name = String::from_utf8_lossy(&header.game_name);
    let game_name = game_name.trim_end_matches('\0');

    println!("  {:<24} {}", "Game Name:".bold(), game_name);
    println!(
        "  {:<24} {}",
        "Game Code:".bold(),
        String::from_utf8_lossy(&header.game_code)
    );
    println!(
        "  {:<24} {}",
        "Maker Code:".bold(),
        String::from_utf8_lossy(&header.maker_code)
    );
    println!(
        "  {:<24} {} / {}",
        "Disk / Version:".bold(),
        header.disk_id,
        header.version
    );
    println!("  {:<24} {:08X}", "Magic:".bold(), u32::from_be_bytes(header.magic));
    println!(
        "  {:<24} {}",
        "Apploader Date:".bold(),
        String::from_utf8_lossy(&apploader.timestamp)
    );
    println!(
        "  {:<24} {:08X}",
        "Apploader Entrypoint:".bold(),
        apploader.entrypoint.get()
    );
    println!(
        "  {:<24} {:08X}",
        "Main DOL Offset:".bold(),
        header.offset_main_executable.get()
    );
    println!(
        "  {:<24} {:08X} ({} bytes)",
        "FST Offset:".bold(),
        header.offset_filesystem.get(),
        header.filesystem_size.get()
    );

    let filesystem = self::read_filesystem(dvd.as_ref());
    println!();
    println!("{}", self::build_tree(&filesystem));
}

fn read_filesystem(dvd: &dyn Dvd) -> FstNode {
    let header = dvd.header();
    let fst_offset = header.offset_filesystem.get() as usize;
    let fst_size = header.filesystem_size.get() as usize;
    let mut buf = vec![0u8; fst_size];
    dvd.read_disc_into(fst_offset, &mut buf);
    FstNode::parse(&buf)
}

fn build_tree(node: &FstNode) -> Tree<String> {
    match node {
        FstNode::Directory { name, children } => {
            let label = if name.is_empty() {
                "/".blue().bold().to_string()
            } else {
                format!("{}", name.blue().bold())
            };
            let mut tree = Tree::new(label);
            for child in children {
                tree.push(self::build_tree(child));
            }
            tree
        }
        FstNode::File { name, size, .. } => {
            let label = format!("{} {}", name, format!("({size} bytes)").dimmed());
            Tree::new(label)
        }
    }
}

pub fn extract(data: Vec<u8>) {
    let dvd = image::load_dvd(data);
    let output_path = std::path::Path::new("output");
    let filesystem = self::read_filesystem(dvd.as_ref());
    self::extract_filesystem(dvd.as_ref(), &filesystem, output_path);
    self::extract_apploader(dvd.as_ref(), output_path);
    self::extract_main_dol(dvd.as_ref(), output_path);
    println!("Extracted disc contents to {}", output_path.display());
}

fn extract_filesystem(dvd: &dyn Dvd, node: &FstNode, path: &std::path::Path) {
    match node {
        FstNode::Directory { name, children } => {
            let dir_path = path.join(name);
            std::fs::create_dir_all(&dir_path).unwrap();
            for child in children {
                self::extract_filesystem(dvd, child, &dir_path);
            }
        }
        FstNode::File { name, offset, size } => {
            let file_path = path.join(name);
            let mut data = vec![0u8; *size as usize];
            dvd.read_disc_into(*offset as usize, &mut data);
            std::fs::write(file_path, &data).unwrap();
            println!("Extracted file: {}", name);
        }
    }
}

fn extract_apploader(dvd: &dyn Dvd, path: &std::path::Path) {
    let apploader_size = dvd.apploader().size.get() as usize;
    let mut data = vec![0u8; apploader_size];
    dvd.read_disc_into(DVD_APPLOADER_OFFSET, &mut data);
    std::fs::write(path.join("apploader.bin"), &data).unwrap();
}

fn extract_main_dol(dvd: &dyn Dvd, path: &std::path::Path) {
    let dol_offset = dvd.header().offset_main_executable.get() as usize;

    let mut header_bytes = vec![0u8; 0x100];
    dvd.read_disc_into(dol_offset, &mut header_bytes);
    let dol_size = image::dol::Dol::parse(header_bytes).size();

    let mut full = vec![0u8; dol_size];
    dvd.read_disc_into(dol_offset, &mut full);
    std::fs::write(path.join("main.dol"), &full).unwrap();
}
