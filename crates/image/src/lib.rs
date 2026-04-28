pub mod dol;
pub mod dvd;
pub mod iso;
pub mod rvz;

#[cfg(feature = "symbols")]
pub mod elf;
#[cfg(feature = "symbols")]
pub mod idb;
#[cfg(feature = "symbols")]
pub mod loader;
pub mod symbols;

pub use dol::Dol;
pub use iso::Iso;
pub use rvz::Rvz;

use dvd::{Apploader, Header};

pub trait Dvd: Send {
    fn header(&self) -> &Header;
    fn apploader(&self) -> &Apploader;
    fn read_disc_into(&self, offset: usize, buf: &mut [u8]);
}

impl<T: Dvd + ?Sized> Dvd for Box<T> {
    fn header(&self) -> &Header {
        (**self).header()
    }

    fn apploader(&self) -> &Apploader {
        (**self).apploader()
    }

    fn read_disc_into(&self, offset: usize, buf: &mut [u8]) {
        (**self).read_disc_into(offset, buf)
    }
}

pub fn load_dvd(data: Vec<u8>) -> Box<dyn Dvd> {
    let data = if data.starts_with(b"PK\x03\x04") {
        self::extract_from_zip(data)
    } else {
        data
    };

    if data.starts_with(b"RVZ\x01") {
        Box::new(Rvz::parse(data))
    } else {
        Box::new(Iso::parse(data))
    }
}

fn extract_from_zip(data: Vec<u8>) -> Vec<u8> {
    use std::io::Read;

    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).expect("failed to open ZIP archive");
    let index = (0..archive.len())
        .find(|&i| {
            let name = archive.by_index(i).unwrap().name().to_ascii_lowercase();
            name.ends_with(".iso") || name.ends_with(".rvz")
        })
        .expect("no disc image found in ZIP");
    let mut entry = archive.by_index(index).unwrap();
    
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).expect("failed to read ZIP entry");
    buf
}

pub struct Section {
    pub offset: u32,
    pub vaddr: u32,
    pub size: u32,
}

pub trait Executable {
    fn text_sections(&self) -> &[Section];
    fn data_sections(&self) -> &[Section];
    fn bss(&self) -> (u32, u32);
    fn entry_point(&self) -> u32;
    fn data(&self) -> &[u8];
}
