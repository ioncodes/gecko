pub mod device;
pub mod regs;

use crate::exi::regs::TransferType;
use crate::mmio::Mmio;
use crate::mmio::constants::EXI_BASE;
use crate::mmio::traits::{MmioAccess, MmioRegister};

pub struct Exi {
    // Channel 0
    pub ch0_csr: regs::Channel0Status,
    pub ch0_mar: regs::Channel0DmaAddress,
    pub ch0_length: regs::Channel0DmaLength,
    pub ch0_cr: regs::Channel0Control,
    pub ch0_data: regs::Channel0Data,
    // Channel 1
    pub ch1_csr: regs::Channel1Status,
    pub ch1_mar: regs::Channel1DmaAddress,
    pub ch1_length: regs::Channel1DmaLength,
    pub ch1_cr: regs::Channel1Control,
    pub ch1_data: regs::Channel1Data,
    // Channel 2
    pub ch2_csr: regs::Channel2Status,
    pub ch2_mar: regs::Channel2DmaAddress,
    pub ch2_length: regs::Channel2DmaLength,
    pub ch2_cr: regs::Channel2Control,
    pub ch2_data: regs::Channel2Data,
    // Devices: [channel][device_slot], 3 channels x 3 slots
    devices: [[Option<Box<dyn device::ExiDevice>>; 3]; 3],
}

impl Exi {
    pub fn new() -> Self {
        Exi {
            ch0_csr: regs::Channel0Status::from_raw(0),
            ch0_mar: regs::Channel0DmaAddress::from_raw(0),
            ch0_length: regs::Channel0DmaLength::from_raw(0),
            ch0_cr: regs::Channel0Control::from_raw(0),
            ch0_data: regs::Channel0Data::from_raw(0),
            ch1_csr: regs::Channel1Status::from_raw(0),
            ch1_mar: regs::Channel1DmaAddress::from_raw(0),
            ch1_length: regs::Channel1DmaLength::from_raw(0),
            ch1_cr: regs::Channel1Control::from_raw(0),
            ch1_data: regs::Channel1Data::from_raw(0),
            ch2_csr: regs::Channel2Status::from_raw(0),
            ch2_mar: regs::Channel2DmaAddress::from_raw(0),
            ch2_length: regs::Channel2DmaLength::from_raw(0),
            ch2_cr: regs::Channel2Control::from_raw(0),
            ch2_data: regs::Channel2Data::from_raw(0),
            devices: [[None, None, None], [None, None, None], [None, None, None]],
        }
    }

    pub fn dummy() -> Self {
        let mut exi = Self::new();
        for ch in 0..3 {
            for slot in 0..3 {
                exi.attach_device(ch, slot, Box::new(device::ExiDummy));
            }
        }
        exi
    }

    pub fn attach_device(&mut self, channel: usize, slot: usize, device: Box<dyn device::ExiDevice>) {
        self.devices[channel][slot] = Some(device);
    }

    /// Decode chip_select field to device slot index (0, 1, or 2)
    fn cs_to_slot(cs: u8) -> Option<usize> {
        match cs {
            0b001 => Some(0),
            0b010 => Some(1),
            0b100 => Some(2),
            _ => None,
        }
    }

    fn start_immediate_transfer(&mut self, channel: usize) {
        match channel {
            0 => self.start_immediate_transfer_for_channel::<0>(),
            1 => self.start_immediate_transfer_for_channel::<1>(),
            2 => self.start_immediate_transfer_for_channel::<2>(),
            _ => unreachable!(),
        }
    }

    fn start_immediate_transfer_for_channel<const CHANNEL: usize>(&mut self) {
        let (transfer_type, transfer_length, chip_select, mut bytes) = match CHANNEL {
            0 => (
                self.ch0_cr.transfer_type(),
                self.ch0_cr.transfer_length(),
                self.ch0_csr.chip_select(),
                self.ch0_data.raw().to_be_bytes(),
            ),
            1 => (
                self.ch1_cr.transfer_type(),
                self.ch1_cr.transfer_length(),
                self.ch1_csr.chip_select(),
                self.ch1_data.raw().to_be_bytes(),
            ),
            2 => (
                self.ch2_cr.transfer_type(),
                self.ch2_cr.transfer_length(),
                self.ch2_csr.chip_select(),
                self.ch2_data.raw().to_be_bytes(),
            ),
            _ => unreachable!(),
        };

        let slot = match Self::cs_to_slot(chip_select) {
            Some(s) => s,
            None => {
                tracing::warn!(
                    channel = CHANNEL,
                    cs = chip_select,
                    "EXI immediate transfer with no/invalid chip select"
                );
                self.finish_transfer_for_channel::<CHANNEL>();
                return;
            }
        };

        let size = (transfer_length as usize) + 1;

        // TODO: idk
        if let Some(device) = &mut self.devices[CHANNEL][slot] {
            for i in 0..size {
                if transfer_type == TransferType::Read {
                    bytes[i] = 0;
                }

                device.transfer_byte(&mut bytes[i]);
            }
        } else {
            bytes[..size].fill(0);
        }

        self.set_data_for_channel::<CHANNEL>(u32::from_be_bytes(bytes));
        self.finish_transfer_for_channel::<CHANNEL>();
    }

    fn pending_dma(&self, channel: usize) -> Option<(u8, TransferType, u32, u32)> {
        match channel {
            0 => self.pending_dma_for_channel::<0>(),
            1 => self.pending_dma_for_channel::<1>(),
            2 => self.pending_dma_for_channel::<2>(),
            _ => None,
        }
    }

    fn pending_dma_for_channel<const CHANNEL: usize>(&self) -> Option<(u8, TransferType, u32, u32)> {
        let (transfer_start, dma_mode, chip_select, transfer_type, address, length) = match CHANNEL {
            0 => (
                self.ch0_cr.transfer_start(),
                self.ch0_cr.dma_mode(),
                self.ch0_csr.chip_select(),
                self.ch0_cr.transfer_type(),
                self.ch0_mar.address(),
                self.ch0_length.length(),
            ),
            1 => (
                self.ch1_cr.transfer_start(),
                self.ch1_cr.dma_mode(),
                self.ch1_csr.chip_select(),
                self.ch1_cr.transfer_type(),
                self.ch1_mar.address(),
                self.ch1_length.length(),
            ),
            2 => (
                self.ch2_cr.transfer_start(),
                self.ch2_cr.dma_mode(),
                self.ch2_csr.chip_select(),
                self.ch2_cr.transfer_type(),
                self.ch2_mar.address(),
                self.ch2_length.length(),
            ),
            _ => unreachable!(),
        };

        if !transfer_start || !dma_mode {
            return None;
        }

        Some((chip_select, transfer_type, address << 5, length << 5))
    }

    pub fn process_dma_transfers(&mut self, mmio: &mut Mmio) {
        const CHANNEL_COUNT: usize = 3;

        for channel in 0..CHANNEL_COUNT {
            let Some((cs, transfer_type, address, length)) = self.pending_dma(channel) else {
                continue;
            };

            tracing::debug!(
                channel,
                cs,
                ?transfer_type,
                address = format!("{:08X}", address),
                length,
                "EXI DMA transfer"
            );

            let slot = match Self::cs_to_slot(cs) {
                Some(s) => s,
                None => {
                    tracing::warn!(channel, cs, "EXI DMA transfer with no/invalid chip select");
                    self.finish_transfer(channel);
                    continue;
                }
            };

            if let Some(device) = &mut self.devices[channel][slot] {
                match transfer_type {
                    TransferType::Read => device.dma_read(mmio.phys_slice_mut(address, length as usize)),
                    TransferType::Write => device.dma_write(mmio.phys_slice(address, length as usize)),
                    TransferType::ReadAndWrite | TransferType::Reserved => {
                        tracing::error!(channel, cs, "EXI DMA transfer with invalid/unimplemented transfer type");
                    }
                }
            }

            self.finish_transfer(channel);
        }
    }

    fn finish_transfer(&mut self, channel: usize) {
        match channel {
            0 => self.finish_transfer_for_channel::<0>(),
            1 => self.finish_transfer_for_channel::<1>(),
            2 => self.finish_transfer_for_channel::<2>(),
            _ => {}
        }
    }

    fn set_data_for_channel<const CHANNEL: usize>(&mut self, val: u32) {
        match CHANNEL {
            0 => self.ch0_data = regs::Channel0Data::from_raw(val),
            1 => self.ch1_data = regs::Channel1Data::from_raw(val),
            2 => self.ch2_data = regs::Channel2Data::from_raw(val),
            _ => unreachable!(),
        }
    }

    fn finish_transfer_for_channel<const CHANNEL: usize>(&mut self) {
        match CHANNEL {
            0 => {
                self.ch0_cr.set_transfer_start(false);
                self.ch0_csr.set_tc_interrupt(true);
            }
            1 => {
                self.ch1_cr.set_transfer_start(false);
                self.ch1_csr.set_tc_interrupt(true);
            }
            2 => {
                self.ch2_cr.set_transfer_start(false);
                self.ch2_csr.set_tc_interrupt(true);
            }
            _ => unreachable!(),
        }
    }

    crate::impl_mmio_dispatch!(
        regs::Channel0Status,
        regs::Channel0DmaAddress,
        regs::Channel0DmaLength,
        regs::Channel0Control,
        regs::Channel0Data,
        regs::Channel1Status,
        regs::Channel1DmaAddress,
        regs::Channel1DmaLength,
        regs::Channel1Control,
        regs::Channel1Data,
        regs::Channel2Status,
        regs::Channel2DmaAddress,
        regs::Channel2DmaLength,
        regs::Channel2Control,
        regs::Channel2Data,
    );

    pub fn mmio_read_u8(&self, offset: u32) -> u8 {
        self.read_raw(EXI_BASE + offset, 1).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled EXI read_u8");
            0
        }) as u8
    }

    pub fn mmio_write_u8(&mut self, offset: u32, val: u8) {
        if !self.write_raw(EXI_BASE + offset, 1, val as u32) {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled EXI write_u8");
        }
    }

    pub fn mmio_read_u16(&self, offset: u32) -> u16 {
        self.read_raw(EXI_BASE + offset, 2).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled EXI read_u16");
            0
        }) as u16
    }

    pub fn mmio_write_u16(&mut self, offset: u32, val: u16) {
        if !self.write_raw(EXI_BASE + offset, 2, val as u32) {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled EXI write_u16");
        }
    }

    pub fn mmio_read_u32(&self, offset: u32) -> u32 {
        self.read_raw(EXI_BASE + offset, 4).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled EXI read_u32");
            0
        })
    }

    pub fn mmio_write_u32(&mut self, offset: u32, val: u32) {
        if !self.write_raw(EXI_BASE + offset, 4, val) {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled EXI write_u32");
        }
    }
}
