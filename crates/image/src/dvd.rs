use zerocopy::byteorder::big_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

pub const DVD_HEADER_SIZE: usize = 0x440;
pub const DVD_HEADER_INFO_SIZE: usize = 0x2000;
pub const DVD_APPLOADER_SIZE: usize = 0x2000;

pub const DVD_HEADER_OFFSET: usize = 0x00000000;
pub const DVD_HEADER_INFO_OFFSET: usize = DVD_HEADER_OFFSET + DVD_HEADER_SIZE;
pub const DVD_APPLOADER_OFFSET: usize = DVD_HEADER_INFO_OFFSET + DVD_HEADER_INFO_SIZE;

/// Offset of the TMD size field within a Wii partition header. The next four
/// bytes (at +0x2A8) are the TMD body offset >> 2.
pub const PARTITION_TMD_SIZE_OFFSET: usize = 0x2A4;

/// Offset of the `ios_title_id` field within the TMD body.
pub const TMD_IOS_TITLE_ID_OFFSET: usize = 0x184;

/// Offset of the `title_id` field within the TMD body.
pub const TMD_TITLE_ID_OFFSET: usize = 0x18C;

#[repr(C, packed)]
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Header {
    pub game_code: [u8; 4],        // 0x00
    pub maker_code: [u8; 2],       // 0x04
    pub disk_id: u8,               // 0x06
    pub version: u8,               // 0x07
    pub audio_streaming: u8,       // 0x08
    pub streaming_buffer_size: u8, // 0x09
    _unused1: [u8; 0x0E],          // 0x0A..0x18
    pub wii_magic: [u8; 4],        // 0x18: 0x5D1C9EA3 on Wii discs, zero otherwise
    pub gc_magic: [u8; 4],         // 0x1C: 0xC2339F3D on GC discs, zero otherwise
    pub game_name: [u8; 0x3E0],    // 0x20
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

impl Header {
    pub fn magic(&self) -> u32 {
        if self.is_wii() {
            u32::from_be_bytes(self.wii_magic)
        } else {
            u32::from_be_bytes(self.gc_magic)
        }
    }

    pub fn is_wii(&self) -> bool {
        self.wii_magic == crate::WII_MAGIC
    }

    pub fn is_gc(&self) -> bool {
        self.gc_magic == crate::GC_MAGIC
    }

    pub fn is_ntsc(&self) -> bool {
        matches!(self.game_code[3], b'E' | b'B' | b'N' | b'J' | b'W' | b'K' | b'Q' | b'T')
    }

    pub fn game_id(&self) -> String {
        let mut buf = String::with_capacity(6);
        for &b in &self.game_code {
            buf.push(if b.is_ascii_graphic() { b as char } else { '_' });
        }
        for &b in &self.maker_code {
            buf.push(if b.is_ascii_graphic() { b as char } else { '_' });
        }
        buf
    }
}

pub fn unshift_wii_header_offsets(bytes: &mut [u8; DVD_HEADER_SIZE]) {
    for off in [0x420, 0x424, 0x428, 0x42C, 0x430, 0x434] {
        let v = u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap());
        bytes[off..off + 4].copy_from_slice(&v.wrapping_shl(2).to_be_bytes());
    }
}

#[repr(C, packed)]
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

#[repr(C, packed)]
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
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
    pub fn parse(fst: &[u8], file_offset_shift: u32) -> Self {
        let total_entries = u32::from_be_bytes(fst[8..12].try_into().unwrap()) as usize;
        let string_table = &fst[total_entries * 12..];

        let mut index = 1;
        let children = Self::parse_dir(fst, string_table, &mut index, total_entries, file_offset_shift);

        Self::Directory {
            name: String::new(),
            children,
        }
    }

    fn parse_dir(fst: &[u8], strings: &[u8], index: &mut usize, end: usize, file_offset_shift: u32) -> Vec<Self> {
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
                    offset: offset << file_offset_shift,
                    size: length,
                });
            } else {
                let next = length as usize;
                let children = Self::parse_dir(fst, strings, index, next, file_offset_shift);
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
