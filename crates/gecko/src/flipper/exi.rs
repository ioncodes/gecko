pub mod device;
pub mod macronix;
pub mod regs;

use crate::flipper::exi::regs::TransferType;
use crate::system::{System, SystemId};

pub struct ExternalInterface {
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
    prev_cs: [u8; 3],
}

impl ExternalInterface {
    pub fn new() -> Self {
        ExternalInterface {
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
            devices: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            prev_cs: [0; 3],
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

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        Self::channel_interrupt_active(&self.ch0_csr)
            || Self::channel_interrupt_active(&self.ch1_csr)
            || Self::channel_interrupt_active(&self.ch2_csr)
    }

    #[inline(always)]
    fn channel_interrupt_active(csr: &impl regs::ChannelStatus) -> bool {
        (csr.exi_interrupt() && csr.exi_interrupt_mask())
            || (csr.tc_interrupt() && csr.tc_interrupt_mask())
            || (csr.ext_interrupt() && csr.ext_interrupt_mask())
    }

    #[inline(always)]
    pub fn start_immediate_transfer<const CHANNEL: usize>(&mut self) {
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
                self.finish_transfer::<CHANNEL>();
                return;
            }
        };

        let size = (transfer_length as usize) + 1;

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

        self.set_data::<CHANNEL>(u32::from_be_bytes(bytes));
        self.finish_transfer::<CHANNEL>();
    }

    #[inline(always)]
    fn set_data<const CHANNEL: usize>(&mut self, val: u32) {
        match CHANNEL {
            0 => self.ch0_data = regs::Channel0Data::from_raw(val),
            1 => self.ch1_data = regs::Channel1Data::from_raw(val),
            2 => self.ch2_data = regs::Channel2Data::from_raw(val),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn finish_transfer<const CHANNEL: usize>(&mut self) {
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
}

crate::mmio_device_dispatch! {
    read = exi_read,
    write = exi_write,
    registers = [
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
    ],
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    if sys.exi.interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Exi);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Exi);
    }
}

#[inline(always)]
pub fn on_chip_select_written<const CHANNEL: usize, const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, new_cs: u8) {
    let prev = sys.exi.prev_cs[CHANNEL];
    if new_cs != prev && new_cs != 0 {
        if let Some(slot) = ExternalInterface::cs_to_slot(new_cs)
            && let Some(device) = &mut sys.exi.devices[CHANNEL][slot]
        {
            device.on_select();
        }
    }
    sys.exi.prev_cs[CHANNEL] = new_cs;
}

#[inline(always)]
pub fn run_dma<const CHANNEL: usize, const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let (cs, transfer_type, address, length) = match CHANNEL {
        0 => (
            sys.exi.ch0_csr.chip_select(),
            sys.exi.ch0_cr.transfer_type(),
            sys.exi.ch0_mar.address() << 5,
            sys.exi.ch0_length.length() << 5,
        ),
        1 => (
            sys.exi.ch1_csr.chip_select(),
            sys.exi.ch1_cr.transfer_type(),
            sys.exi.ch1_mar.address() << 5,
            sys.exi.ch1_length.length() << 5,
        ),
        2 => (
            sys.exi.ch2_csr.chip_select(),
            sys.exi.ch2_cr.transfer_type(),
            sys.exi.ch2_mar.address() << 5,
            sys.exi.ch2_length.length() << 5,
        ),
        _ => unreachable!(),
    };

    tracing::debug!(
        channel = CHANNEL,
        cs,
        ?transfer_type,
        address = format!("{:08X}", address),
        length,
        "EXI DMA transfer"
    );

    let slot = match ExternalInterface::cs_to_slot(cs) {
        Some(s) => s,
        None => {
            tracing::warn!(channel = CHANNEL, cs, "EXI DMA transfer with no/invalid chip select");
            sys.exi.finish_transfer::<CHANNEL>();
            return;
        }
    };

    if let Some(device) = &mut sys.exi.devices[CHANNEL][slot] {
        match transfer_type {
            TransferType::Read => device.dma_read(sys.mmio.phys_slice_mut(address, length as usize)),
            TransferType::Write => device.dma_write(sys.mmio.phys_slice(address, length as usize)),
            TransferType::ReadAndWrite | TransferType::Reserved => {
                tracing::error!(
                    channel = CHANNEL,
                    cs,
                    "EXI DMA transfer with invalid/unimplemented transfer type"
                );
            }
        }
    }

    sys.exi.finish_transfer::<CHANNEL>();
}
