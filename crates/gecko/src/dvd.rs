pub mod regs;

use image::Executable;

use crate::gamecube::GameCube;
use crate::mmio::constants::DI_BASE;
use crate::mmio::traits::{MmioAccess, MmioRegister, MmioRw};

const DICMDBUF0: u32 = DI_BASE + 0x08;
const DICMDBUF1: u32 = DI_BASE + 0x0C;
const DICMDBUF2: u32 = DI_BASE + 0x10;
const DIIMMBUF: u32 = DI_BASE + 0x20;

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
    pub transfer_started: bool,
    pub dvd: Option<image::dvd::Dvd>,
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
            transfer_started: false,
            dvd: None,
        }
    }

    pub fn interrupt_active(&self) -> bool {
        (self.status.break_complete() && self.status.break_complete_mask())
            || (self.status.device_error() && self.status.device_error_mask())
            || (self.status.transfer_complete() && self.status.transfer_complete_mask())
            || (self.cover.cover_interrupt() && self.cover.cover_interrupt_mask())
    }

    #[inline(always)]
    fn resolve_command(&mut self) -> Option<Command> {
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

impl GameCube {
    #[inline(always)]
    fn process_dvd_command(&mut self, cmd: Command, dvd: &image::dvd::Dvd) {
        match cmd {
            Command::DriveInfo => {
                let dst = self.di.dma_address.address();
                let buffer = self.mmio.phys_slice_mut(dst, 0x20);
                buffer.copy_from_slice(&[0x69; 0x20]); // TODO: Drive Info?
            }
            Command::ReadSectorData => {
                let src = self.di.cmdbuf1 << 2;
                let dst = self.di.dma_address.address();
                let len = self.di.cmdbuf2 as usize;
                assert!(len == self.di.dma_length.length() as usize, "DMA length mismatch");

                let buffer = self.mmio.phys_slice_mut(dst, len);
                buffer.copy_from_slice(&dvd.data()[src as usize..src as usize + len]);

                tracing::debug!(
                    src = format!("{:08X}", src),
                    dst = format!("{:08X}", dst),
                    len = format!("{:08X}", len),
                    "ReadSectorData command"
                );
            }
            Command::ReadDiskId => {
                let src = self.di.cmdbuf1;
                let dst = self.di.dma_address.address();
                let len = self.di.dma_length.length() as usize;

                let buffer = self.mmio.phys_slice_mut(dst, len);
                buffer.copy_from_slice(&dvd.data()[..len]);

                tracing::debug!(
                    src = format!("{:08X}", src),
                    dst = format!("{:08X}", dst),
                    len = format!("{:08X}", len),
                    "ReadDiskId command"
                );
            }
            Command::AudioToggle(enable) => {
                tracing::warn!(enable, "AudioToggle stubbed");
                self.di.immbuf = 0;
            }
        }
    }

    #[inline(always)]
    pub fn start_dvd_transfer(&mut self) {
        if !self.di.transfer_started {
            return;
        }

        let Some(dvd) = self.di.dvd.take() else {
            // TODO: Setting this causes unrecoverable error when there is no DVD
            //self.di.status.set_device_error(true);
            self.di.control.set_tstart(false);
            self.di.transfer_started = false;
            return;
        };

        if let Some(cmd) = self.di.resolve_command() {
            self.process_dvd_command(cmd, &dvd);
        }

        self.di.dvd = Some(dvd);
        self.di.transfer_started = false;

        // The interrupt must not fire until some time!!
        // This is important as it will break the IPL if we immediately raise it...
        // When DVDReadDiskID issues the DI command, it returns to BS2_Tick which updates
        // the internal state machine and then calls into restore_irq. The code assumes
        // that the interrupt wont fire until after it has returned and updated the state.
        // Else it will get trapped inside restore_irq, right after mtmsr, executing in a loop
        // of the DVD dispatch handler which in turn re-issues the same command...
        const DI_TRANSFER_DELAY: u64 = 10_000; // Based off of vxpm and hazel
        self.scheduler
            .schedule_in(DI_TRANSFER_DELAY, crate::scheduler::EventKind::DiTransferComplete);
    }

    pub fn complete_dvd_transfer(&mut self) {
        self.di.control.set_tstart(false);
        // DMA length tracks the progress of the transfer, so when it hits 0, the transfer is complete
        // On failure, this would denote how many bytes were not transfered, but we close our eyes
        // and just hope nothing depends on that!
        self.di.dma_length = regs::DiDmaLengthRegister::from_raw(0);
        self.di.status.set_transfer_complete(true);
        self.check_di_interrupts();
    }

    pub fn insert_dvd(&mut self, dvd: image::dvd::Dvd) {
        let name = String::from_utf8_lossy(&dvd.header.game_name);
        let name = name.trim_end_matches('\0');
        tracing::info!("DVD inserted: {}", name);
        self.di.dvd = Some(dvd);
        self.close_cover();
    }
}

impl MmioRw for DvdInterface {
    const BASE: u32 = DI_BASE;
    const NAME: &'static str = "DVD";

    fn read_raw(&mut self, addr: u32, access_size: u32) -> Option<u32> {
        if <regs::DiStatusRegister as MmioRegister>::fits(addr, access_size) {
            return Some(<regs::DiStatusRegister as MmioAccess<Self>>::read_at(
                self,
                addr,
                access_size,
            ));
        }
        if <regs::DiCoverRegister as MmioRegister>::fits(addr, access_size) {
            return Some(<regs::DiCoverRegister as MmioAccess<Self>>::read_at(
                self,
                addr,
                access_size,
            ));
        }
        if <regs::DiDmaAddressRegister as MmioRegister>::fits(addr, access_size) {
            return Some(<regs::DiDmaAddressRegister as MmioAccess<Self>>::read_at(
                self,
                addr,
                access_size,
            ));
        }
        if <regs::DiDmaLengthRegister as MmioRegister>::fits(addr, access_size) {
            return Some(<regs::DiDmaLengthRegister as MmioAccess<Self>>::read_at(
                self,
                addr,
                access_size,
            ));
        }
        if <regs::DiControlRegister as MmioRegister>::fits(addr, access_size) {
            return Some(<regs::DiControlRegister as MmioAccess<Self>>::read_at(
                self,
                addr,
                access_size,
            ));
        }
        if <regs::DiConfigurationRegister as MmioRegister>::fits(addr, access_size) {
            return Some(<regs::DiConfigurationRegister as MmioAccess<Self>>::read_at(
                self,
                addr,
                access_size,
            ));
        }

        match addr {
            DICMDBUF0 if access_size == 4 => Some(self.cmdbuf0),
            DICMDBUF1 if access_size == 4 => Some(self.cmdbuf1),
            DICMDBUF2 if access_size == 4 => Some(self.cmdbuf2),
            DIIMMBUF if access_size == 4 => Some(self.immbuf),
            _ => None,
        }
    }

    fn write_raw(&mut self, addr: u32, access_size: u32, val: u32) -> bool {
        if <regs::DiStatusRegister as MmioRegister>::fits(addr, access_size) {
            <regs::DiStatusRegister as MmioAccess<Self>>::write_at(self, addr, access_size, val);
            return true;
        }
        if <regs::DiCoverRegister as MmioRegister>::fits(addr, access_size) {
            <regs::DiCoverRegister as MmioAccess<Self>>::write_at(self, addr, access_size, val);
            return true;
        }
        if <regs::DiDmaAddressRegister as MmioRegister>::fits(addr, access_size) {
            <regs::DiDmaAddressRegister as MmioAccess<Self>>::write_at(self, addr, access_size, val);
            return true;
        }
        if <regs::DiDmaLengthRegister as MmioRegister>::fits(addr, access_size) {
            <regs::DiDmaLengthRegister as MmioAccess<Self>>::write_at(self, addr, access_size, val);
            return true;
        }
        if <regs::DiControlRegister as MmioRegister>::fits(addr, access_size) {
            <regs::DiControlRegister as MmioAccess<Self>>::write_at(self, addr, access_size, val);
            return true;
        }
        if <regs::DiConfigurationRegister as MmioRegister>::fits(addr, access_size) {
            <regs::DiConfigurationRegister as MmioAccess<Self>>::write_at(self, addr, access_size, val);
            return true;
        }

        match addr {
            DICMDBUF0 if access_size == 4 => {
                self.cmdbuf0 = val;
                tracing::debug!(
                    cmd = format!("{:02X}", val >> 24),
                    sub1 = format!("{:02X}", (val >> 16) & 0xFF),
                    sub2 = format!("{:04X}", val & 0xFFFF),
                    "DICMDBUF0 write"
                );
            }
            DICMDBUF1 if access_size == 4 => {
                self.cmdbuf1 = val;
                tracing::debug!(val = format!("{val:08X}"), "DICMDBUF1 write");
            }
            DICMDBUF2 if access_size == 4 => {
                self.cmdbuf2 = val;
                tracing::debug!(val = format!("{val:08X}"), "DICMDBUF2 write");
            }
            DIIMMBUF if access_size == 4 => {
                self.immbuf = val;
            }
            _ => return false,
        }
        true
    }
}

impl crate::gamecube::GameCube {
    pub fn check_di_interrupts(&mut self) {
        if self.di.interrupt_active() {
            self.pi.assert_interrupt(crate::flipper::pi::InterruptFlag::Di);
        } else {
            self.pi.clear_interrupt(crate::flipper::pi::InterruptFlag::Di);
        }
    }

    pub fn open_cover(&mut self) {
        tracing::debug!("DVD drive cover opened");
        self.di.cover = self.di.cover.with_cover_interrupt(true).with_cover_status(true);
        self.check_di_interrupts();
    }

    pub fn close_cover(&mut self) {
        tracing::debug!("DVD drive cover closed");
        self.di.cover = self.di.cover.with_cover_status(false).with_cover_interrupt(false);
        self.check_di_interrupts();
    }
}
