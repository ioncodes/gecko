use image::{Executable, dvd::{DVD_APPLOADER_OFFSET, FstNode}};
use owo_colors::OwoColorize;
use termtree::Tree;

pub fn info(data: Vec<u8>) {
    let dvd = image::dvd::Dvd::parse(data);

    let game_name = String::from_utf8_lossy(&dvd.header.game_name);
    let game_name = game_name.trim_end_matches('\0');

    println!("  {:<24} {}", "Game Name:".bold(), game_name);
    println!(
        "  {:<24} {}",
        "Game Code:".bold(),
        String::from_utf8_lossy(&dvd.header.game_code)
    );
    println!(
        "  {:<24} {}",
        "Maker Code:".bold(),
        String::from_utf8_lossy(&dvd.header.maker_code)
    );
    println!(
        "  {:<24} {} / {}",
        "Disk / Version:".bold(),
        dvd.header.disk_id,
        dvd.header.version
    );
    println!("  {:<24} {:08X}", "Magic:".bold(), u32::from_be_bytes(dvd.header.magic));
    println!(
        "  {:<24} {}",
        "Apploader Date:".bold(),
        String::from_utf8_lossy(&dvd.apploader.timestamp)
    );
    println!(
        "  {:<24} {:08X}",
        "Apploader Entrypoint:".bold(),
        dvd.apploader.entrypoint.get()
    );
    println!(
        "  {:<24} {:08X}",
        "Main DOL Offset:".bold(),
        dvd.header.offset_main_executable.get()
    );
    println!(
        "  {:<24} {:08X} ({} bytes)",
        "FST Offset:".bold(),
        dvd.header.offset_filesystem.get(),
        dvd.header.filesystem_size.get()
    );

    println!();
    println!("{}", build_tree(&dvd.filesystem));
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
                tree.push(build_tree(child));
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
    let dvd = image::dvd::Dvd::parse(data);
    let output_path = std::path::Path::new("output");
    extract_filesystem(&dvd, &dvd.filesystem, output_path);
    extract_apploader(&dvd, output_path);
    extract_main_dol(&dvd, output_path);
    println!("Extracted ISO contents to {}", output_path.display());
}

fn extract_filesystem(dvd: &image::dvd::Dvd, node: &FstNode, path: &std::path::Path) {
    match node {
        FstNode::Directory { name, children } => {
            let dir_path = path.join(name);
            std::fs::create_dir_all(&dir_path).unwrap();
            for child in children {
                extract_filesystem(dvd, child, &dir_path);
            }
        }
        FstNode::File { name, offset, size } => {
            let file_path = path.join(name);
            let data = &dvd.data()[*offset as usize..(*offset + *size) as usize];
            std::fs::write(file_path, data).unwrap();
            println!("Extracted file: {}", name);
        }
    }
}

fn extract_apploader(dvd: &image::dvd::Dvd, path: &std::path::Path) {
    let apploader = &dvd.apploader;
    let data = &dvd.data()[DVD_APPLOADER_OFFSET..(DVD_APPLOADER_OFFSET + apploader.size.get() as usize)];
    std::fs::write(path.join("apploader.bin"), data).unwrap();
}

fn extract_main_dol(dvd: &image::dvd::Dvd, path: &std::path::Path) {
    let dol_offset = dvd.header.offset_main_executable.get() as usize;
    let main_dol = image::Dol::parse(dvd.data()[dol_offset..].to_vec());
    std::fs::write(path.join("main.dol"), &main_dol.data()[..main_dol.size()]).unwrap();
}