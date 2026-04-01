use crate::cpu::instruction::Instruction;
use crate::cpu::{self, Cpu, IPL_RESET_VECTOR};
use crate::dvd::DvdInterface;
use crate::flipper::ai::AudioInterface;
use crate::flipper::cp::CommandProcessor;
use crate::flipper::dsp::Dsp;
use crate::flipper::exi::ExternalInterface;
use crate::flipper::exi::macronix::ExiMacronix;
use crate::flipper::gx::GraphicsProcessor;
use crate::flipper::mi::MemoryInterface;
use crate::flipper::pe::PixelEngine;
use crate::flipper::pi::ProcessorInterface;
use crate::flipper::si::{SerialInterface, pad};
use crate::flipper::vi::VideoInterface;
#[cfg(feature = "idle-skip")]
use crate::idle::{IDLE_LOOP_MAX_INSTRS, IdleCheck, IdleDetector};
use crate::mmio::Mmio;
use crate::scheduler::{CYCLES_PER_VSYNC, DSP_TICKS_PER_CPU_CYCLE, EventKind, Scheduler};
#[cfg(feature = "scripting")]
use crate::scripting::{HookFlags, ScriptHookFilters, ScriptHookState, ScriptHost};
use image::Executable;

pub struct GameCube {
    pub vsync_pending: bool,
    pub cpu: Cpu,
    pub scheduler: Scheduler,
    pub mmio: Mmio,
    pub vi: VideoInterface,
    pub pe: PixelEngine,
    pub pi: ProcessorInterface,
    pub dsp: Dsp,
    pub exi: ExternalInterface,
    pub gx: GraphicsProcessor,
    pub cp: CommandProcessor,
    pub di: DvdInterface,
    pub si: SerialInterface,
    pub ai: AudioInterface,
    pub mi: MemoryInterface,
    #[cfg(feature = "idle-skip")]
    idle: IdleDetector,

    #[cfg(feature = "scripting")]
    pub script_host: Option<Box<dyn ScriptHost>>,
    #[cfg(feature = "scripting")]
    pub script_hook_flags: HookFlags,
    #[cfg(feature = "scripting")]
    pub script_hook_filters: ScriptHookFilters,
}

impl GameCube {
    #[cfg(feature = "scripting")]
    #[inline(always)]
    pub fn apply_script_hook_state(&mut self, state: ScriptHookState) {
        self.script_hook_flags = state.flags;
        self.script_hook_filters = state.filters;
    }

    #[cfg(feature = "scripting")]
    #[inline(always)]
    pub fn sync_pending_script_hook_state(&mut self, host: &mut dyn ScriptHost) {
        #[cfg(feature = "scripting-mut-traps")]
        match host.take_pending_hook_state() {
            Ok(Some(state)) => self.apply_script_hook_state(state),
            Ok(None) => {}
            Err(err) => tracing::error!(target: "script", error = %err, "failed to refresh script traps"),
        }

        #[cfg(not(feature = "scripting-mut-traps"))]
        let _ = host;
    }

    pub fn new(entrypoint: u32) -> Self {
        GameCube {
            vsync_pending: false,
            cpu: Cpu::new(entrypoint),
            scheduler: Scheduler::new(),
            mmio: Mmio::new(),
            vi: VideoInterface::new(),
            pe: PixelEngine::new(),
            pi: ProcessorInterface::new(),
            dsp: Dsp::new(),
            exi: ExternalInterface::dummy(),
            gx: GraphicsProcessor::new(),
            cp: CommandProcessor::new(),
            di: DvdInterface::new(),
            si: SerialInterface::new(),
            ai: AudioInterface::new(),
            mi: MemoryInterface::new(),
            #[cfg(feature = "idle-skip")]
            idle: IdleDetector::new(),

            #[cfg(feature = "scripting")]
            script_host: None,
            #[cfg(feature = "scripting")]
            script_hook_flags: HookFlags::empty(),
            #[cfg(feature = "scripting")]
            script_hook_filters: ScriptHookFilters::default(),
        }
    }

    pub fn with_image(exe: &impl Executable) -> Self {
        let mut emulator = GameCube::new(exe.entry_point());
        let data = exe.data();

        // Copy TEXT sections to memory
        for section in exe.text_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                emulator.mmio.virt_write_u8(addr, value);
            }
        }

        // Copy DATA sections to memory
        for section in exe.data_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                emulator.mmio.virt_write_u8(addr, value);
            }
        }

        // Zero out the BSS section
        let (bss_start, bss_size) = exe.bss();
        for i in 0..bss_size {
            let addr = bss_start + i;
            emulator.mmio.virt_write_u8(addr, 0);
        }

        emulator
    }

    pub fn with_ipl(ipl: &[u8]) -> Self {
        // Text Sections (1):
        // | idx | offset     | vaddr      | size       | end        |
        // |-----|------------|------------|------------|------------|
        // | 0   | 0x00000100 | 0x81300000 | 0x001FF7E0 | 0x814FF7E0 |
        // Data Sections (0):
        // | idx | offset | vaddr | size | end |
        // |-----|--------|-------|------|-----|
        // Entry point: 0x81300000
        // BSS: 0x00000000 - 0x00000000 (size: 0x00000000)
        // => BS2 DOL, does not apply to the actual IPL here!!

        let mut emulator = GameCube::new(IPL_RESET_VECTOR);
        emulator.cpu.msr.set_ip(true);
        emulator.mmio.ipl = ipl.to_vec();
        emulator.exi.attach_device(
            ExiMacronix::CHANNEL,
            ExiMacronix::DEVICE,
            Box::new(ExiMacronix::new(ipl.to_vec())),
        );
        // TODO: this makes 0x8130107C (NTSC BS2) exit the DVD state machine
        // as it forces it to enter "state 19"
        emulator.open_cover();
        emulator
    }

    #[inline(always)]
    pub fn step(&mut self) {
        // Fire any events that are due
        while let Some(event) = self.scheduler.poll() {
            match event {
                EventKind::VSync => {
                    self.vsync_pending = true;
                    self.scheduler.schedule_in(CYCLES_PER_VSYNC, EventKind::VSync);
                }
                EventKind::ViHalfLine => {
                    self.vi.on_half_line(self.scheduler.cycles);
                    self.vi.half_line_scheduled = false;
                    self.maybe_schedule_vi_half_line();
                    self.check_vi_interrupts();
                }
                EventKind::DiTransferComplete => {
                    self.complete_dvd_transfer();
                }
                EventKind::DspTick => {
                    self.tick_dsp();
                    self.scheduler.schedule_in(DSP_TICKS_PER_CPU_CYCLE, EventKind::DspTick);
                }
            }
        }

        // Deliver external interrupt when EE=1 and any enabled PI interrupt is pending
        if self.cpu.msr.external_interrupt_enable() && self.pi.interrupt_pending() {
            self.cause_external_interrupt();
            self.scheduler.cycles += 1;
            return;
        }

        // CPU pre-hook
        #[cfg(feature = "scripting")]
        if self.script_hook_flags.contains(HookFlags::CPU_PRE) {
            let pc = self.cpu.pc;
            if self.script_hook_filters.cpu_pre.matches(pc) {
                if let Some(mut host) = self.script_host.take() {
                    host.on_cpu_pre(self);
                    self.sync_pending_script_hook_state(host.as_mut());
                    self.script_host = Some(host);
                }
            }
        }

        // Fetch and execute next instruction
        self.cpu.cia = self.cpu.pc;
        self.cpu.nia = self.cpu.cia.wrapping_add(4);
        let instr = Instruction(self.mmio.fetch_instruction(self.cpu.cia));
        cpu::lut::dispatch(self, instr);
        self.scheduler.cycles += 1;

        // CPU post-hook
        #[cfg(feature = "scripting")]
        if self.script_hook_flags.contains(HookFlags::CPU_POST) {
            let pc = self.cpu.cia;
            if self.script_hook_filters.cpu_post.matches(pc) {
                if let Some(mut host) = self.script_host.take() {
                    host.on_cpu_post(self);
                    self.sync_pending_script_hook_state(host.as_mut());
                    self.script_host = Some(host);
                }
            }
        }

        #[cfg(feature = "idle-skip")]
        match self.idle.check(self.cpu.cia, self.cpu.nia) {
            IdleCheck::Skip => {
                if let Some(deadline) = self.scheduler.next_event_deadline() {
                    self.scheduler.cycles = deadline;
                }
            }
            IdleCheck::Validate { start, end } => {
                let safe = self.is_polling_loop(start, end);
                self.idle.set_validated(safe);
                if safe {
                    if let Some(deadline) = self.scheduler.next_event_deadline() {
                        self.scheduler.cycles = deadline;
                    }
                }
            }
            IdleCheck::Continue => {}
        }

        self.cpu.pc = self.cpu.nia;
    }

    #[inline(always)]
    pub fn prepare_frame(&mut self) {
        self.vsync_pending = false;
        self.si.update_polling();
        self.check_si_interrupts();
    }

    pub fn run_until_vsync(&mut self) {
        self.prepare_frame();
        while !self.vsync_pending {
            self.step();
        }
    }

    /// Read the instructions in `[start, end]` and check whether the loop is a
    /// side effect free MMIO polling loop that can safely be skipped.
    #[cfg(feature = "idle-skip")]
    #[inline(always)]
    fn is_polling_loop(&self, start: u32, end: u32) -> bool {
        let count = ((end - start) / 4 + 1) as usize;
        let mut buf = [0u32; IDLE_LOOP_MAX_INSTRS];
        for i in 0..count.min(buf.len()) {
            buf[i] = self.mmio.fetch_instruction(start + (i as u32) * 4);
        }
        crate::idle::validate_polling_loop(&buf[..count.min(buf.len())], &self.cpu.gprs)
    }

    #[cfg(feature = "scripting")]
    pub fn set_script_host(&mut self, host: Box<dyn ScriptHost>) {
        self.apply_script_hook_state(host.hook_state());
        self.script_host = Some(host);
    }

    #[cfg(all(feature = "scripting", feature = "scripting-mut-traps"))]
    pub fn refresh_script_traps(&mut self) -> Result<(), String> {
        let Some(mut host) = self.script_host.take() else {
            return Ok(());
        };

        let refresh_result = host.force_refresh_traps();
        match refresh_result {
            Ok(state) => {
                self.apply_script_hook_state(state);
                self.script_host = Some(host);
                Ok(())
            }
            Err(err) => {
                self.script_host = Some(host);
                Err(err)
            }
        }
    }

    pub fn frame_size(&self) -> (usize, usize) {
        let fmt = self.vi.dcr.video_format();
        (fmt.columns(), fmt.lines())
    }

    pub fn add_primary_controller(&mut self, input: pad::PadStatus) {
        self.si.pad_state[0] = input;
    }

    pub fn primary_controller_mut(&mut self) -> &mut pad::PadStatus {
        &mut self.si.pad_state[0]
    }
}
