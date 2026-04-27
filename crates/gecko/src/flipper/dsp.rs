pub mod addr;
pub mod condition;
pub mod core;
#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod instruction;
pub mod interpreter;
pub mod regs;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/dsp_lut.rs"));
}

use crate::flipper::dsp::instruction::Instruction;
use crate::mmio::Mmio;
use crate::system::{GC, System, SystemId};

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
    pub audio_dma_start_addr: regs::AudioDmaStartAddr,
    pub audio_dma_control: regs::AudioDmaControl,

    // IFX Registers
    pub dma_control: core::regs::DspDmaControl,
    pub dma_length: u16,
    pub dma_dsp_addr: u16,
    pub dma_ram_addr_hi: u16,
    pub dma_ram_addr_lo: u16,
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
            audio_dma_start_addr: regs::AudioDmaStartAddr::from_raw(0),
            audio_dma_control: regs::AudioDmaControl::from_raw(0),
            dma_control: core::regs::DspDmaControl::new(),
            dma_length: 0,
            dma_dsp_addr: 0,
            dma_ram_addr_hi: 0,
            dma_ram_addr_lo: 0,
        }
    }

    pub fn process_aram_dma(&mut self, mmio: &mut Mmio) {
        let ram_addr = (self.aram_dma_mmio_addr.raw() & 0x3FFFFFFF) as usize;
        let aram_addr = self.aram_dma_aram_addr.raw() as usize;
        let count = self.aram_dma_control.count() as usize;

        tracing::debug!(
            ram_addr = format!("{ram_addr:08X}"),
            aram_addr = format!("{aram_addr:08X}"),
            count,
            direction = ?self.aram_dma_control.direction(),
            "ARAM DMA"
        );

        let within_bounds = aram_addr + count <= self.aram.len();
        match self.aram_dma_control.direction() {
            regs::DmaDirection::AramToRam if within_bounds => {
                let src = &self.aram[aram_addr..aram_addr + count];
                let dst = mmio.virt_slice_mut(ram_addr as u32, count);
                dst.copy_from_slice(src);
            }
            regs::DmaDirection::RamToAram if within_bounds => {
                let src = mmio.virt_slice(ram_addr as u32, count);
                self.aram[aram_addr..aram_addr + count].copy_from_slice(&src);
            }
            _ => tracing::warn!("Ignoring out-of-bounds ARAM DMA transfer"),
        }

        self.aram_dma_control.set_count(0);
        self.csr.set_dma_status(false);
        self.csr = self.csr.with_ar_interrupt(true);
    }

    pub fn process_ucode_upload(&mut self, mmio: &mut Mmio) {
        const UCODE_ADDR: usize = 0x8100_0000;
        const UCODE_SIZE: usize = 1024;
        let src = mmio.virt_slice(UCODE_ADDR as u32, UCODE_SIZE);
        self.iram[..UCODE_SIZE].copy_from_slice(&src);

        tracing::info!(
            mmio_addr = format!("{UCODE_ADDR:08X}"),
            count = UCODE_SIZE,
            "DSP stub uploaded from RAM to IRAM, executing IRAM"
        );

        self.csr.set_dma_status(false);
        self.csr.set_dsp_interrupt(true);
    }

    pub fn process_dsp_dma(&mut self, mmio: &mut Mmio) {
        let ram_addr = ((self.dma_ram_addr_hi as u32) << 16) | self.dma_ram_addr_lo as u32;
        let dsp_addr = (self.dma_dsp_addr as usize) * 2; // word address -> byte offset
        let len = self.dma_length as usize;

        tracing::debug!(
            ram_addr = format!("{ram_addr:08X}"),
            dsp_addr = format!("{dsp_addr:04X}"),
            len,
            dscr = format!("{:04X}", self.dma_control.raw()),
            "DSP DMA"
        );

        let memory = match self.dma_control.memory_type() {
            core::regs::DspMemoryType::Data => &mut *self.dram,
            core::regs::DspMemoryType::Instruction => &mut *self.iram,
        };

        match self.dma_control.direction() {
            core::regs::DspDmaDirection::MainToDsp => {
                let src = mmio.virt_slice(ram_addr, len);
                memory[dsp_addr..dsp_addr + len].copy_from_slice(&src);
            }
            core::regs::DspDmaDirection::DspToMain => {
                let src = &memory[dsp_addr..dsp_addr + len];
                let dst = mmio.virt_slice_mut(ram_addr, len);
                dst.copy_from_slice(src);
            }
        }

        self.dma_length = 0;
    }
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    /// Execute a single DSP instruction. Returns `false` if the DSP is halted or
    /// in reset (no instruction was executed).
    #[inline(always)]
    pub fn step_dsp_instruction(&mut self) -> bool {
        if self.dsp.csr.reset() || self.dsp.csr.halt() {
            return false;
        }

        // Check for external interrupt (CPU->DSP mailbox interrupt via CSR bit 1)
        if self.dsp.csr.pi_interrupt() && self.dsp.registers.status.external_interrupt_enable() {
            self.dsp.csr = self.dsp.csr.with_pi_interrupt(false);
            self.dsp.registers.call_stack.push(self.dsp.registers.pc);
            self.dsp.registers.data_stack.push(self.dsp.registers.status.raw());
            self.dsp.registers.status = self.dsp.registers.status.with_external_interrupt_enable(false);
            self.dsp.registers.pc = 0x000E;
        }

        let pc = self.dsp.registers.pc as usize;
        let w0 = self.dsp.read_imem(pc as u16);
        let w1 = self.dsp.read_imem((pc as u16).wrapping_add(1));
        let buf = [(w0 >> 8) as u8, w0 as u8, (w1 >> 8) as u8, w1 as u8];
        let instr = Instruction::from_be_bytes(&buf);
        self.dsp.registers.cia = self.dsp.registers.pc;
        let natural_nia = self.dsp.registers.cia.wrapping_add(lut::instr_size(instr) as u16);
        self.dsp.registers.nia = natural_nia;

        let ext_op = instr.ext_opcode();
        if ext_op.is_some() {
            self.dsp.registers.cache_ext_ac();
        }

        self::dispatch(self, instr);

        if let Some(ext) = ext_op {
            self::dispatch_gc_dsp_ext(self, instruction::GcDspExt(ext));
        }

        // Check if we've reached the end of a loop.
        let at_loop_end =
            !self.dsp.registers.loop_addr.is_empty() && self.dsp.registers.nia == self.dsp.registers.loop_addr.top();
        if at_loop_end {
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
        true
    }

    /// Execute a batch of DSP instructions (scheduler hot path).
    #[inline(always)]
    pub fn execute_dsp_batch(&mut self) {
        for _ in 0..crate::scheduler::DSP_BATCH_SIZE {
            if !self.step_dsp_instruction() {
                break;
            }
        }
        self::refresh_interrupts(self);
    }
}

crate::mmio_device_dispatch! {
    read = dsp_read,
    write = dsp_write,
    registers = [
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
        regs::AudioDmaStartAddr,
        regs::AudioDmaControl,
    ],
}

impl Dsp {
    /// Read a 16-bit word from instruction memory (IRAM 0x0000-0x0FFF, IROM 0x8000-0x8FFF).
    #[inline(always)]
    pub fn read_imem(&self, addr: u16) -> u16 {
        match addr {
            0x0000..0x1000 => read_word(&*self.iram, addr),
            0x8000..0x9000 => read_word(&*self.irom, addr - 0x8000),
            _ => 0,
        }
    }

    /// Load a binary file into IROM.
    pub fn load_irom(&mut self, data: &[u8]) {
        let len = data.len().min(self.irom.len());
        self.irom[..len].copy_from_slice(&data[..len]);
        tracing::info!(size = len, "loaded DSP IROM");
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        (self.csr.ai_interrupt() && self.csr.ai_interrupt_mask())
            || (self.csr.ar_interrupt() && self.csr.ar_interrupt_mask())
            || (self.csr.dsp_interrupt() && self.csr.dsp_interrupt_mask())
    }
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    if gc.dsp.interrupt_active() {
        gc.pi.assert_interrupt(InterruptFlag::Dsp);
    } else {
        gc.pi.clear_interrupt(InterruptFlag::Dsp);
    }
}

#[inline(always)]
pub fn read_dmem<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>, addr: u16) -> u16 {
    match addr {
        0x0000..0x1000 => read_word(&*gc.dsp.dram, addr),
        0x1000..0x2000 => read_word(&*gc.dsp.coef, addr - 0x1000),
        0xFF00..=0xFFFF => read_ifx(gc, addr),
        _ => 0,
    }
}

#[inline(always)]
pub fn write_dmem<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>, addr: u16, value: u16) {
    match addr {
        0x0000..0x1000 => write_word(&mut *gc.dsp.dram, addr, value),
        0xFF00..=0xFFFF => write_ifx(gc, addr, value),
        _ => {}
    }
}

#[inline(always)]
fn read_ifx<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>, addr: u16) -> u16 {
    match addr {
        // CMBH (CPU Mailbox High): reading returns data + M bit.
        // M is only cleared when CMBL is read.
        addr::IFX_CMBH => gc.dsp.mailbox_to_dsp_hi.raw(),
        // CMBL (CPU Mailbox Low): reading clears CMBH.M (busy)
        addr::IFX_CMBL => {
            gc.dsp.mailbox_to_dsp_hi.set_busy(false);
            gc.dsp.mailbox_to_dsp_lo.raw()
        }
        // DMBH (DSP Mailbox High): DSP reads back what it wrote
        addr::IFX_DMBH => gc.dsp.mailbox_to_cpu_hi.raw(),
        // DMBL (DSP Mailbox Low): DSP reads back what it wrote (no side effects)
        addr::IFX_DMBL => gc.dsp.mailbox_to_cpu_lo.raw(),
        // DSP DMA registers
        addr::IFX_DSCR => gc.dsp.dma_control.raw(),
        addr::IFX_DSBL => gc.dsp.dma_length,
        addr::IFX_DSPA => gc.dsp.dma_dsp_addr,
        addr::IFX_DSMAH => gc.dsp.dma_ram_addr_hi,
        addr::IFX_DSMAL => gc.dsp.dma_ram_addr_lo,
        _ => {
            tracing::debug!(addr = format!("{:04X}", addr), "Read from unknown DSP IFX register");
            read_word(&*gc.dsp.ifx, addr - 0xFF00)
        }
    }
}

#[inline(always)]
fn write_ifx<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>, addr: u16, value: u16) {
    match addr {
        // DMBH (DSP Mailbox High): store data bits (14:0), busy is preserved
        addr::IFX_DMBH => {
            let busy = gc.dsp.mailbox_to_cpu_hi.busy();
            gc.dsp.mailbox_to_cpu_hi = regs::MailboxToCpuHi::from_raw(value & 0x7FFF).with_busy(busy);
        }
        // DMBL (DSP Mailbox Low): writing sets DMBH.M
        addr::IFX_DMBL => {
            gc.dsp.mailbox_to_cpu_lo = regs::MailboxToCpuLo::from_raw(value);
            gc.dsp.mailbox_to_cpu_hi.set_busy(true);
        }
        // DIRQ: DSP explicitly raises interrupt to CPU
        addr::IFX_DIRQ => {
            if value & 1 != 0 {
                tracing::debug!("DSP DIRQ: requesting CPU interrupt");
                gc.dsp.csr.set_dsp_interrupt(true);
            }
        }
        // CMBH/CMBL are read-only from DSP side
        addr::IFX_CMBH | addr::IFX_CMBL => {}
        // DSP DMA: writing DSBL triggers the transfer, run inline.
        addr::IFX_DSBL => {
            gc.dsp.dma_length = value;
            gc.dsp.process_dsp_dma(&mut gc.mmio);
        }
        addr::IFX_DSCR => gc.dsp.dma_control = core::regs::DspDmaControl::from_raw(value),
        addr::IFX_DSPA => gc.dsp.dma_dsp_addr = value,
        addr::IFX_DSMAH => gc.dsp.dma_ram_addr_hi = value,
        addr::IFX_DSMAL => gc.dsp.dma_ram_addr_lo = value,
        _ => {
            tracing::debug!(
                addr = format!("{:04X}", addr),
                value = format!("{:04X}", value),
                "Write to unknown DSP IFX register"
            );
            write_word(&mut *gc.dsp.ifx, addr - 0xFF00, value);
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

#[inline(always)]
pub fn dispatch<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    if SYSTEM == GC {
        let ctx: &mut System<{ GC }> = unsafe { ::core::mem::transmute(ctx) };
        self::lut::dispatch(ctx, instr);
    } else {
        todo!()
    }
}

#[inline(always)]
pub fn dispatch_gc_dsp_ext<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: instruction::GcDspExt) {
    if SYSTEM == GC {
        let ctx: &mut System<{ GC }> = unsafe { ::core::mem::transmute(ctx) };
        self::lut::dispatch_gc_dsp_ext(ctx, instr);
    } else {
        todo!()
    }
}