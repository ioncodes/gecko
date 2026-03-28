pub mod instruction;
pub mod interpreter;
pub mod regs;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/dsp_lut.rs"));
}

use crate::flipper::dsp::instruction::Instruction;
use crate::gamecube::GameCube;
use crate::mmio::Mmio;
use crate::mmio::constants::DSP_BASE;
use crate::mmio::traits::{MmioAccess, MmioRegister, MmioRw};

pub struct Dsp {
    // Registers
    pub pc: u16,

    // IMEM = IRAM + IROM
    pub iram: Box<[u8; 0x1000]>, // 0x0000 - 0x0FFF
    pub irom: Box<[u8; 0x1000]>, // 0x8000 - 0x8FFF

    // DMEM = DRAM + COEF + IFX
    pub dram: Box<[u8; 0x1000]>, // 0x0000 - 0x0FFF
    pub coef: Box<[u8; 0x1000]>, // 0x1000 - 0x1FFF
    pub ifx: Box<[u8; 0x100]>,   // 0xFF00 - 0xFFFF

    // Auxiliary RAM (16 MB)
    pub aram: Box<[u8; 16 * 1024 * 1024]>,

    // I/O Registers
    pub csr: regs::ControlStatus,
    pub mailbox_to_dsp_hi: regs::MailboxToDspHi,
    pub mailbox_to_dsp_lo: regs::MailboxToDspLo,
    pub mailbox_to_cpu_hi: regs::MailboxToCpuHi,
    pub mailbox_to_cpu_lo: regs::MailboxToCpuLo,
    pub aram_info: regs::AramInfo,
    pub aram_refresh: regs::AramRefresh,
    pub aram_dma_mmio_addr: regs::AramDmaMmioAddr,
    pub aram_dma_aram_addr: regs::AramDmaAramAddr,
    pub aram_dma_control: regs::AramDmaControl,

    // Flags set by register write handlers, consumed by process_pending_dma
    pub pending_aram_dma: bool,
    pub pending_ucode_upload: bool,
}

impl Dsp {
    pub fn new() -> Self {
        let aram = unsafe { Box::<[u8; 16 * 1024 * 1024]>::new_zeroed().assume_init() };
        let iram = unsafe { Box::<[u8; 0x1000]>::new_zeroed().assume_init() };
        let irom = unsafe { Box::<[u8; 0x1000]>::new_zeroed().assume_init() };
        let dram = unsafe { Box::<[u8; 0x1000]>::new_zeroed().assume_init() };
        let coef = unsafe { Box::<[u8; 0x1000]>::new_zeroed().assume_init() };
        let ifx = unsafe { Box::<[u8; 0x100]>::new_zeroed().assume_init() };

        Dsp {
            pc: 0,
            iram,
            irom,
            dram,
            coef,
            ifx,
            aram,
            csr: regs::ControlStatus::default(),
            mailbox_to_dsp_hi: regs::MailboxToDspHi::from_raw(0),
            mailbox_to_dsp_lo: regs::MailboxToDspLo::from_raw(0),
            mailbox_to_cpu_hi: regs::MailboxToCpuHi::from_raw(0),
            mailbox_to_cpu_lo: regs::MailboxToCpuLo::from_raw(0),
            aram_info: regs::AramInfo::from_raw(0),
            aram_refresh: regs::AramRefresh::from_raw(0),
            aram_dma_mmio_addr: regs::AramDmaMmioAddr::from_raw(0),
            aram_dma_aram_addr: regs::AramDmaAramAddr::from_raw(0),
            aram_dma_control: regs::AramDmaControl::from_raw(0),
            pending_aram_dma: false,
            pending_ucode_upload: false,
        }
    }

    /// Called by the bus after every DSP MMIO write
    ///
    /// - If an ARAM DMA was triggered (write to DmaControl), execute the transfer and
    ///   assert ARINT in the CSR
    /// - If ucode upload (falling-edge) was detected (CSR bit 11: 1->0), copy from
    ///   main RAM at DmaMmioAddr into IRAM (like Dolphin), then HLE the mailbox response.
    #[inline]
    pub fn process_pending_dma(&mut self, mmio: &mut Mmio) {
        // Handle ARAM DMA
        if self.pending_aram_dma {
            self.pending_aram_dma = false;

            let mmio_addr = (self.aram_dma_mmio_addr.raw() & 0x3FFFFFFF) as usize;
            let aram_addr = self.aram_dma_aram_addr.raw() as usize;
            let count = self.aram_dma_control.count() as usize * 4;

            if self.aram_dma_control.direction() == regs::DmaDirection::AramToRam {
                // ARAM -> main RAM
                let src = self.aram[aram_addr..aram_addr + count].to_vec();
                mmio.ram[mmio_addr..mmio_addr + count].copy_from_slice(&src);
            } else {
                // main RAM -> ARAM
                let src = mmio.ram[mmio_addr..mmio_addr + count].to_vec();
                self.aram[aram_addr..aram_addr + count].copy_from_slice(&src);
            }

            tracing::debug!(
                mmio_addr = format!("{mmio_addr:08X}"),
                aram_addr = format!("{aram_addr:08X}"),
                count,
                direction = ?self.aram_dma_control.direction(),
                "ARAM DMA complete"
            );

            // Assert ARAM DMA complete interrupt in CSR
            self.csr = self.csr.with_ar_interrupt(true);
        }

        // Handle DSP ucode upload
        if self.pending_ucode_upload {
            self.pending_ucode_upload = false;

            const UCODE_ADDR: usize = 0x8100_0000;
            const UCODE_SIZE: usize = 1024;
            let src = mmio.virt_slice(UCODE_ADDR as u32, UCODE_SIZE);
            self.iram[..UCODE_SIZE].copy_from_slice(&src);

            tracing::debug!(
                mmio_addr = format!("{UCODE_ADDR:08X}"),
                count = UCODE_SIZE,
                "uploaded ucode from RAM to IRAM"
            );

            // HLE: Write expected response to mailbox
            self.mailbox_to_cpu_hi = regs::MailboxToCpuHi::from_raw(0x8071);
            self.mailbox_to_cpu_lo = regs::MailboxToCpuLo::from_raw(0xFEED);
        }
    }
}

impl GameCube {
    pub fn tick_dsp(&mut self) {
        if self.dsp.csr.reset() || self.dsp.csr.halt() {
            return;
        }

        let instr = Instruction::from_be_bytes(&self.dsp.iram[self.dsp.pc as usize..]);
        crate::flipper::dsp::lut::dispatch(self, instr);
        self.dsp.pc += crate::flipper::dsp::lut::instr_size(instr) as u16;
    }
}

impl MmioRw for Dsp {
    const BASE: u32 = DSP_BASE;
    const NAME: &'static str = "DSP";

    crate::impl_mmio_dispatch!(
        regs::ControlStatus,
        regs::MailboxToDspHi,
        regs::MailboxToDspLo,
        regs::MailboxToCpuHi,
        regs::MailboxToCpuLo,
        regs::AramInfo,
        regs::AramRefresh,
        regs::AramDmaMmioAddr,
        regs::AramDmaAramAddr,
        regs::AramDmaControl,
    );
}

impl Dsp {
    pub fn interrupt_active(&self) -> bool {
        (self.csr.ai_interrupt() && self.csr.ai_interrupt_mask())
            || (self.csr.ar_interrupt() && self.csr.ar_interrupt_mask())
            || (self.csr.dsp_interrupt() && self.csr.dsp_interrupt_mask())
    }
}

impl crate::gamecube::GameCube {
    pub fn check_dsp_interrupts(&mut self) {
        if self.dsp.interrupt_active() {
            self.pi.assert_interrupt(crate::flipper::pi::InterruptFlag::Dsp);
        } else {
            self.pi.clear_interrupt(crate::flipper::pi::InterruptFlag::Dsp);
        }
    }
}
