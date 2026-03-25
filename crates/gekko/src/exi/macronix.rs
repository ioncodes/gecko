// GameCube IPL ROM / SRAM / RTC device (EXI channel 0, device 1)
// Macronix device according to yagcd, TODO: update name
// - Address 0x000000..0x1FFFFF: Mask ROM (2 MB)
// - Address 0x800000..0x800043: SRAM (TODO: 64 or 68 bytes??)
// - Address 0x840000: RTC (seconds since 2000-01-01?)

const IPL_START: u32 = 0x000000;
const IPL_END: u32 = 0x1FFFFF;

const SRAM_START: u32 = 0x800000;
const SRAM_END: u32 = 0x800043;
const SRAM_SIZE: usize = 68;

const RTC_START: u32 = 0x840000;
const RTC_END: u32 = 0x840004;

pub struct ExiMacronix {
    rom: Vec<u8>,
    sram: Sram,
    command: u32,
    bytes_received: usize,
    cursor: usize,
}

impl ExiMacronix {
    pub const CHANNEL: usize = 0;
    pub const DEVICE: usize = 1;

    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            sram: Sram::ntsc_default(),
            command: 0,
            bytes_received: 0,
            cursor: 0,
        }
    }

    fn address(&self) -> u32 {
        (self.command >> 6) & 0x1FF_FFFF
    }

    fn is_write(&self) -> bool {
        self.command & 0x8000_0000 != 0
    }

    fn rtc_seconds() -> u32 {
        use std::time::{SystemTime, UNIX_EPOCH};
        const GC_EPOCH_OFFSET: u64 = 946_684_800;
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| (d.as_secs() - GC_EPOCH_OFFSET) as u32)
            .unwrap_or(0)
    }
}

impl super::device::ExiDevice for ExiMacronix {
    fn on_select(&mut self) {
        self.command = 0;
        self.bytes_received = 0;
        self.cursor = 0;
    }

    fn transfer_byte(&mut self, byte: &mut u8) {
        if self.bytes_received < 4 {
            self.command = (self.command << 8) | (*byte as u32);
            *byte = 0xFF;
            self.bytes_received += 1;
            return;
        }

        let addr = self.address();

        match addr {
            IPL_START..=IPL_END => {
                if !self.is_write() {
                    let offset = (addr as usize + self.cursor) % self.rom.len().max(1);
                    *byte = self.rom[offset];
                    tracing::debug!(
                        addr = format!("{:06X}", addr),
                        cursor = self.cursor,
                        value = format!("{:02X}", byte),
                        "IPL byte read"
                    );
                }
            }
            SRAM_START..=SRAM_END => {
                let offset = ((addr - SRAM_START) as usize + self.cursor) % SRAM_SIZE;
                if self.is_write() {
                    self.sram.data[offset] = *byte;
                    tracing::debug!(
                        addr = format!("{:06X}", addr),
                        cursor = self.cursor,
                        value = format!("{:02X}", byte),
                        "SRAM byte write"
                    );
                } else {
                    *byte = self.sram.data[offset];
                    tracing::debug!(
                        addr = format!("{:06X}", addr),
                        cursor = self.cursor,
                        value = format!("{:02X}", byte),
                        "SRAM byte read"
                    );
                }
            }
            RTC_START..=RTC_END => {
                if !self.is_write() {
                    let rtc = Self::rtc_seconds().to_be_bytes();
                    *byte = rtc[self.cursor % 4];
                    tracing::debug!(
                        addr = format!("{:06X}", addr),
                        cursor = self.cursor,
                        value = format!("{:02X}", byte),
                        "RTC byte read"
                    );
                }
            }
            _ => {
                if !self.is_write() {
                    *byte = 0;
                    tracing::debug!(addr = format!("{:06X}", addr), "Unknown address read");
                }
            }
        }

        self.cursor += 1;
    }

    fn dma_read(&mut self, buf: &mut [u8]) {
        let addr = self.address();
        match addr {
            IPL_START..=IPL_END => {
                let start = addr as usize;
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = self.rom[start + i];
                }
                tracing::debug!(
                    addr = format!("{:06X}", addr),
                    size = buf.len(),
                    data = format!("{:02X?}", &buf),
                    "IPL DMA read"
                );
            }
            SRAM_START..=SRAM_END => {
                let base = (addr - SRAM_START) as usize;
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = self.sram.data[(base + i) % SRAM_SIZE];
                }
                tracing::debug!(
                    addr = format!("{:06X}", addr),
                    size = buf.len(),
                    data = format!("{:02X?}", &buf),
                    "SRAM DMA read"
                );
            }
            RTC_START..=RTC_END => {
                let rtc = Self::rtc_seconds();
                let rtc_bytes = rtc.to_be_bytes();
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = rtc_bytes[i % 4];
                }
                tracing::debug!(
                    addr = format!("{:06X}", addr),
                    size = buf.len(),
                    data = format!("{:02X?}", &buf),
                    time = rtc,
                    "RTC DMA read"
                );
            }
            _ => {
                buf.fill(0);
            }
        }
    }

    fn dma_write(&mut self, buf: &[u8]) {
        let addr = self.address();
        if addr >= SRAM_START && addr <= SRAM_END {
            let base = (addr - SRAM_START) as usize;
            for (i, b) in buf.iter().enumerate() {
                self.sram.data[(base + i) % SRAM_SIZE] = *b;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Sram {
    pub data: [u8; SRAM_SIZE],
}

impl Sram {
    /// Dumped from NTSC IPL
    #[rustfmt::skip]
    pub fn ntsc_default() -> Self {
        Self {
            data: [
                0x01, 0x04, 0xFE, 0xFB,
                0x00, 0x14, 0xFF, 0xE8,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x14,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
                0x01, 0x04, 0xFE, 0xFB, // TODO: start and end is cooked? isnt it supposed to be 64 bytes?
            ],
        }
    }

    // TODO: WIP
    pub fn fix_checksums(&mut self) {
        let (a, b) = Self::compute_checksums(&self.data);
        self.data[0x00..0x02].copy_from_slice(&a.to_be_bytes());
        self.data[0x02..0x04].copy_from_slice(&b.to_be_bytes());
    }

    fn compute_checksums(data: &[u8; SRAM_SIZE]) -> (u16, u16) {
        let mut sum: u16 = 0;
        for i in (0x04..0x40).step_by(2) {
            let word = u16::from_be_bytes([data[i], data[i + 1]]);
            sum = sum.wrapping_add(word);
        }
        (sum, !sum)
    }
}
