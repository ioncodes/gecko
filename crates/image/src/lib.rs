#[cfg(feature = "banner")]
pub mod banner;
pub mod dol;
pub mod dvd;
pub mod ipl;
pub mod iso;
#[cfg(feature = "rvz")]
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
#[cfg(feature = "rvz")]
pub use rvz::Rvz;

use dvd::{Apploader, Header};

/// Wii disc magic word at header offset 0x18.
pub const WII_MAGIC: [u8; 4] = [0x5D, 0x1C, 0x9E, 0xA3];
/// GameCube disc magic word at header offset 0x1C.
pub const GC_MAGIC: [u8; 4] = [0xC2, 0x33, 0x9F, 0x3D];

pub trait Dvd: Send {
    fn header(&self) -> &Header;
    fn apploader(&self) -> &Apploader;
    fn read_disc_into(&self, offset: usize, buf: &mut [u8]);
    fn data_partition_offset(&self) -> u64;

    fn read_raw_disc(&self, offset: usize, buf: &mut [u8]);

    fn tmd_body_offset(&self) -> usize {
        let partition_offset = self.data_partition_offset() as usize;

        // Partition header layout: ticket at +0, TMD descriptor at +0x2A4.
        // We only need the TMD offset field (>>2).
        let mut tmd_descriptor = [0u8; 8];
        self.read_raw_disc(partition_offset + dvd::PARTITION_TMD_SIZE_OFFSET, &mut tmd_descriptor);
        let tmd_offset_field = u32::from_be_bytes(tmd_descriptor[4..8].try_into().unwrap());
        partition_offset + ((tmd_offset_field as usize) << 2)
    }

    fn tmd_ios_title_id(&self) -> u64 {
        assert!(
            self.header().is_wii(),
            "TMD IOS title ID is only applicable to Wii discs"
        );

        let mut ios_title_id = [0u8; 8];
        self.read_raw_disc(self.tmd_body_offset() + dvd::TMD_IOS_TITLE_ID_OFFSET, &mut ios_title_id);
        u64::from_be_bytes(ios_title_id)
    }

    fn tmd_title_id(&self) -> u64 {
        assert!(self.header().is_wii(), "TMD title ID is only applicable to Wii discs");

        let mut title_id = [0u8; 8];
        self.read_raw_disc(self.tmd_body_offset() + dvd::TMD_TITLE_ID_OFFSET, &mut title_id);
        u64::from_be_bytes(title_id)
    }
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

    fn data_partition_offset(&self) -> u64 {
        (**self).data_partition_offset()
    }

    fn read_raw_disc(&self, offset: usize, buf: &mut [u8]) {
        (**self).read_raw_disc(offset, buf)
    }

    fn tmd_ios_title_id(&self) -> u64 {
        (**self).tmd_ios_title_id()
    }

    fn tmd_title_id(&self) -> u64 {
        (**self).tmd_title_id()
    }
}

pub fn load_dvd(data: Vec<u8>) -> Box<dyn Dvd> {
    let data = if data.starts_with(b"PK\x03\x04") {
        self::extract_from_zip(data)
    } else {
        data
    };

    if data.starts_with(b"RVZ\x01") {
        #[cfg(feature = "rvz")]
        {
            Box::new(Rvz::parse(data))
        }
        #[cfg(not(feature = "rvz"))]
        {
            panic!("RVZ support is not compiled in (enable the `rvz` feature)");
        }
    } else {
        Box::new(Iso::parse(data))
    }
}

pub fn is_disc_entry(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".iso") || name.ends_with(".rvz")
}

fn extract_from_zip(data: Vec<u8>) -> Vec<u8> {
    use std::io::Read;

    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).expect("failed to open ZIP archive");
    let index = (0..archive.len())
        .find(|&i| self::is_disc_entry(archive.by_index(i).unwrap().name()))
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
