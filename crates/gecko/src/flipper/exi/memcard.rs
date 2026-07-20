use std::path::PathBuf;

const BLOCK_SIZE: u32 = 0x2000;
const PAGE_SIZE: usize = 128;

const MC_STATUS_BUSY: u8 = 0x80;
const MC_STATUS_UNLOCKED: u8 = 0x40;
const MC_STATUS_ERASE_ERROR: u8 = 0x10;
const MC_STATUS_PROGRAM_ERROR: u8 = 0x08;
const MC_STATUS_READY: u8 = 0x01;
const MC_STATUS_INIT: u8 = MC_STATUS_UNLOCKED | MC_STATUS_READY;

const FLASH_ID: u16 = 0xC221;
const CMD_DONE_DELAY_CYCLES: u64 = 5000;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Command {
    NintendoId,
    ReadArray,
    SetInterrupt,
    ReadStatus,
    ReadId,
    ClearStatus,
    SectorErase,
    PageProgram,
    ChipErase,
    Unknown,
}

impl Command {
    fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Command::NintendoId,
            0x52 => Command::ReadArray,
            0x81 => Command::SetInterrupt,
            0x83 => Command::ReadStatus,
            0x85 => Command::ReadId,
            0x89 => Command::ClearStatus,
            0xF1 => Command::SectorErase,
            0xF2 => Command::PageProgram,
            0xF4 => Command::ChipErase,
            _ => Command::Unknown,
        }
    }

    fn to_byte(self) -> u8 {
        match self {
            Command::NintendoId => 0x00,
            Command::ReadArray => 0x52,
            Command::SetInterrupt => 0x81,
            Command::ReadStatus => 0x83,
            Command::ReadId => 0x85,
            Command::ClearStatus => 0x89,
            Command::SectorErase => 0xF1,
            Command::PageProgram => 0xF2,
            Command::ChipErase => 0xF4,
            Command::Unknown => 0xFF,
        }
    }
}

pub struct ExiMemoryCard {
    data: Vec<u8>,
    path: Option<PathBuf>,
    size_mask: u32,
    card_id: u32,

    command: Command,
    position: u32,
    address: u32,
    status: u8,
    interrupt_switch: u8,
    program_buffer: [u8; PAGE_SIZE],
}

impl ExiMemoryCard {
    pub fn new(path: Option<PathBuf>, total_blocks: u32) -> Self {
        let size_bytes = (total_blocks * BLOCK_SIZE) as usize;

        let mut data = vec![0xFF; size_bytes];

        if let Some(path) = &path {
            super::device::load_backing(path, &mut data, "memory card");
        }

        Self {
            data,
            path,
            size_mask: size_bytes as u32 - 1,
            card_id: total_blocks / 16,
            command: Command::NintendoId,
            position: 0,
            address: 0,
            status: MC_STATUS_INIT,
            interrupt_switch: 0,
            program_buffer: [0; PAGE_SIZE],
        }
    }

    fn flush(&self) {
        if let Some(path) = &self.path {
            super::device::persist_backing(path, &self.data, "memory card");
        }
    }

    fn accumulate_address(&mut self, byte: u8) {
        match self.position {
            1 => self.address = (byte as u32) << 17,
            2 => self.address |= (byte as u32) << 9,
            3 => self.address |= ((byte as u32) & 3) << 7,
            4 => self.address |= (byte as u32) & 0x7F,
            _ => {}
        }
    }

    fn advance_address(&mut self) {
        self.address = (self.address & !0x1FF) | ((self.address + 1) & 0x1FF);
    }
}

impl super::device::ExiDevice for ExiMemoryCard {
    fn on_select(&mut self) {
        self.position = 0;
    }

    fn transfer_byte(&mut self, byte: &mut u8) {
        if self.position == 0 {
            let opcode = *byte;
            self.command = Command::from_byte(opcode);
            *byte = 0xFF;

            tracing::debug!(opcode = format!("{:02X}", opcode), "memory card command");

            if self.command == Command::ClearStatus {
                self.status &= !(MC_STATUS_PROGRAM_ERROR | MC_STATUS_ERASE_ERROR);
                self.status |= MC_STATUS_READY;
                self.position = 0;
            }
        } else {
            match self.command {
                Command::NintendoId => {
                    if self.position == 1 {
                        *byte = 0x80;
                    } else {
                        let shift = 24 - (((self.position - 2) & 3) * 8);
                        *byte = (self.card_id >> shift) as u8;
                    }
                }
                Command::ReadArray => {
                    self.accumulate_address(*byte);

                    if self.position == 1 {
                        *byte = 0xFF;
                    } else {
                        *byte = self.data[(self.address & self.size_mask) as usize];

                        if self.position >= 9 {
                            self.advance_address();
                        }
                    }
                }
                Command::ReadStatus => *byte = self.status,
                Command::ReadId => {
                    *byte = if self.position == 1 || self.position & 1 == 0 {
                        (FLASH_ID >> 8) as u8
                    } else {
                        FLASH_ID as u8
                    };
                }
                Command::SectorErase => {
                    self.accumulate_address(*byte);
                    *byte = 0xFF;
                }
                Command::SetInterrupt => {
                    if self.position == 1 {
                        self.interrupt_switch = *byte;
                    }
                    *byte = 0xFF;
                }
                Command::PageProgram => {
                    self.accumulate_address(*byte);

                    if self.position >= 5 {
                        self.program_buffer[((self.position - 5) & 0x7F) as usize] = *byte;
                    }
                    *byte = 0xFF;
                }
                _ => *byte = 0xFF,
            }
        }

        self.position += 1;
    }

    fn on_deselect(&mut self) -> Option<u64> {
        match self.command {
            Command::SectorErase if self.position > 2 => {
                let block = (self.address & self.size_mask) & !(BLOCK_SIZE - 1);
                let start = block as usize;
                self.data[start..start + BLOCK_SIZE as usize].fill(0xFF);

                self.status |= MC_STATUS_BUSY;
                self.status &= !MC_STATUS_READY;

                self.flush();
                tracing::debug!(block = block / BLOCK_SIZE, "memory card sector erase");

                Some(CMD_DONE_DELAY_CYCLES)
            }
            Command::PageProgram if self.position >= 5 => {
                self.status &= !MC_STATUS_BUSY;

                let count = self.position - 5;
                for i in 0..count {
                    self.data[(self.address & self.size_mask) as usize] = self.program_buffer[(i & 0x7F) as usize];
                    self.advance_address();
                }

                self.flush();
                tracing::debug!(bytes = count, "memory card page program");

                Some(CMD_DONE_DELAY_CYCLES)
            }
            Command::ChipErase if self.position > 2 => {
                self.data.fill(0xFF);
                self.status &= !MC_STATUS_BUSY;

                self.flush();
                tracing::debug!("memory card chip erase");

                None
            }
            _ => None,
        }
    }

    fn complete_command(&mut self) -> bool {
        self.status |= MC_STATUS_READY;
        self.status &= !MC_STATUS_BUSY;
        self.interrupt_switch != 0
    }

    fn dma_read(&mut self, buf: &mut [u8]) {
        let mask = self.size_mask as usize;
        let start = (self.address & self.size_mask) as usize;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.data[(start + i) & mask];
        }
    }

    fn dma_write(&mut self, buf: &[u8]) {
        let mask = self.size_mask as usize;
        let start = (self.address & self.size_mask) as usize;
        for (i, b) in buf.iter().enumerate() {
            self.data[(start + i) & mask] = *b;
        }
        self.flush();
    }

    fn save_state(&mut self, w: &mut crate::savestate::StateWriter) {
        w.bytes(&self.data);
        w.u8(self.command.to_byte());
        w.u32(self.position);
        w.u32(self.address);
        w.u8(self.status);
        w.u8(self.interrupt_switch);
        w.pod(&self.program_buffer);
    }

    fn load_state(&mut self, r: &mut crate::savestate::StateReader<'_>) -> Result<(), crate::savestate::StateError> {
        r.bytes_into(&mut self.data)?;

        self.command = Command::from_byte(r.u8()?);

        self.position = r.u32()?;
        self.address = r.u32()?;
        self.status = r.u8()?;
        self.interrupt_switch = r.u8()?;
        self.program_buffer = r.pod()?;

        self.flush();
        Ok(())
    }
}
