use crate::flipper::{ai, dsp};
use crate::gamecube::GameCube;
use crate::mmio::traits::{MmioAccess, WriteMask};
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

impl MmioAccess<GameCube> for ControlStatus {
    fn read(gc: &mut GameCube) -> Self {
        gc.dsp.csr
    }

    fn write(self, gc: &mut GameCube, _: WriteMask) {
        tracing::trace!("CSR write: {:016b} (prev: {:016b})", self.raw(), gc.dsp.csr.raw());

        let mut csr = gc.dsp.csr;

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

        // reset vector falling edge (bit 11: 1->0) triggers DMA of stub from main RAM into IRAM
        // and resets the PC to 0x0000 (IRAM) so the uploaded stub executes
        let trigger_ucode_upload =
            gc.dsp.csr.reset_vector() == ResetVector::High && self.reset_vector() == ResetVector::Low;
        if trigger_ucode_upload {
            tracing::info!("ucode upload, PC -> 0x0000 (IRAM)");
            gc.dsp.registers.pc = 0x0000;
        }

        // On reset, set PC to the address indicated by the reset vector
        if self.reset() {
            let addr = self.reset_vector().address();
            tracing::debug!(reset_vector = ?self.reset_vector(), pc = format!("{addr:04X}"), "DSP reset, executing from reset vector");
            gc.dsp.registers.pc = addr;
        }
        csr = csr.with_reset(false);

        gc.dsp.csr = csr;

        // Side effects after CSR is committed
        if trigger_ucode_upload {
            gc.dsp.process_ucode_upload(&mut gc.mmio);
        }
        dsp::refresh_interrupts(gc);
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

impl MmioAccess<GameCube> for MailboxToDspHi {
    fn read(gc: &mut GameCube) -> Self {
        gc.dsp.mailbox_to_dsp_hi
    }

    fn write(self, gc: &mut GameCube, _: WriteMask) {
        // CPU writing CMBH sets data bits (14:0), busy is preserved
        gc.dsp.mailbox_to_dsp_hi = self.with_busy(gc.dsp.mailbox_to_dsp_hi.busy());
    }
}

// 0xCC005002 2 [W] CPU-to-DSP Mailbox Low
// Writing sets CMBH.M (busy), signaling to the DSP that mail is ready.

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct MailboxToDspLo {}
crate::mmio_reg!(MailboxToDspLo: u16 @ 0xCC005002);

impl MmioAccess<GameCube> for MailboxToDspLo {
    fn read(gc: &mut GameCube) -> Self {
        gc.dsp.mailbox_to_dsp_lo
    }

    fn write(self, gc: &mut GameCube, _: WriteMask) {
        gc.dsp.mailbox_to_dsp_lo = self;
        gc.dsp.mailbox_to_dsp_hi = gc.dsp.mailbox_to_dsp_hi.with_busy(true);
        tracing::debug!(
            hi = format!("{:04X}", gc.dsp.mailbox_to_dsp_hi.raw()),
            lo = format!("{:04X}", self.raw()),
            "CPU->DSP mailbox"
        );
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

impl MmioAccess<GameCube> for MailboxToCpuHi {
    fn read(gc: &mut GameCube) -> Self {
        gc.dsp.mailbox_to_cpu_hi
    }

    fn write(self, _gc: &mut GameCube, _: WriteMask) {
        // Read-only from CPU side
    }
}

// 0xCC005006 2 [R] DSP-to-CPU Mailbox Low (DMBL)
// Reading clears DMBH.M (bit 15), indicating the CPU has consumed the mail.

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct MailboxToCpuLo {}
crate::mmio_reg!(MailboxToCpuLo: u16 @ 0xCC005006);

impl MmioAccess<GameCube> for MailboxToCpuLo {
    fn read(gc: &mut GameCube) -> Self {
        let val = gc.dsp.mailbox_to_cpu_lo;
        // Reading DMBL clears DMBH.M (bit 15), signaling the CPU has consumed the mail
        gc.dsp.mailbox_to_cpu_hi.set_busy(false);
        val
    }

    fn write(self, _gc: &mut GameCube, _: WriteMask) {
        // Read-only from CPU side
    }
}

// 0xCC005012 2 [R/W] AR_INFO - ARAM Info/Size

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramInfo {}
crate::mmio_reg!(AramInfo: u16 @ 0xCC005012);
crate::mmio_default_access!(AramInfo => GameCube.dsp.aram_info);

// 0xCC005016 2 [R/W] AR_MODE - ARAM Mode

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramMode {
    #[bits(0)]
    pub status: bool,
}
crate::mmio_reg!(AramMode: u16 @ 0xCC005016);

impl MmioAccess<GameCube> for AramMode {
    fn read(gc: &mut GameCube) -> Self {
        gc.dsp.aram_mode.with_status(true)
    }

    fn write(self, gc: &mut GameCube, _: WriteMask) {
        gc.dsp.aram_mode = self;
    }
}

// 0xCC00501A 2 [R/W] AR_REFRESH - ARAM Refresh

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramRefresh {}
crate::mmio_reg!(AramRefresh: u16 @ 0xCC00501A);
crate::mmio_default_access!(AramRefresh => GameCube.dsp.aram_refresh);

// 0xCC005020 4 [R/W] ARAM DMA MMIO Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramDmaMmioAddr {}
crate::mmio_reg!(AramDmaMmioAddr: u32 @ 0xCC005020);
crate::mmio_default_access!(AramDmaMmioAddr => GameCube.dsp.aram_dma_mmio_addr);

// 0xCC005024 4 [R/W] ARAM DMA ARAM Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AramDmaAramAddr {}
crate::mmio_reg!(AramDmaAramAddr: u32 @ 0xCC005024);
crate::mmio_default_access!(AramDmaAramAddr => GameCube.dsp.aram_dma_aram_addr);

// 0xCC005030 4 [W] Audio DMA Start Address (High + Low)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AudioDmaStartAddr {}
crate::mmio_reg!(AudioDmaStartAddr: u32 @ 0xCC005030);
crate::mmio_default_access!(AudioDmaStartAddr => GameCube.dsp.audio_dma_start_addr);

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

impl MmioAccess<GameCube> for AudioDmaControl {
    fn read(gc: &mut GameCube) -> Self {
        gc.dsp.audio_dma_control
    }

    fn write(self, gc: &mut GameCube, _: WriteMask) {
        gc.dsp.audio_dma_control = self;
        ai::start_audio_dma(gc);
        dsp::refresh_interrupts(gc);
    }
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

impl MmioAccess<GameCube> for AramDmaControl {
    fn read(gc: &mut GameCube) -> Self {
        gc.dsp.aram_dma_control
    }

    fn write(self, gc: &mut GameCube, _: WriteMask) {
        gc.dsp.aram_dma_control = self;

        const ARAM_DMA_DELAY: u64 = 10_000;
        gc.scheduler.schedule_in(ARAM_DMA_DELAY, |gc| {
            gc.dsp.process_aram_dma(&mut gc.mmio);
            dsp::refresh_interrupts(gc);
        });
    }
}
