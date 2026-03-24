use zerocopy::byteorder::big_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::{Dol, Executable, Section};

pub const DVD_HEADER_SIZE: usize = 0x440;
pub const DVD_HEADER_INFO_SIZE: usize = 0x2000;
pub const DVD_APPLOADER_SIZE: usize = 0x2000;

pub const DVD_HEADER_OFFSET: usize = 0x00000000;
pub const DVD_HEADER_INFO_OFFSET: usize = DVD_HEADER_OFFSET + DVD_HEADER_SIZE;
pub const DVD_APPLOADER_OFFSET: usize = DVD_HEADER_INFO_OFFSET + DVD_HEADER_INFO_SIZE;

#[repr(C)]
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Debug)]
pub struct Header {
    pub game_code: [u8; 4],
    pub maker_code: [u8; 2],
    pub disk_id: u8,
    pub version: u8,
    pub audio_streaming: u8,
    pub streaming_buffer_size: u8,
    _unused1: [u8; 0x12],
    pub magic: [u8; 4],
    pub game_name: [u8; 0x3E0],
    pub offset_debug_monitor: U32,
    pub vaddr_debug_monitor: U32,
    _unused2: [u8; 0x18],
    pub offset_main_executable: U32,
    pub offset_filesystem: U32,
    pub filesystem_size: U32,
    pub filesystem_max_size: U32,
    pub user_position: U32,
    pub user_size: U32,
    _unused3: [u8; 0x8],
}

#[repr(C)]
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Debug)]
pub struct HeaderInfo {
    pub debug_monitor_size: U32,
    pub simulated_memory_size: U32,
    pub argument_offset: U32,
    pub debug_flag: U32,
    pub track_location: U32,
    pub track_size: U32,
    pub country_code: U32,
    _unused0: [u8; DVD_HEADER_INFO_SIZE - 0x1C],
}

#[repr(C)]
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Debug)]
pub struct Apploader {
    pub timestamp: [u8; 10],
    _unused0: [u8; 6],
    pub entrypoint: U32,
    pub size: U32,
    pub trailer_size: U32,
    _unused1: [u8; 4],
    pub code: [u8; DVD_APPLOADER_SIZE - 0x20],
}

pub enum FstNode {
    File { name: String, offset: u32, size: u32 },
    Directory { name: String, children: Vec<FstNode> },
}

impl FstNode {
    pub fn parse(fst: &[u8]) -> Self {
        let total_entries = u32::from_be_bytes(fst[8..12].try_into().unwrap()) as usize;
        let string_table = &fst[total_entries * 12..];

        let mut index = 1;
        let children = Self::parse_dir(fst, string_table, &mut index, total_entries);

        Self::Directory {
            name: String::new(),
            children,
        }
    }

    fn parse_dir(fst: &[u8], strings: &[u8], index: &mut usize, end: usize) -> Vec<Self> {
        let mut entries = Vec::new();
        while *index < end {
            let base = *index * 12;
            let flags = fst[base];
            let name_off = u32::from_be_bytes([0, fst[base + 1], fst[base + 2], fst[base + 3]]) as usize;
            let name = Self::read_cstr(strings, name_off);
            let offset = u32::from_be_bytes(fst[base + 4..base + 8].try_into().unwrap());
            let length = u32::from_be_bytes(fst[base + 8..base + 12].try_into().unwrap());
            *index += 1;

            if flags == 0 {
                entries.push(Self::File {
                    name,
                    offset,
                    size: length,
                });
            } else {
                let next = length as usize;
                let children = Self::parse_dir(fst, strings, index, next);
                entries.push(Self::Directory { name, children });
            }
        }
        entries
    }

    fn read_cstr(table: &[u8], offset: usize) -> String {
        let slice = &table[offset..];
        let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        String::from_utf8_lossy(&slice[..len]).into_owned()
    }

    pub fn name(&self) -> &str {
        match self {
            Self::File { name, .. } => name,
            Self::Directory { name, .. } => name,
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }
}

pub struct Dvd {
    pub header: Header,
    pub header_info: HeaderInfo,
    pub apploader: Apploader,
    pub filesystem: FstNode,
    data: Vec<u8>,
    sections: Vec<Section>,
}

impl Dvd {
    pub fn parse(data: Vec<u8>) -> Self {
        let header = Header::read_from_bytes(&data[DVD_HEADER_OFFSET..DVD_HEADER_OFFSET + DVD_HEADER_SIZE]).unwrap();
        let header_info =
            HeaderInfo::read_from_bytes(&data[DVD_HEADER_INFO_OFFSET..DVD_HEADER_INFO_OFFSET + DVD_HEADER_INFO_SIZE])
                .unwrap();
        let apploader =
            Apploader::read_from_bytes(&data[DVD_APPLOADER_OFFSET..DVD_APPLOADER_OFFSET + DVD_APPLOADER_SIZE]).unwrap();

        let fst_start = header.offset_filesystem.get() as usize;
        let fst_end = fst_start + header.filesystem_size.get() as usize;
        let filesystem = FstNode::parse(&data[fst_start..fst_end]);

        let apploader_section = Section {
            offset: DVD_APPLOADER_OFFSET as u32,
            vaddr: 0x81200000,
            size: apploader.size.get(),
        };

        let main_dol = Dol::parse(data[header.offset_main_executable.get() as usize..].to_vec());
        let main_section = Section {
            offset: header.offset_main_executable.get(),
            vaddr: main_dol.entry_point(),
            size: main_dol.size() as u32,
        };

        Dvd {
            header,
            header_info,
            apploader,
            filesystem,
            data,
            sections: vec![apploader_section, main_section],
        }
    }
}

impl Executable for Dvd {
    fn entry_point(&self) -> u32 {
        self.apploader.entrypoint.get()
    }

    fn data(&self) -> &[u8] {
        &self.data
    }

    fn text_sections(&self) -> &[crate::Section] {
        &self.sections
    }

    fn data_sections(&self) -> &[crate::Section] {
        &[]
    }

    fn bss(&self) -> (u32, u32) {
        (0, 0)
    }
}
