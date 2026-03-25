#[cfg(feature = "scripting")]
use crate::scripting::{HookFlags, ScriptHookFilters, ScriptHookState, ScriptHost};
use crate::{
    cpu::{self, Cpu, IPL_RESET_VECTOR, semantics::Instruction},
    exi::{Exi, macronix::ExiMacronix},
    flipper::{
        ai::Ai,
        cp::Cp,
        di::Di,
        dsp::Dsp,
        gx::Gx,
        mi::Mi,
        pe::Pe,
        pi::Pi,
        si::{Si, pad},
        vi::Vi,
    },
    idle::{IDLE_LOOP_MAX_INSTRS, IdleCheck, IdleDetector},
    mmio::Mmio,
    scheduler::{CYCLES_PER_VSYNC, EventKind, Scheduler},
};
use image::Executable;

pub struct Gekko {
    pub vsync_pending: bool,
    pub cpu: Cpu,
    pub scheduler: Scheduler,
    pub mmio: Mmio,
    pub vi: Vi,
    pub pe: Pe,
    pub pi: Pi,
    pub dsp: Dsp,
    pub exi: Exi,
    pub gx: Gx,
    pub cp: Cp,
    pub di: Di,
    pub si: Si,
    pub ai: Ai,
    pub mi: Mi,
    idle: IdleDetector,

    #[cfg(feature = "scripting")]
    pub script_host: Option<Box<dyn ScriptHost>>,
    #[cfg(feature = "scripting")]
    pub script_hook_flags: HookFlags,
    #[cfg(feature = "scripting")]
    pub script_hook_filters: ScriptHookFilters,
}

impl Gekko {
    #[cfg(feature = "scripting")]
    #[inline(always)]
    pub(crate) fn apply_script_hook_state(&mut self, state: ScriptHookState) {
        self.script_hook_flags = state.flags;
        self.script_hook_filters = state.filters;
    }

    #[cfg(feature = "scripting")]
    #[inline(always)]
    pub(crate) fn sync_pending_script_hook_state(&mut self, host: &mut dyn ScriptHost) {
        #[cfg(feature = "scripting-mut-traps")]
        match host.take_pending_hook_state() {
            Ok(Some(state)) => self.apply_script_hook_state(state),
            Ok(None) => {}
            Err(err) => tracing::error!(target: "script", error = %err, "failed to refresh script traps"),
        }

        #[cfg(not(feature = "scripting-mut-traps"))]
        let _ = host;
    }

    pub fn new(entrypoint: u32, idle_skip: bool) -> Self {
        Gekko {
            vsync_pending: false,
            cpu: Cpu::new(entrypoint),
            scheduler: Scheduler::new(),
            mmio: Mmio::new(),
            vi: Vi::new(),
            pe: Pe::new(),
            pi: Pi::new(),
            dsp: Dsp::new(),
            exi: Exi::dummy(),
            gx: Gx::new(),
            cp: Cp::new(),
            di: Di::new(),
            si: Si::new(),
            ai: Ai::new(),
            mi: Mi::new(),
            idle: IdleDetector::new(idle_skip),

            #[cfg(feature = "scripting")]
            script_host: None,
            #[cfg(feature = "scripting")]
            script_hook_flags: HookFlags::empty(),
            #[cfg(feature = "scripting")]
            script_hook_filters: ScriptHookFilters::default(),
        }
    }

    pub fn with_image(exe: &impl Executable, idle_skip: bool) -> Self {
        let mut gekko = Gekko::new(exe.entry_point(), idle_skip);
        let data = exe.data();

        // Copy TEXT sections to memory
        for section in exe.text_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                gekko.mmio.virt_write_u8(addr, value);
            }
        }

        // Copy DATA sections to memory
        for section in exe.data_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                gekko.mmio.virt_write_u8(addr, value);
            }
        }

        // Zero out the BSS section
        let (bss_start, bss_size) = exe.bss();
        for i in 0..bss_size {
            let addr = bss_start + i;
            gekko.mmio.virt_write_u8(addr, 0);
        }

        gekko
    }

    pub fn with_ipl(ipl: &[u8], idle_skip: bool) -> Self {
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

        let mut gekko = Gekko::new(IPL_RESET_VECTOR, idle_skip);
        gekko.cpu.msr.set_ip(true);
        gekko.mmio.ipl = ipl.to_vec();
        gekko.exi.attach_device(
            ExiMacronix::CHANNEL,
            ExiMacronix::DEVICE,
            Box::new(ExiMacronix::new(ipl.to_vec())),
        );
        // TODO: this makes 0x8130107C (NTSC BS2) exit the DVD state machine
        // as it forces it to enter "state 19"
        gekko.open_cover();
        gekko
    }

    #[inline]
    pub fn step(&mut self) {
        // Fire any events that are due
        while let Some(event) = self.scheduler.poll() {
            match event {
                EventKind::VSync => {
                    self.vsync_pending = true;
                    let next = self.scheduler.cycles + CYCLES_PER_VSYNC;
                    self.scheduler.schedule_at(next, EventKind::VSync);
                }
                EventKind::ViHalfLine => {
                    self.vi.on_half_line(self.scheduler.cycles);
                    self.vi.half_line_scheduled = false;
                    self.maybe_schedule_vi_half_line();
                    self.check_vi_interrupts();
                }
            }
        }

        // Deliver external interrupt when EE=1 and any enabled PI interrupt is pending
        if self.cpu.msr.external_interrupt_enable() && self.pi.interrupt_pending() {
            self.cause_external_interrupt();
            self.scheduler.cycles += 1;
            return;
        }

        // TODO: hack IPL state machine
        // if self.cpu.pc == 0x81301284 {
        //     self.cpu.pc += 4;
        // }

        // if self.cpu.pc == 0x81300BD8 {
        //     self.cpu.pc += 4;
        // }

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

    pub fn run_until_vsync(&mut self) {
        self.vsync_pending = false;
        // Update SI controller input at the start of each frame
        self.si.update_polling();
        self.check_si_interrupts();
        while !self.vsync_pending {
            self.step();
        }
    }

    /// Read the instructions in `[start, end]` and check whether the loop is a
    /// side effect free MMIO polling loop that can safely be skipped.
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
