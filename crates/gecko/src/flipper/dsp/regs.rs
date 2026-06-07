use crate::flipper::{ai, dsp};
use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::scheduler;
use crate::system::{System, SystemId};
use chapa::BitEnum;

// 0xCC00500A 2 [R/W] CSR - DSP Control/Status Register

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum ResetVector {
    Low = 0,
    High = 1,
}

impl ResetVector {
    pub fn address(&self) -> u16 {
        match self {
            ResetVector::High => 0x8000, // IROM
            ResetVector::Low => 0x0000,  // IRAM
        }
    }
}

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ControlStatus {
    #[bits(0, alias = "res")]
    pub reset: bool,

    #[bits(1, alias = "piint")]
    pub pi_interrupt: bool,

    #[bits(2)]
    pub halt: bool,

    #[bits(3, alias = "aiint")]
    pub ai_interrupt: bool,

    #[bits(4, alias = "aiintmask")]
    pub ai_interrupt_mask: bool,

    #[bits(5, alias = "arint")]
    pub ar_interrupt: bool,

    #[bits(6, alias = "arintmask")]
    pub ar_interrupt_mask: bool,

    #[bits(7, alias = "dspint")]
    pub dsp_interrupt: bool,

    #[bits(8, alias = "dspintmask")]
    pub dsp_interrupt_mask: bool,

    #[bits(9, alias = "dspdma")]
    pub dma_status: bool,

    #[bits(11)]
    pub reset_vector: ResetVector,
}
crate::mmio_reg!(ControlStatus: u16 @ 0xCC00500A);

impl Default for ControlStatus {
    fn default() -> Self {
        ControlStatus::new().with_halt(true)
    }
}

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for ControlStatus {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.csr
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        tracing::trace!("CSR write: {:016b} (prev: {:016b})", self.raw(), sys.dsp.csr.raw());

        let was_active = !sys.dsp.csr.halt() && !sys.dsp.csr.reset();
        let mut csr = sys.dsp.csr;

        if self.ai_interrupt() {
            csr = csr.with_ai_interrupt(false);
        }
        if self.ar_interrupt() {
            csr = csr.with_ar_interrupt(false);
        }
        if self.dsp_interrupt() {
            csr = csr.with_dsp_interrupt(false);
        }

        csr = csr
            .with_pi_interrupt(self.pi_interrupt())
            .with_halt(self.halt())
            .with_ai_interrupt_mask(self.ai_interrupt_mask())
            .with_ar_interrupt_mask(self.ar_interrupt_mask())
            .with_dsp_interrupt_mask(self.dsp_interrupt_mask())
            .with_dma_status(self.dma_status())
            .with_reset_vector(self.reset_vector());

        // On reset, set PC to the address indicated by the reset vector. After
        // reset the DSP executes from IROM (0x8000) when reset_vector=High; the
        // SDK then drives the boot mailbox protocol with the IROM, which in
        // turn DMAs the AX firmware into IRAM and jumps to it.
        if self.reset() {
            let addr = self.reset_vector().address();
            tracing::debug!(reset_vector = ?self.reset_vector(), pc = format!("{addr:04X}"), "DSP reset, executing from reset vector");
            sys.dsp.registers.pc = addr;
        }
        csr = csr.with_reset(false);

        sys.dsp.csr = csr;

        let now_active = !sys.dsp.csr.halt() && !sys.dsp.csr.reset();
        match (was_active, now_active) {
            (false, true) => {
                sys.dsp.scheduler_suspended = false;
                sys.scheduler.schedule_in(
                    scheduler::dsp_batch_interval(SYSTEM),
                    scheduler::dsp_batch_handler::<SYSTEM>,
                );
            }
            (true, false) => {
                sys.dsp.scheduler_suspended = false;
                sys.scheduler.cancel(scheduler::dsp_batch_handler::<SYSTEM>);
            }
            _ => {}
        }

        dsp::refresh_interrupts(sys);
    }
}

// 0xCC005000 2 [W] CPU-to-DSP Mailbox High

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct MailboxToDspHi {
    #[bits(0..=14)]
    pub data: u16,

    #[bits(15)]
    pub busy: bool,
}
crate::mmio_reg!(MailboxToDspHi: u16 @ 0xCC005000);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for MailboxToDspHi {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.mailbox_to_dsp_hi
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        // CPU writing CMBH sets data bits (14:0), busy is preserved
        sys.dsp.mailbox_to_dsp_hi = self.with_busy(sys.dsp.mailbox_to_dsp_hi.busy());
    }
}

// 0xCC005002 2 [W] CPU-to-DSP Mailbox Low
// Writing sets CMBH.M (busy), signaling to the DSP that mail is ready.

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct MailboxToDspLo {}
crate::mmio_reg!(MailboxToDspLo: u16 @ 0xCC005002);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for MailboxToDspLo {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.mailbox_to_dsp_lo
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.dsp.mailbox_to_dsp_lo = self;
        sys.dsp.mailbox_to_dsp_hi = sys.dsp.mailbox_to_dsp_hi.with_busy(true);
        tracing::debug!(
            hi = format!("{:04X}", sys.dsp.mailbox_to_dsp_hi.raw()),
            lo = format!("{:04X}", self.raw()),
            "CPU->DSP mailbox"
        );

        // Synchronous DSP drain: let the DSP consume the command and
        // answer the mailbox before the CPU returns. AX ucode reads the
        // mailbox, renders 96 samples and DMAs them to the AID "ping pong?"
        // buffer all within one mailbox roundtirp. Without this the
        // periodic batch handler can run DSP a few hundred us late, so
        // AID consumes the buffer with the prior cycle's contents at the
        // start of each render before the new contents land.
        // Based off of FFCC, thanks SpinningCube, zayd, JustinCase
        sys.drain_dsp_synchronous(64 * 1024);
        crate::flipper::dsp::wake_dsp_scheduler::<SYSTEM>(sys);
    }
}

// 0xCC005004 2 [R] DSP-to-CPU Mailbox High (DMBH)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct MailboxToCpuHi {
    #[bits(0..=14)]
    pub data: u16,

    #[bits(15)]
    pub busy: bool,
}
crate::mmio_reg!(MailboxToCpuHi: u16 @ 0xCC005004);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for MailboxToCpuHi {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.mailbox_to_cpu_hi
    }

    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        // Read-only from CPU side
    }
}

// 0xCC005006 2 [R] DSP-to-CPU Mailbox Low (DMBL)
// Reading clears DMBH.M (bit 15), indicating the CPU has consumed the mail.

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct MailboxToCpuLo {}
crate::mmio_reg!(MailboxToCpuLo: u16 @ 0xCC005006);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for MailboxToCpuLo {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        let val = sys.dsp.mailbox_to_cpu_lo;
        // Reading DMBL clears DMBH.M (bit 15), signaling the CPU has consumed the mail
        sys.dsp.mailbox_to_cpu_hi.set_busy(false);
        crate::flipper::dsp::wake_dsp_scheduler::<SYSTEM>(sys);
        val
    }

    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        // Read-only from CPU side
    }
}

// 0xCC005012 2 [R/W] AR_INFO - ARAM Info/Size

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramInfo {}
crate::mmio_reg!(AramInfo: u16 @ 0xCC005012);
crate::mmio_default_access!(AramInfo => System.dsp.aram_info);

// 0xCC005016 2 [R/W] AR_MODE - ARAM Mode

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramMode {
    #[bits(0)]
    pub status: bool,
}
crate::mmio_reg!(AramMode: u16 @ 0xCC005016);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AramMode {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.aram_mode.with_status(true)
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.dsp.aram_mode = self;
    }
}

// 0xCC00501A 2 [R/W] AR_REFRESH - ARAM Refresh

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramRefresh {}
crate::mmio_reg!(AramRefresh: u16 @ 0xCC00501A);
crate::mmio_default_access!(AramRefresh => System.dsp.aram_refresh);

// 0xCC005020 4 [R/W] ARAM DMA MMIO Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramDmaMmioAddr {}
crate::mmio_reg!(AramDmaMmioAddr: u32 @ 0xCC005020);
crate::mmio_default_access!(AramDmaMmioAddr => System.dsp.aram_dma_mmio_addr);

// 0xCC005024 4 [R/W] ARAM DMA ARAM Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramDmaAramAddr {}
crate::mmio_reg!(AramDmaAramAddr: u32 @ 0xCC005024);
crate::mmio_default_access!(AramDmaAramAddr => System.dsp.aram_dma_aram_addr);

// 0xCC005030 4 [W] Audio DMA Start Address (High + Low)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AudioDmaStartAddr {}
crate::mmio_reg!(AudioDmaStartAddr: u32 @ 0xCC005030);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AudioDmaStartAddr {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.audio_dma_start_addr
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.dsp.audio_dma_start_addr = self;
    }
}

// 0xCC005036 2 [W] Audio DMA Control/Length

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AudioDmaControl {
    #[bits(0..=14)]
    pub length: u16,

    #[bits(15)]
    pub play: bool,
}
crate::mmio_reg!(AudioDmaControl: u16 @ 0xCC005036);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AudioDmaControl {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.audio_dma_control
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let was_playing = sys.dsp.audio_dma_control.play();
        sys.dsp.audio_dma_control = self;
        match (was_playing, self.play()) {
            (false, true) => ai::start_audio_dma(sys),
            (true, false) => ai::stop_audio_dma(sys),
            _ => {}
        }
        dsp::refresh_interrupts(sys);
    }
}

// 0xCC00503A 2 [R] Audio DMA Blocks Remaining

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AudioDmaBlocksLeft {}
crate::mmio_reg!(AudioDmaBlocksLeft: u16 @ 0xCC00503A);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AudioDmaBlocksLeft {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        Self::from_raw(sys.ai.audio_dma_remaining_blocks.saturating_sub(1))
    }

    fn write(self, _: &mut System<SYSTEM>, _: WriteMask) {}
}

// 0xCC005028 4 [R/W] ARAM DMA Count/Control

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum DmaDirection {
    RamToAram = 0,
    AramToRam = 1,
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramDmaControl {
    #[bits(0..=30)]
    pub count: u32,

    #[bits(31)]
    pub direction: DmaDirection,
}
crate::mmio_reg!(AramDmaControl: u32 @ 0xCC005028);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AramDmaControl {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.dsp.aram_dma_control
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.dsp.aram_dma_control = self;

        const ARAM_DMA_DELAY_US: u64 = 20;
        sys.scheduler.schedule_in(
            crate::scheduler::microseconds_to_cycles(SYSTEM, ARAM_DMA_DELAY_US),
            |sys| {
                sys.dsp.process_aram_dma(&mut sys.mmio);
                dsp::refresh_interrupts(sys);
            },
        );
    }
}
