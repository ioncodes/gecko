pub mod regs;

use crate::system::{System, SystemId};

pub enum Command {
    DriveInfo,
    ReadDiskId,
    ReadSectorData,
    AudioToggle(bool),
}

pub struct DvdInterface {
    pub status: regs::DiStatusRegister,
    pub cover: regs::DiCoverRegister,
    pub dma_address: regs::DiDmaAddressRegister,
    pub dma_length: regs::DiDmaLengthRegister,
    pub control: regs::DiControlRegister,
    pub config: regs::DiConfigurationRegister,
    pub cmdbuf0: u32,
    pub cmdbuf1: u32,
    pub cmdbuf2: u32,
    pub immbuf: u32,
    pub dvd: Option<Box<dyn image::Dvd>>,
}

impl DvdInterface {
    pub fn new() -> Self {
        Self {
            status: regs::DiStatusRegister::from_raw(0),
            cover: regs::DiCoverRegister::from_raw(0),
            dma_address: regs::DiDmaAddressRegister::from_raw(0),
            dma_length: regs::DiDmaLengthRegister::from_raw(0),
            control: regs::DiControlRegister::from_raw(0),
            config: regs::DiConfigurationRegister::default(),
            cmdbuf0: 0,
            cmdbuf1: 0,
            cmdbuf2: 0,
            immbuf: 0,
            dvd: None,
        }
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        (self.status.break_complete() && self.status.break_complete_mask())
            || (self.status.device_error() && self.status.device_error_mask())
            || (self.status.transfer_complete() && self.status.transfer_complete_mask())
            || (self.cover.cover_interrupt() && self.cover.cover_interrupt_mask())
    }

    #[inline(always)]
    fn resolve_command(&self) -> Option<Command> {
        let cmd = self.cmdbuf0 >> 24;
        let sub1 = (self.cmdbuf0 >> 16) & 0xFF;
        let sub2 = self.cmdbuf0 & 0xFFFF;

        match (cmd, sub1, sub2) {
            (0x12, 0x00, 0x0000) => Some(Command::DriveInfo),
            (0xA8, _, 0x0000) => Some(Command::ReadSectorData),
            (0xA8, _, 0x0040) => Some(Command::ReadDiskId),
            (0xE4, 0x00, _) => Some(Command::AudioToggle(false)),
            (0xE4, 0x01, _) => Some(Command::AudioToggle(true)),
            _ => {
                tracing::error!(
                    cmd = format!("{cmd:02X}"),
                    sub1 = format!("{sub1:02X}"),
                    sub2 = format!("{sub2:04X}"),
                    cmdbuf1 = format!("{:08X}", self.cmdbuf1),
                    cmdbuf2 = format!("{:08X}", self.cmdbuf2),
                    "Unknown DI command"
                );
                None
            }
        }
    }
}

crate::mmio_device_dispatch! {
    read = di_read,
    write = di_write,
    registers = [
        regs::DiStatusRegister,
        regs::DiCoverRegister,
        regs::DiCommandBuf0,
        regs::DiCommandBuf1,
        regs::DiCommandBuf2,
        regs::DiDmaAddressRegister,
        regs::DiDmaLengthRegister,
        regs::DiControlRegister,
        regs::DiImmBuf,
        regs::DiConfigurationRegister,
    ],
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    if sys.di.interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Di);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Di);
    }
}

pub fn start_transfer<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let Some(dvd) = sys.di.dvd.take() else {
        // TODO: Setting this causes unrecoverable error when there is no DVD
        //sys.di.status.set_device_error(true);
        sys.di.control.set_tstart(false);
        return;
    };

    if let Some(cmd) = sys.di.resolve_command() {
        self::process_dvd_command(sys, cmd, &dvd);
    }

    sys.di.dvd = Some(dvd);

    // The interrupt must not fire until some time!!
    // This is important as it will break the IPL if we immediately raise it...
    // When DVDReadDiskID issues the DI command, it returns to BS2_Tick which updates
    // the internal state machine and then calls into restore_irq. The code assumes
    // that the interrupt wont fire until after it has returned and updated the state.
    // Else it will get trapped inside restore_irq, right after mtmsr, executing in a loop
    // of the DVD dispatch handler which in turn re-issues the same command...
    const DI_TRANSFER_DELAY_US: u64 = 20; // Based off of vxpm and hazel (~10k cycles at GC clock)
    sys.scheduler.schedule_in(
        crate::scheduler::microseconds_to_cycles(SYSTEM, DI_TRANSFER_DELAY_US),
        |sys| {
            sys.di.control.set_tstart(false);
            // DMA length tracks the progress of the transfer, so when it hits 0, the
            // transfer is complete. On failure, this would denote how many bytes were
            // not transferred, but we close our eyes and just hope nothing depends on
            // that!
            sys.di.dma_length = regs::DiDmaLengthRegister::from_raw(0);
            sys.di.status.set_transfer_complete(true);
            self::refresh_interrupts(sys);
        },
    );
}

#[inline(always)]
fn process_dvd_command<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, cmd: Command, dvd: &dyn image::Dvd) {
    match cmd {
        Command::DriveInfo => {
            let dst = sys.di.dma_address.address();
            let buffer = sys.mmio.phys_slice_mut(dst, 0x20);
            buffer.copy_from_slice(&[0x69; 0x20]); // TODO: Drive Info?
        }
        Command::ReadSectorData => {
            let src = sys.di.cmdbuf1 << 2;
            let dst = sys.di.dma_address.address();
            let len = sys.di.cmdbuf2 as usize;
            assert!(len == sys.di.dma_length.length() as usize, "DMA length mismatch");

            let buffer = sys.mmio.phys_slice_mut(dst, len);
            dvd.read_disc_into(src as usize, buffer);

            tracing::debug!(
                src = format!("{:08X}", src),
                dst = format!("{:08X}", dst),
                len = format!("{:08X}", len),
                "ReadSectorData command"
            );
        }
        Command::ReadDiskId => {
            let src = sys.di.cmdbuf1;
            let dst = sys.di.dma_address.address();
            let len = sys.di.dma_length.length() as usize;

            let buffer = sys.mmio.phys_slice_mut(dst, len);
            dvd.read_disc_into(0, buffer);

            tracing::debug!(
                src = format!("{:08X}", src),
                dst = format!("{:08X}", dst),
                len = format!("{:08X}", len),
                "ReadDiskId command"
            );
        }
        Command::AudioToggle(enable) => {
            tracing::warn!(enable, "AudioToggle stubbed");
            sys.di.immbuf = 0;
        }
    }
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    pub fn insert_dvd(&mut self, dvd: Box<dyn image::Dvd>) {
        let name = String::from_utf8_lossy(&dvd.header().game_name);
        let name = name.trim_end_matches('\0');
        tracing::info!("DVD inserted: {}", name);
        self.di.dvd = Some(dvd);
        self.close_cover();
    }

    pub fn open_cover(&mut self) {
        tracing::debug!("DVD drive cover opened");
        self.di.cover = self.di.cover.with_cover_interrupt(true).with_cover_status(true);
        self::refresh_interrupts(self);
    }

    pub fn close_cover(&mut self) {
        tracing::debug!("DVD drive cover closed");
        self.di.cover = self.di.cover.with_cover_status(false).with_cover_interrupt(false);
        self::refresh_interrupts(self);
    }
}
