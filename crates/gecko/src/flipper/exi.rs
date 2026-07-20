pub mod device;
pub mod macronix;
pub mod memcard;
pub mod regs;

use crate::flipper::exi::regs::TransferType;
use crate::system::{System, SystemId};

const DEVICE_STATE_CAPACITY: usize = 256;

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

    pub fn device_connected(&self, channel: usize) -> bool {
        self.devices[channel][0].as_ref().is_some_and(|d| d.connected())
    }

    pub fn macronix_mut(&mut self) -> Option<&mut macronix::ExiMacronix> {
        self.devices[macronix::ExiMacronix::CHANNEL][macronix::ExiMacronix::DEVICE]
            .as_mut()
            .and_then(|d| d.as_any_mut())
            .and_then(|any| any.downcast_mut::<macronix::ExiMacronix>())
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
    pub fn start_immediate_transfer<const CHANNEL: usize, const SYSTEM: SystemId>(&mut self) {
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
            if device.is_stub() && SYSTEM == crate::system::GC {
                tracing::warn!(
                    channel = CHANNEL,
                    slot,
                    ?transfer_type,
                    size,
                    "EXI immediate transfer to unimplemented device"
                );
            }

            for i in 0..size {
                if transfer_type == TransferType::Read {
                    bytes[i] = 0;
                }
                device.transfer_byte(&mut bytes[i]);
            }
        } else {
            if SYSTEM == crate::system::GC {
                tracing::warn!(
                    channel = CHANNEL,
                    slot,
                    ?transfer_type,
                    size,
                    "EXI immediate transfer to empty slot"
                );
            }

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
    fn set_exi_interrupt<const CHANNEL: usize>(&mut self, val: bool) {
        match CHANNEL {
            0 => self.ch0_csr.set_exi_interrupt(val),
            1 => self.ch1_csr.set_exi_interrupt(val),
            2 => self.ch2_csr.set_exi_interrupt(val),
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

    if new_cs != prev {
        if new_cs != 0 {
            if let Some(slot) = ExternalInterface::cs_to_slot(new_cs)
                && let Some(device) = &mut sys.exi.devices[CHANNEL][slot]
            {
                device.on_select();
            }
        } else if let Some(slot) = ExternalInterface::cs_to_slot(prev)
            && let Some(delay) = sys.exi.devices[CHANNEL][slot].as_mut().and_then(|d| d.on_deselect())
        {
            sys.scheduler
                .schedule_in(delay, crate::scheduler::Handler::memcard_cmd_done(CHANNEL));
        }
    }

    sys.exi.prev_cs[CHANNEL] = new_cs;
}

pub(crate) fn memcard_cmd_done<const CHANNEL: usize, const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let raise = sys.exi.devices[CHANNEL][0]
        .as_mut()
        .is_some_and(|d| d.complete_command());

    if raise {
        sys.exi.set_exi_interrupt::<CHANNEL>(true);
    }

    self::refresh_interrupts(sys);
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

    let address = crate::mmio::virt_to_phys(address);

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
        let stub = device.is_stub();

        match transfer_type {
            TransferType::Read => {
                if stub && SYSTEM == crate::system::GC {
                    tracing::warn!(
                        channel = CHANNEL,
                        slot,
                        length,
                        "EXI DMA read from unimplemented device"
                    );
                }

                device.dma_read(sys.mmio.phys_slice_mut(address, length as usize));
                #[cfg(feature = "jit")]
                sys.mmio.queue_icbi_for_range(address, length);
            }
            TransferType::Write => {
                if stub && SYSTEM == crate::system::GC {
                    tracing::error!(
                        channel = CHANNEL,
                        slot,
                        length,
                        "EXI DMA write to unimplemented device (data dropped)"
                    );
                }

                device.dma_write(sys.mmio.phys_slice(address, length as usize));
            }
            TransferType::ReadAndWrite | TransferType::Reserved => {
                tracing::error!(
                    channel = CHANNEL,
                    cs,
                    "EXI DMA transfer with invalid/unimplemented transfer type"
                );
            }
        }
    } else if SYSTEM == crate::system::GC {
        tracing::warn!(channel = CHANNEL, slot, ?transfer_type, length, "EXI DMA to empty slot");
    }

    sys.exi.finish_transfer::<CHANNEL>();
}

impl ExternalInterface {
    pub fn save_state(&mut self, w: &mut crate::savestate::StateWriter) {
        w.pod(&self.ch0_csr);
        w.pod(&self.ch0_mar);
        w.pod(&self.ch0_length);
        w.pod(&self.ch0_cr);
        w.pod(&self.ch0_data);
        w.pod(&self.ch1_csr);
        w.pod(&self.ch1_mar);
        w.pod(&self.ch1_length);
        w.pod(&self.ch1_cr);
        w.pod(&self.ch1_data);
        w.pod(&self.ch2_csr);
        w.pod(&self.ch2_mar);
        w.pod(&self.ch2_length);
        w.pod(&self.ch2_cr);
        w.pod(&self.ch2_data);
        w.pod(&self.prev_cs);

        for device in self.devices.iter_mut().flatten() {
            match device {
                Some(device) => {
                    w.bool(true);

                    let mut inner = crate::savestate::StateWriter::with_capacity(DEVICE_STATE_CAPACITY);
                    device.save_state(&mut inner);
                    w.bytes(&inner.into_inner());
                }
                None => w.bool(false),
            }
        }
    }

    pub fn load_state(
        &mut self,
        r: &mut crate::savestate::StateReader<'_>,
    ) -> Result<(), crate::savestate::StateError> {
        self.ch0_csr = r.pod()?;
        self.ch0_mar = r.pod()?;
        self.ch0_length = r.pod()?;
        self.ch0_cr = r.pod()?;
        self.ch0_data = r.pod()?;
        self.ch1_csr = r.pod()?;
        self.ch1_mar = r.pod()?;
        self.ch1_length = r.pod()?;
        self.ch1_cr = r.pod()?;
        self.ch1_data = r.pod()?;
        self.ch2_csr = r.pod()?;
        self.ch2_mar = r.pod()?;
        self.ch2_length = r.pod()?;
        self.ch2_cr = r.pod()?;
        self.ch2_data = r.pod()?;
        self.prev_cs = r.pod()?;

        for device in self.devices.iter_mut().flatten() {
            let present = r.bool()?;
            if present != device.is_some() {
                return Err(crate::savestate::StateError::Corrupt(
                    "EXI device topology mismatch (different boot configuration?)",
                ));
            }

            if let Some(device) = device {
                let payload = r.bytes()?;
                let mut inner = crate::savestate::StateReader::new(payload);
                device.load_state(&mut inner)?;
            }
        }

        Ok(())
    }
}
