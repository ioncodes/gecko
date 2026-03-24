pub mod dvd;
pub mod elf;
pub mod symbols;

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

pub struct Dol {
    raw: Vec<u8>,
    text_sections: Vec<Section>,
    data_sections: Vec<Section>,
    bss_start: u32,
    bss_size: u32,
    entry_point: u32,
}

impl Dol {
    pub fn parse(data: Vec<u8>) -> Self {
        let mut text_sections = Vec::new();
        let mut data_sections = Vec::new();

        for i in 0..=6 {
            let offset = u32::from_be_bytes(data[0x00 + (i * 4)..0x00 + (i * 4) + 4].try_into().unwrap());
            let vaddr = u32::from_be_bytes(data[0x48 + (i * 4)..0x48 + (i * 4) + 4].try_into().unwrap());
            let size = u32::from_be_bytes(data[0x90 + (i * 4)..0x90 + (i * 4) + 4].try_into().unwrap());

            if size > 0 {
                text_sections.push(Section { offset, vaddr, size });
            }
        }

        for i in 0..=10 {
            let offset = u32::from_be_bytes(data[0x1C + (i * 4)..0x1C + (i * 4) + 4].try_into().unwrap());
            let vaddr = u32::from_be_bytes(data[0x64 + (i * 4)..0x64 + (i * 4) + 4].try_into().unwrap());
            let size = u32::from_be_bytes(data[0xAC + (i * 4)..0xAC + (i * 4) + 4].try_into().unwrap());

            if size > 0 {
                data_sections.push(Section { offset, vaddr, size });
            }
        }

        let bss_start = u32::from_be_bytes(data[0xD8..0xDC].try_into().unwrap());
        let bss_size = u32::from_be_bytes(data[0xDC..0xE0].try_into().unwrap());
        let entry_point = u32::from_be_bytes(data[0xE0..0xE4].try_into().unwrap());

        Dol {
            raw: data,
            text_sections,
            data_sections,
            bss_start,
            bss_size,
            entry_point,
        }
    }
}

impl Dol {
    pub fn size(&self) -> usize {
        self.text_sections
            .iter()
            .chain(self.data_sections.iter())
            .map(|s| (s.offset + s.size) as usize)
            .max()
            .unwrap_or(0)
    }
}

impl Executable for Dol {
    fn text_sections(&self) -> &[Section] {
        &self.text_sections
    }

    fn data_sections(&self) -> &[Section] {
        &self.data_sections
    }

    fn bss(&self) -> (u32, u32) {
        (self.bss_start, self.bss_size)
    }

    fn entry_point(&self) -> u32 {
        self.entry_point
    }

    fn data(&self) -> &[u8] {
        &self.raw
    }
}
