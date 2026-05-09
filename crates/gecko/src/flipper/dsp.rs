pub mod accelerator;
pub mod addr;
pub mod condition;
pub mod core;
#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod instruction;
pub mod interpreter;
#[cfg(feature = "jit")]
pub mod jit;
pub mod regs;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/dsp_lut.rs"));
}

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut_wii {
    include!(concat!(env!("OUT_DIR"), "/dsp_lut_wii.rs"));
}

use crate::flipper::dsp::instruction::Instruction;
use crate::mmio::Mmio;
use crate::system::{System, SystemId};

#[cfg(feature = "jit")]
pub const DSP_JIT_CHAIN_BUDGET: u32 = 16;

pub struct Dsp {
    pub registers: core::Registers,

    pub iram: Box<[u8; 0x2000]>,
    pub irom: Box<[u8; 0x2000]>,

    pub dram: Box<[u8; 0x2000]>,
    pub coef: Box<[u8; 0x2000]>,
    pub ifx: Box<[u8; 0x200]>,

    pub aram: Box<[u8; 16 * 1024 * 1024]>,

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

    pub dma_control: core::regs::DspDmaControl,
    pub dma_length: u16,
    pub dma_dsp_addr: u16,
    pub dma_ram_addr_hi: u16,
    pub dma_ram_addr_lo: u16,

    pub accelerator: accelerator::Accelerator,

    #[cfg(feature = "jit")]
    pub jit: Option<Box<dyn DspJitHandle + Send>>,

    #[cfg(feature = "jit")]
    pub chain_budget: u32,

    #[cfg(feature = "jit")]
    pub instr_count: u32,

    pub poll_cache: std::cell::Cell<Option<(u16, bool, bool)>>,

    pub scheduler_suspended: bool,
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
            accelerator: accelerator::Accelerator::new(),

            #[cfg(feature = "jit")]
            jit: None,
            #[cfg(feature = "jit")]
            chain_budget: 0,
            #[cfg(feature = "jit")]
            instr_count: 0,
            poll_cache: std::cell::Cell::new(None),
            scheduler_suspended: false,
        }
    }

    #[inline]
    pub fn is_waiting_for_cpu_mail(&self) -> bool {
        let (cpu, _dsp) = self.poll_cache_for_pc(self.registers.pc);
        cpu
    }

    #[inline]
    pub fn is_waiting_for_dsp_mail(&self) -> bool {
        let (_cpu, dsp) = self.poll_cache_for_pc(self.registers.pc);
        dsp
    }

    #[inline]
    pub fn mailbox_wait_state(&self) -> (bool, bool) {
        self.poll_cache_for_pc(self.registers.pc)
    }

    fn poll_cache_for_pc(&self, pc: u16) -> (bool, bool) {
        if let Some((cached_pc, cpu, dsp)) = self.poll_cache.get() {
            if cached_pc == pc {
                return (cpu, dsp);
            }
        }

        const OFFSETS: [i16; 3] = [0, -1, -3];
        let cpu = OFFSETS.iter().any(|&o| self.matches_cpu_mail_wait(o));
        let dsp = OFFSETS.iter().any(|&o| self.matches_dsp_mail_wait(o));
        self.poll_cache.set(Some((pc, cpu, dsp)));
        (cpu, dsp)
    }

    fn matches_cpu_mail_wait(&self, offset: i16) -> bool {
        let start = self.registers.pc.wrapping_add_signed(offset);
        let words = self.read_imem_window::<5>(start);

        let pattern_a = [0x26FE, 0x02C0, 0x8000, 0x029C, start];
        let pattern_b = [0x27FE, 0x03C0, 0x8000, 0x029C, start];
        let pattern_c = [0x26FE, 0x02A0, 0x8000, 0x029D, start];
        let pattern_d = [0x27FE, 0x03A0, 0x8000, 0x029D, start];
        words == pattern_a || words == pattern_b || words == pattern_c || words == pattern_d
    }

    fn matches_dsp_mail_wait(&self, offset: i16) -> bool {
        let start = self.registers.pc.wrapping_add_signed(offset);
        let words = self.read_imem_window::<5>(start);

        let pattern_a = [0x26FC, 0x02C0, 0x8000, 0x029D, start];
        let pattern_b = [0x27FC, 0x03C0, 0x8000, 0x029D, start];
        let pattern_c = [0x26FC, 0x02A0, 0x8000, 0x029C, start];
        let pattern_d = [0x27FC, 0x03A0, 0x8000, 0x029C, start];
        words == pattern_a || words == pattern_b || words == pattern_c || words == pattern_d
    }

    fn read_imem_window<const N: usize>(&self, start: u16) -> [u16; N] {
        let mut out = [0u16; N];
        for i in 0..N {
            out[i] = self.read_imem(start.wrapping_add(i as u16));
        }
        out
    }

    pub fn process_aram_dma<const SYSTEM: SystemId>(&mut self, mmio: &mut Mmio<SYSTEM>) {
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

    pub fn process_ucode_upload<const SYSTEM: SystemId>(&mut self, mmio: &mut Mmio<SYSTEM>) {
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

        self.poll_cache.set(None);

        #[cfg(feature = "jit")]
        if let Some(jit) = self.jit.as_mut() {
            jit.flush();
        }
    }

    pub fn process_dsp_dma<const SYSTEM: SystemId>(&mut self, mmio: &mut Mmio<SYSTEM>) {
        let ram_addr = ((self.dma_ram_addr_hi as u32) << 16) | self.dma_ram_addr_lo as u32;
        let dsp_addr = (self.dma_dsp_addr as usize) * 2;
        let len = self.dma_length as usize;

        tracing::debug!(
            ram_addr = format!("{ram_addr:08X}"),
            dsp_addr = format!("{dsp_addr:04X}"),
            len,
            dir = ?self.dma_control.direction(),
            mem = ?self.dma_control.memory_type(),
            "DSP DMA"
        );

        let memory = match self.dma_control.memory_type() {
            core::regs::DspMemoryType::Data => &mut *self.dram,
            core::regs::DspMemoryType::Instruction => &mut *self.iram,
        };

        let mem_type = self.dma_control.memory_type();
        let direction = self.dma_control.direction();
        match direction {
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

        if matches!(
            (mem_type, direction),
            (
                core::regs::DspMemoryType::Instruction,
                core::regs::DspDmaDirection::MainToDsp
            )
        ) {
            self.poll_cache.set(None);
            #[cfg(feature = "jit")]
            if let Some(jit) = self.jit.as_mut() {
                jit.flush();
            }
        }

        self.dma_length = 0;
    }
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    #[inline(always)]
    pub fn step_dsp_instruction(&mut self) -> bool {
        if self.dsp.csr.reset() || self.dsp.csr.halt() {
            return false;
        }

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

    #[cfg(feature = "jit")]
    pub fn execute_dsp_batch(&mut self) {
        if self.dsp.csr.reset() || self.dsp.csr.halt() {
            self::refresh_interrupts(self);
            return;
        }

        if self.dsp.jit.is_none() {
            self.dsp.jit = Some(Box::new(jit::JitEngine::<SYSTEM>::new()));
        }

        let ctx_ptr = self as *mut crate::system::System<SYSTEM> as *mut ::core::ffi::c_void;
        let iram_ptr = self.dsp.iram.as_ptr();
        let irom_ptr = self.dsp.irom.as_ptr();
        let iram_len = self.dsp.iram.len();
        let irom_len = self.dsp.irom.len();
        let iram = unsafe { ::core::slice::from_raw_parts(iram_ptr, iram_len) };
        let irom = unsafe { ::core::slice::from_raw_parts(irom_ptr, irom_len) };

        let mut budget = crate::scheduler::DSP_BATCH_SIZE as u64;
        while budget > 0 {
            let cpu_mail_quiet = !self.dsp.mailbox_to_dsp_hi.busy();
            let dsp_mail_full = self.dsp.mailbox_to_cpu_hi.busy();

            if (cpu_mail_quiet && self.dsp.is_waiting_for_cpu_mail())
                || (dsp_mail_full && self.dsp.is_waiting_for_dsp_mail())
            {
                break;
            }

            if self.dsp.csr.reset() || self.dsp.csr.halt() {
                break;
            }

            if self.dsp.csr.pi_interrupt() && self.dsp.registers.status.external_interrupt_enable() {
                self.dsp.csr = self.dsp.csr.with_pi_interrupt(false);
                self.dsp.registers.call_stack.push(self.dsp.registers.pc);
                self.dsp.registers.data_stack.push(self.dsp.registers.status.raw());
                self.dsp.registers.status = self.dsp.registers.status.with_external_interrupt_enable(false);
                self.dsp.registers.pc = 0x000E;
            }

            let start_pc = self.dsp.registers.pc;
            self.dsp.chain_budget = DSP_JIT_CHAIN_BUDGET;
            self.dsp.instr_count = 0;

            let next_pc = self.dsp.jit.as_mut().unwrap().run_block(ctx_ptr, iram, irom, start_pc);
            self.dsp.registers.pc = next_pc;

            let consumed = (self.dsp.instr_count as u64).max(1);
            budget = budget.saturating_sub(consumed);

            let chain_depth = DSP_JIT_CHAIN_BUDGET - self.dsp.chain_budget;
            self.dsp.jit.as_mut().unwrap().record_chain_depth(chain_depth);
        }

        self::refresh_interrupts(self);
    }

    #[cfg(not(feature = "jit"))]
    #[inline(always)]
    pub fn execute_dsp_batch(&mut self) {
        for _ in 0..crate::scheduler::DSP_BATCH_SIZE {
            if !self.step_dsp_instruction() {
                break;
            }
        }
        self::refresh_interrupts(self);
    }

    #[cfg(feature = "jit")]
    pub fn drain_dsp_synchronous(&mut self, max_steps: u32) {
        let already_busy = self.dsp.mailbox_to_cpu_hi.busy();

        if self.dsp.csr.reset() || self.dsp.csr.halt() {
            self::refresh_interrupts(self);
            return;
        }

        if self.dsp.jit.is_none() {
            self.dsp.jit = Some(Box::new(jit::JitEngine::<SYSTEM>::new()));
        }

        let ctx_ptr = self as *mut crate::system::System<SYSTEM> as *mut ::core::ffi::c_void;
        let iram_ptr = self.dsp.iram.as_ptr();
        let irom_ptr = self.dsp.irom.as_ptr();
        let iram_len = self.dsp.iram.len();
        let irom_len = self.dsp.irom.len();
        let iram = unsafe { ::core::slice::from_raw_parts(iram_ptr, iram_len) };
        let irom = unsafe { ::core::slice::from_raw_parts(irom_ptr, irom_len) };

        let mut budget = max_steps as u64;
        while budget > 0 {
            if self.dsp.csr.reset() || self.dsp.csr.halt() {
                break;
            }

            if !already_busy && self.dsp.mailbox_to_cpu_hi.busy() {
                break;
            }

            let cpu_mail_quiet = !self.dsp.mailbox_to_dsp_hi.busy();
            let dsp_mail_full = self.dsp.mailbox_to_cpu_hi.busy();
            if (cpu_mail_quiet && self.dsp.is_waiting_for_cpu_mail())
                || (dsp_mail_full && self.dsp.is_waiting_for_dsp_mail())
            {
                break;
            }

            if self.dsp.csr.pi_interrupt() && self.dsp.registers.status.external_interrupt_enable() {
                self.dsp.csr = self.dsp.csr.with_pi_interrupt(false);
                self.dsp.registers.call_stack.push(self.dsp.registers.pc);
                self.dsp.registers.data_stack.push(self.dsp.registers.status.raw());
                self.dsp.registers.status = self.dsp.registers.status.with_external_interrupt_enable(false);
                self.dsp.registers.pc = 0x000E;
            }

            let start_pc = self.dsp.registers.pc;
            self.dsp.chain_budget = DSP_JIT_CHAIN_BUDGET;
            self.dsp.instr_count = 0;

            let next_pc = self.dsp.jit.as_mut().unwrap().run_block(ctx_ptr, iram, irom, start_pc);
            self.dsp.registers.pc = next_pc;

            let consumed = (self.dsp.instr_count as u64).max(1);
            budget = budget.saturating_sub(consumed);

            let chain_depth = DSP_JIT_CHAIN_BUDGET - self.dsp.chain_budget;
            self.dsp.jit.as_mut().unwrap().record_chain_depth(chain_depth);
        }

        self::refresh_interrupts(self);
    }

    #[cfg(not(feature = "jit"))]
    pub fn drain_dsp_synchronous(&mut self, max_steps: u32) {
        let already_busy = self.dsp.mailbox_to_cpu_hi.busy();

        for _ in 0..max_steps {
            if !self.step_dsp_instruction() {
                break;
            }

            if !already_busy && self.dsp.mailbox_to_cpu_hi.busy() {
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
        regs::AudioDmaBlocksLeft,
    ],
}

impl Dsp {
    #[inline(always)]
    pub fn read_imem(&self, addr: u16) -> u16 {
        match addr {
            0x0000..0x1000 => read_word(&*self.iram, addr),
            0x8000..0x9000 => read_word(&*self.irom, addr - 0x8000),
            _ => 0,
        }
    }

    pub fn load_irom(&mut self, data: &[u8]) {
        let len = data.len().min(self.irom.len());
        self.irom[..len].copy_from_slice(&data[..len]);
        tracing::info!(size = len, "loaded DSP IROM");
    }

    pub fn load_coef(&mut self, data: &[u8]) {
        let len = data.len().min(self.coef.len());
        self.coef[..len].copy_from_slice(&data[..len]);
        tracing::info!(size = len, "loaded DSP coefficient ROM");
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        (self.csr.ai_interrupt() && self.csr.ai_interrupt_mask())
            || (self.csr.ar_interrupt() && self.csr.ar_interrupt_mask())
            || (self.csr.dsp_interrupt() && self.csr.dsp_interrupt_mask())
    }
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    if sys.dsp.interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Dsp);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Dsp);
    }

    if sys.dsp.csr.pi_interrupt() && sys.dsp.registers.status.external_interrupt_enable() {
        self::wake_dsp_scheduler::<SYSTEM>(sys);
    }
}

#[cfg(feature = "jit-stats")]
pub static DSP_SUSPEND_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "jit-stats")]
pub static DSP_WAKE_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[inline]
pub fn wake_dsp_scheduler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if !sys.dsp.scheduler_suspended {
        return;
    }

    if sys.dsp.csr.halt() || sys.dsp.csr.reset() {
        return;
    }

    sys.dsp.scheduler_suspended = false;
    sys.scheduler.schedule_in(
        crate::scheduler::dsp_batch_interval(SYSTEM),
        crate::scheduler::dsp_batch_handler::<SYSTEM>,
    );

    #[cfg(feature = "jit-stats")]
    DSP_WAKE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub fn read_dmem<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, addr: u16) -> u16 {
    match addr {
        0x0000..0x1000 => read_word(&*sys.dsp.dram, addr),
        0x1000..0x2000 => read_word(&*sys.dsp.coef, addr - 0x1000),
        0xFF00..=0xFFFF => read_ifx(sys, addr),
        _ => 0,
    }
}

#[inline(always)]
pub fn write_dmem<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, addr: u16, value: u16) {
    match addr {
        0x0000..0x1000 => write_word(&mut *sys.dsp.dram, addr, value),
        0xFF00..=0xFFFF => write_ifx(sys, addr, value),
        _ => {}
    }
}

#[inline(always)]
fn read_ifx<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, addr: u16) -> u16 {
    match addr {
        // CMBH (CPU Mailbox High): reading returns data + M bit.
        // M is only cleared when CMBL is read.
        addr::IFX_CMBH => sys.dsp.mailbox_to_dsp_hi.raw(),
        // CMBL (CPU Mailbox Low): reading clears CMBH.M (busy)
        addr::IFX_CMBL => {
            sys.dsp.mailbox_to_dsp_hi.set_busy(false);
            sys.dsp.mailbox_to_dsp_lo.raw()
        }
        // DMBH (DSP Mailbox High): DSP reads back what it wrote
        addr::IFX_DMBH => sys.dsp.mailbox_to_cpu_hi.raw(),
        // DMBL (DSP Mailbox Low): DSP reads back what it wrote (no side effects)
        addr::IFX_DMBL => sys.dsp.mailbox_to_cpu_lo.raw(),
        // DSP DMA registers
        addr::IFX_DSCR => sys.dsp.dma_control.raw(),
        addr::IFX_DSBL => sys.dsp.dma_length,
        addr::IFX_DSPA => sys.dsp.dma_dsp_addr,
        addr::IFX_DSMAH => sys.dsp.dma_ram_addr_hi,
        addr::IFX_DSMAL => sys.dsp.dma_ram_addr_lo,
        // Audio sample accelerator
        addr::IFX_FORMAT => sys.dsp.accelerator.format.raw(),
        addr::IFX_ACSAH => (sys.dsp.accelerator.start_addr >> 16) as u16,
        addr::IFX_ACSAL => sys.dsp.accelerator.start_addr as u16,
        addr::IFX_ACEAH => (sys.dsp.accelerator.end_addr >> 16) as u16,
        addr::IFX_ACEAL => sys.dsp.accelerator.end_addr as u16,
        addr::IFX_ACCAH => (sys.dsp.accelerator.current_addr >> 16) as u16,
        addr::IFX_ACCAL => sys.dsp.accelerator.current_addr as u16,
        addr::IFX_PRED_SCALE => sys.dsp.accelerator.pred_scale,
        addr::IFX_YN1 => sys.dsp.accelerator.yn1 as u16,
        addr::IFX_YN2 => sys.dsp.accelerator.yn2 as u16,
        addr::IFX_GAIN => sys.dsp.accelerator.gain as u16,
        addr::IFX_ACIN => sys.dsp.accelerator.input,
        addr::IFX_ACDSAMP => accelerator::read_decoded_sample::<SYSTEM>(&mut sys.dsp, sys.mmio.ram_view()),
        addr::IFX_ACDRAW => accelerator::read_raw::<SYSTEM>(&mut sys.dsp, sys.mmio.ram_view()),
        _ => {
            tracing::debug!(addr = format!("{:04X}", addr), "Read from unknown DSP IFX register");
            read_word(&*sys.dsp.ifx, addr - 0xFF00)
        }
    }
}

#[inline(always)]
fn write_ifx<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, addr: u16, value: u16) {
    match addr {
        // DMBH (DSP Mailbox High): store data bits (14:0), busy is preserved
        addr::IFX_DMBH => {
            let busy = sys.dsp.mailbox_to_cpu_hi.busy();
            sys.dsp.mailbox_to_cpu_hi = regs::MailboxToCpuHi::from_raw(value & 0x7FFF).with_busy(busy);
        }
        // DMBL (DSP Mailbox Low): writing sets DMBH.M
        addr::IFX_DMBL => {
            sys.dsp.mailbox_to_cpu_lo = regs::MailboxToCpuLo::from_raw(value);
            sys.dsp.mailbox_to_cpu_hi.set_busy(true);
        }
        // DIRQ: DSP explicitly raises interrupt to CPU
        addr::IFX_DIRQ => {
            if value & 1 != 0 {
                tracing::debug!("DSP DIRQ: requesting CPU interrupt");
                sys.dsp.csr.set_dsp_interrupt(true);
            }
        }
        // CMBH/CMBL are read-only from DSP side
        addr::IFX_CMBH | addr::IFX_CMBL => {}

        addr::IFX_DSBL => {
            sys.dsp.dma_length = value;
            sys.dsp.process_dsp_dma(&mut sys.mmio);
        }
        addr::IFX_DSCR => sys.dsp.dma_control = core::regs::DspDmaControl::from_raw(value),
        addr::IFX_DSPA => sys.dsp.dma_dsp_addr = value,
        addr::IFX_DSMAH => sys.dsp.dma_ram_addr_hi = value,
        addr::IFX_DSMAL => sys.dsp.dma_ram_addr_lo = value,
        // Audio sample accelerator
        addr::IFX_FORMAT => sys.dsp.accelerator.format = accelerator::SampleFormat::from_raw(value),
        addr::IFX_ACSAH => {
            let new = ((value as u32) << 16) | (sys.dsp.accelerator.start_addr & 0x0000_FFFF);
            sys.dsp.accelerator.set_start_addr(new);
        }
        addr::IFX_ACSAL => {
            let new = (sys.dsp.accelerator.start_addr & 0xFFFF_0000) | value as u32;
            sys.dsp.accelerator.set_start_addr(new);
        }
        addr::IFX_ACEAH => {
            let new = ((value as u32) << 16) | (sys.dsp.accelerator.end_addr & 0x0000_FFFF);
            sys.dsp.accelerator.set_end_addr(new);
        }
        addr::IFX_ACEAL => {
            let new = (sys.dsp.accelerator.end_addr & 0xFFFF_0000) | value as u32;
            sys.dsp.accelerator.set_end_addr(new);
        }
        addr::IFX_ACCAH => {
            let new = ((value as u32) << 16) | (sys.dsp.accelerator.current_addr & 0x0000_FFFF);
            sys.dsp.accelerator.set_current_addr(new);
        }
        addr::IFX_ACCAL => {
            let new = (sys.dsp.accelerator.current_addr & 0xFFFF_0000) | value as u32;
            sys.dsp.accelerator.set_current_addr(new);
        }
        addr::IFX_PRED_SCALE => sys.dsp.accelerator.set_pred_scale(value),
        addr::IFX_YN1 => sys.dsp.accelerator.yn1 = value as i16,
        addr::IFX_YN2 => sys.dsp.accelerator.set_yn2(value as i16),
        addr::IFX_GAIN => sys.dsp.accelerator.gain = value as i16,
        addr::IFX_ACIN => sys.dsp.accelerator.input = value,
        addr::IFX_ACDRAW => accelerator::write_raw::<SYSTEM>(&mut sys.dsp, sys.mmio.ram_view_mut(), value),
        // ACDSAMP is read-only
        addr::IFX_ACDSAMP => {}
        _ => {
            tracing::debug!(
                addr = format!("{:04X}", addr),
                value = format!("{:04X}", value),
                "Write to unknown DSP IFX register"
            );
            write_word(&mut *sys.dsp.ifx, addr - 0xFF00, value);
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
    if SYSTEM == crate::system::GC {
        let ctx: &mut crate::gamecube::GameCube = unsafe { ::core::mem::transmute(ctx) };
        self::lut::dispatch(ctx, instr);
    } else {
        let ctx: &mut crate::wii::Wii = unsafe { ::core::mem::transmute(ctx) };
        self::lut_wii::dispatch(ctx, instr);
    }
}

#[inline(always)]
pub fn dispatch_gc_dsp_ext<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: instruction::GcDspExt) {
    if SYSTEM == crate::system::GC {
        let ctx: &mut crate::gamecube::GameCube = unsafe { ::core::mem::transmute(ctx) };
        self::lut::dispatch_gc_dsp_ext(ctx, instr);
    } else {
        let ctx: &mut crate::wii::Wii = unsafe { ::core::mem::transmute(ctx) };
        self::lut_wii::dispatch_gc_dsp_ext(ctx, instr);
    }
}

#[cfg(feature = "jit")]
pub trait DspJitHandle {
    fn run_block(&mut self, ctx_ptr: *mut ::core::ffi::c_void, iram: &[u8], irom: &[u8], start_pc: u16) -> u16;
    fn record_chain_depth(&mut self, depth: u32);
    fn flush(&mut self);
    fn dump_hot_blocks(&self, top_k: usize);
    fn dump_hot_blocks_csv(&self, top_k: usize, path: &std::path::Path) -> std::io::Result<()>;
    fn dump_top_clif(&mut self, top_k: usize, iram: &[u8], irom: &[u8]);
    fn cached_blocks(&self) -> Vec<crate::jit_cache::CachedBlockDsp>;
    fn precompile_blocks(
        &mut self,
        iram: &[u8],
        irom: &[u8],
        blocks: &[crate::jit_cache::CachedBlockDsp],
    ) -> (usize, usize);
}

#[cfg(feature = "jit")]
impl<const SYSTEM: SystemId> DspJitHandle for jit::JitEngine<SYSTEM> {
    fn run_block(&mut self, ctx_ptr: *mut ::core::ffi::c_void, iram: &[u8], irom: &[u8], start_pc: u16) -> u16 {
        let entry = self.lookup_or_compile(iram, irom, start_pc);
        Self::run_block(self, ctx_ptr, entry)
    }

    fn record_chain_depth(&mut self, depth: u32) {
        Self::record_chain_depth(self, depth);
    }

    fn flush(&mut self) {
        Self::flush(self);
    }

    fn dump_hot_blocks(&self, _top_k: usize) {
        #[cfg(feature = "jit-stats")]
        Self::dump_hot_blocks(self, _top_k);
        #[cfg(not(feature = "jit-stats"))]
        tracing::warn!("feature `jit-stats` is not enabled. Rebuild with `--features jit-stats`.");
    }

    fn dump_hot_blocks_csv(&self, _top_k: usize, _path: &std::path::Path) -> std::io::Result<()> {
        #[cfg(feature = "jit-stats")]
        return Self::dump_hot_blocks_csv(self, _top_k, _path);
        #[cfg(not(feature = "jit-stats"))]
        Ok(())
    }

    fn cached_blocks(&self) -> Vec<crate::jit_cache::CachedBlockDsp> {
        Self::cached_blocks(self)
    }

    fn precompile_blocks(
        &mut self,
        iram: &[u8],
        irom: &[u8],
        blocks: &[crate::jit_cache::CachedBlockDsp],
    ) -> (usize, usize) {
        Self::precompile_blocks(self, iram, irom, blocks)
    }

    fn dump_top_clif(&mut self, _top_k: usize, _iram: &[u8], _irom: &[u8]) {
        #[cfg(feature = "jit-stats")]
        {
            let mut pcs: Vec<(u16, u64)> = self.hits.iter().map(|(&pc, &n)| (pc, n)).collect();
            pcs.sort_by(|a, b| b.1.cmp(&a.1));
            for (pc, hits) in pcs.into_iter().take(_top_k) {
                tracing::info!("hits={hits} pc={pc:04X}");
                self.dump_block_clif(pc, _iram, _irom);
            }
        }
        #[cfg(not(feature = "jit-stats"))]
        tracing::warn!("feature `jit-stats` is not enabled. Rebuild with `--features jit-stats`.");
    }
}
