use crate::flipper::si;
use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::system::{System, SystemId};
use chapa::BitEnum;

// 0xCC006430  4  R/W  SIPOLL (SI Poll Register)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct SiPoll {
    #[bits(0..=3)]
    pub vbcpy: u8,

    #[bits(4..=7)]
    pub enable: u8,

    #[bits(8..=15)]
    pub y_times: u8,

    #[bits(16..=25)]
    pub x_lines: u16,
}
crate::mmio_reg!(SiPoll: u32 @ 0xCC006430);
crate::mmio_default_access!(SiPoll => System.si.poll);

// 0xCC006434  4  R/W  SICOMCSR (SI Communication Control Status Register)

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum Channel {
    Channel0 = 0,
    Channel1 = 1,
    Channel2 = 2,
    Channel3 = 3,
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct SiComcsr {
    #[bits(0)]
    pub tstart: bool,

    #[bits(1..=2)]
    pub channel: Channel,

    #[bits(6)]
    pub callback_enable: bool,

    #[bits(7)]
    pub command_enable: bool,

    #[bits(8..=14)]
    pub in_length: u8,

    #[bits(16..=22)]
    pub out_length: u8,

    #[bits(24)]
    pub channel_enable: bool,

    #[bits(25..=26)]
    pub channel_number: u8,

    #[bits(27)]
    pub rdst_interrupt_mask: bool,

    #[bits(28)]
    pub rdst_interrupt: bool,

    #[bits(29)]
    pub com_error: bool,

    #[bits(30)]
    pub tc_interrupt_mask: bool,

    #[bits(31)]
    pub tc_interrupt: bool,
}
crate::mmio_reg!(SiComcsr: u32 @ 0xCC006434);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for SiComcsr {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.si.comcsr
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let mut csr = sys.si.comcsr;

        if self.tc_interrupt() {
            csr = csr.with_tc_interrupt(false);
        }
        if self.rdst_interrupt() {
            csr = csr.with_rdst_interrupt(false);
        }

        csr = csr
            .with_tc_interrupt_mask(self.tc_interrupt_mask())
            .with_rdst_interrupt_mask(self.rdst_interrupt_mask())
            .with_command_enable(self.command_enable())
            .with_callback_enable(self.callback_enable())
            .with_channel(self.channel())
            .with_in_length(self.in_length())
            .with_out_length(self.out_length())
            .with_channel_enable(self.channel_enable())
            .with_channel_number(self.channel_number());

        sys.si.comcsr = csr;

        // Process SI buffer transfer when TSTART is written
        if self.tstart() {
            sys.si.run_si_buffer();
        }

        si::refresh_interrupts(sys);
    }
}

// 0xCC006438  4  R/W  SISR (SI Status Register)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct SiStatusRegister {
    // Channel 3
    #[bits(0)]
    pub unrun3: bool,
    #[bits(1)]
    pub ovrun3: bool,
    #[bits(2)]
    pub coll3: bool,
    #[bits(3)]
    pub norep3: bool,
    #[bits(4)]
    pub wrst3: bool,
    #[bits(5)]
    pub rdst3: bool,

    // Channel 2
    #[bits(8)]
    pub unrun2: bool,
    #[bits(9)]
    pub ovrun2: bool,
    #[bits(10)]
    pub coll2: bool,
    #[bits(11)]
    pub norep2: bool,
    #[bits(12)]
    pub wrst2: bool,
    #[bits(13)]
    pub rdst2: bool,

    // Channel 1
    #[bits(16)]
    pub unrun1: bool,
    #[bits(17)]
    pub ovrun1: bool,
    #[bits(18)]
    pub coll1: bool,
    #[bits(19)]
    pub norep1: bool,
    #[bits(20)]
    pub wrst1: bool,
    #[bits(21)]
    pub rdst1: bool,

    // Channel 0
    #[bits(24)]
    pub unrun0: bool,
    #[bits(25)]
    pub ovrun0: bool,
    #[bits(26)]
    pub coll0: bool,
    #[bits(27)]
    pub norep0: bool,
    #[bits(28)]
    pub wrst0: bool,
    #[bits(29)]
    pub rdst0: bool,

    #[bits(31)]
    pub wr: bool,
}
crate::mmio_reg!(SiStatusRegister: u32 @ 0xCC006438);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for SiStatusRegister {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.si.status
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let mut status = sys.si.status;

        if self.norep0() {
            status = status.with_norep0(false);
        }
        if self.coll0() {
            status = status.with_coll0(false);
        }
        if self.ovrun0() {
            status = status.with_ovrun0(false);
        }
        if self.unrun0() {
            status = status.with_unrun0(false);
        }

        if self.norep1() {
            status = status.with_norep1(false);
        }
        if self.coll1() {
            status = status.with_coll1(false);
        }
        if self.ovrun1() {
            status = status.with_ovrun1(false);
        }
        if self.unrun1() {
            status = status.with_unrun1(false);
        }

        if self.norep2() {
            status = status.with_norep2(false);
        }
        if self.coll2() {
            status = status.with_coll2(false);
        }
        if self.ovrun2() {
            status = status.with_ovrun2(false);
        }
        if self.unrun2() {
            status = status.with_unrun2(false);
        }

        if self.norep3() {
            status = status.with_norep3(false);
        }
        if self.coll3() {
            status = status.with_coll3(false);
        }
        if self.ovrun3() {
            status = status.with_ovrun3(false);
        }
        if self.unrun3() {
            status = status.with_unrun3(false);
        }

        if self.wr() {
            sys.si.status = status;
            sys.si.send_channel_commands();
            status = sys.si.status;
            status = status
                .with_wr(false)
                .with_wrst0(false)
                .with_wrst1(false)
                .with_wrst2(false)
                .with_wrst3(false);
        }

        sys.si.status = status;
        si::refresh_interrupts(sys);
    }
}
