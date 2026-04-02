pub mod condition;
pub mod core;
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
    pub registers: core::Registers,

    // IMEM = IRAM + IROM
    pub iram: Box<[u8; 0x2000]>, // 0x0000 - 0x0FFF
    pub irom: Box<[u8; 0x2000]>, // 0x8000 - 0x8FFF

    // DMEM = DRAM + COEF + IFX
    pub dram: Box<[u8; 0x2000]>, // 0x0000 - 0x0FFF (0x1000 words)
    pub coef: Box<[u8; 0x2000]>, // 0x1000 - 0x1FFF (0x1000 words)
    pub ifx: Box<[u8; 0x200]>,   // 0xFF00 - 0xFFFF (0x100 words)

    // Auxiliary RAM (16 MB)
    pub aram: Box<[u8; 16 * 1024 * 1024]>,

    // I/O Registers
    pub csr: regs::ControlStatus,
    pub mailbox_to_dsp_hi: regs::MailboxToDspHi,
    pub mailbox_to_dsp_lo: regs::MailboxToDspLo,
    pub mailbox_to_cpu_hi: regs::MailboxToCpuHi,
    pub mailbox_to_cpu_lo: regs::MailboxToCpuLo,
    pub aram_info: regs::AramInfo,
    pub aram_mode: regs::AramMode,
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
        let iram = unsafe { Box::<[u8; 0x2000]>::new_zeroed().assume_init() };
        let irom = unsafe { Box::<[u8; 0x2000]>::new_zeroed().assume_init() };
        let dram = unsafe { Box::<[u8; 0x2000]>::new_zeroed().assume_init() };
        let coef = unsafe { Box::<[u8; 0x2000]>::new_zeroed().assume_init() };
        let ifx = unsafe { Box::<[u8; 0x200]>::new_zeroed().assume_init() };

        Dsp {
            registers: core::Registers::default(),
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
            aram_mode: regs::AramMode::from_raw(0),
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
    /// - If ucode upload was triggered (CSR bit 11 falling edge), DMA 1024 bytes from
    ///   main RAM into IRAM (we LLE, but we could HLE the mailbox response)
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

            tracing::info!(
                mmio_addr = format!("{UCODE_ADDR:08X}"),
                count = UCODE_SIZE,
                "DSP stub uploaded from RAM to IRAM, executing IRAM"
            );

            // HLE: Write expected response to mailbox
            // self.mailbox_to_cpu_hi = regs::MailboxToCpuHi::from_raw(0x8071);
            // self.mailbox_to_cpu_lo = regs::MailboxToCpuLo::from_raw(0xFEED);
        }
    }
}

impl GameCube {
    #[inline(always)]
    pub fn tick_dsp(&mut self) {
        if self.dsp.csr.reset() || self.dsp.csr.halt() {
            return;
        }

        let pc = self.dsp.registers.pc as usize;
        let w0 = self.dsp.read_imem(pc as u16);
        let w1 = self.dsp.read_imem(pc as u16 + 1);
        let buf = [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8];
        let instr = Instruction::from_be_bytes(&buf);
        self.dsp.registers.cia = self.dsp.registers.pc;
        self.dsp.registers.nia = self
            .dsp
            .registers
            .cia
            .wrapping_add(crate::flipper::dsp::lut::instr_size(instr) as u16);

        crate::flipper::dsp::lut::dispatch(self, instr);

        // Dispatch extension opcode if present
        if let Some(ext) = instr.ext_opcode() {
            let ext = instruction::GcDspExt(ext);
            crate::flipper::dsp::lut::dispatch_gc_dsp_ext(self, ext);
        }

        // Check if we've reached the end of a loop stack
        let is_end_of_loop = self.dsp.registers.nia == self.dsp.registers.loop_addr.top();
        if !self.dsp.registers.loop_addr.is_empty() && is_end_of_loop {
            let counter = self.dsp.registers.loop_counter.top().wrapping_sub(1);
            if counter != 0 {
                self.dsp.registers.loop_counter.set_top(counter);
                self.dsp.registers.nia = self.dsp.registers.call_stack.top();
            } else {
                self.dsp.registers.loop_counter.pop();
                self.dsp.registers.loop_addr.pop();
                self.dsp.registers.call_stack.pop();
            }
        }

        self.dsp.registers.pc = self.dsp.registers.nia;
        self.check_dsp_interrupts();
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
        regs::AramMode,
        regs::AramRefresh,
        regs::AramDmaMmioAddr,
        regs::AramDmaAramAddr,
        regs::AramDmaControl,
    );
}

impl Dsp {
    /// Read a 16-bit word from instruction memory (IRAM 0x0000-0x0FFF, IROM 0x8000-0x8FFF).
    pub fn read_imem(&self, addr: u16) -> u16 {
        match addr {
            0x0000..0x1000 => read_word(&*self.iram, addr),
            0x8000..0x9000 => read_word(&*self.irom, addr - 0x8000),
            _ => 0,
        }
    }

    /// Read a 16-bit word from data memory (DRAM 0x0000-0x0FFF, COEF 0x1000-0x1FFF, IFX 0xFF00-0xFFFF).
    pub fn read_dmem(&mut self, addr: u16) -> u16 {
        match addr {
            0x0000..0x1000 => read_word(&*self.dram, addr),
            0x1000..0x2000 => read_word(&*self.coef, addr - 0x1000),
            0xFF00..=0xFFFF => self.read_ifx(addr),
            _ => 0,
        }
    }

    /// Read a 16-bit word from IFX register space, with mailbox side-effects.
    pub fn read_ifx(&mut self, addr: u16) -> u16 {
        match addr {
            // CMBH (CPU Mailbox High): reading returns data + M bit;
            // clears M (busy) so the CPU sees the mailbox as free.
            0xFFFE => {
                let val = self.mailbox_to_dsp_hi.raw();
                self.mailbox_to_dsp_hi.set_busy(false);
                val
            }
            // CMBL (CPU Mailbox Low): reading clears CMBH.M (busy)
            0xFFFF => {
                self.mailbox_to_dsp_hi.set_busy(false);
                self.mailbox_to_dsp_lo.raw()
            }
            // DMBH (DSP Mailbox High): DSP reads back what it wrote
            0xFFFC => self.mailbox_to_cpu_hi.raw(),
            // DMBL (DSP Mailbox Low): DSP reads back what it wrote (no side effects)
            0xFFFD => self.mailbox_to_cpu_lo.raw(),
            _ => read_word(&*self.ifx, addr - 0xFF00),
        }
    }

    /// Write a 16-bit word to IFX register space, with mailbox side-effects.
    pub fn write_ifx(&mut self, addr: u16, value: u16) {
        match addr {
            // DMBH (DSP Mailbox High): store data bits (14:0), clear M bit
            // M will be set when DMBL is written (matching Dolphin behavior)
            0xFFFC => {
                self.mailbox_to_cpu_hi = regs::MailboxToCpuHi::from_raw(value & 0x7FFF);
            }
            // DMBL (DSP Mailbox Low): writing sets DMBH.M
            // Interrupt is NOT raised here; the ucode must explicitly write DIRQ (0xFFFB)
            0xFFFD => {
                self.mailbox_to_cpu_lo = regs::MailboxToCpuLo::from_raw(value);
                self.mailbox_to_cpu_hi.set_busy(true);
            }
            // DIRQ: DSP explicitly raises interrupt to CPU
            0xFFFB => {
                if value & 1 != 0 {
                    self.csr.set_dsp_interrupt(true);
                }
            }
            // CMBH/CMBL are read-only from DSP side
            0xFFFE | 0xFFFF => {}
            _ => write_word(&mut *self.ifx, addr - 0xFF00, value),
        }
    }

    /// Write a 16-bit word to data memory (DRAM 0x0000-0x0FFF, IFX 0xFF00-0xFFFF).
    pub fn write_dmem(&mut self, addr: u16, value: u16) {
        match addr {
            0x0000..0x1000 => write_word(&mut *self.dram, addr, value),
            0xFF00..=0xFFFF => self.write_ifx(addr, value),
            _ => {}
        }
    }

    /// Load a binary file into IROM.
    pub fn load_irom(&mut self, data: &[u8]) {
        let len = data.len().min(self.irom.len());
        self.irom[..len].copy_from_slice(&data[..len]);
        tracing::info!(size = len, "loaded DSP IROM");
    }

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

/// Read a big-endian u16 from a byte slice at a DSP word address.
#[inline(always)]
fn read_word(mem: &[u8], word_addr: u16) -> u16 {
    let off = word_addr as usize * 2;
    u16::from_be_bytes([mem[off], mem[off + 1]])
}

/// Write a big-endian u16 into a byte slice at a DSP word address.
#[inline(always)]
fn write_word(mem: &mut [u8], word_addr: u16, value: u16) {
    let off = word_addr as usize * 2;
    mem[off..off + 2].copy_from_slice(&value.to_be_bytes());
}
