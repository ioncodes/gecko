use crate::HostInput;
use crate::audio::{AudioSink, EmptyAudioSink};
use crate::dvd::DvdInterface;
use crate::flipper::ai::AudioInterface;
use crate::flipper::cp::CommandProcessor;
use crate::flipper::dsp::Dsp;
use crate::flipper::exi::ExternalInterface;
use crate::flipper::gx::GraphicsProcessor;
use crate::flipper::mi::MemoryInterface;
use crate::flipper::pe::PixelEngine;
use crate::flipper::pi::ProcessorInterface;
use crate::flipper::si::SerialInterface;
use crate::flipper::vi::VideoInterface;
use crate::gekko::Gekko;
use crate::hollywood::Hollywood;
#[cfg(feature = "hooks")]
use crate::hooks::{HookFilters, HookFlags, HookState, Host};
use crate::host::{EmptyRenderSink, RenderSink};
use crate::mmio::Mmio;
use crate::scheduler::Scheduler;
use crate::starlet::Starlet;
use image::Executable;

pub type SystemId = u8;

pub const GC: SystemId = 0;
pub const WII: SystemId = 1;

pub struct System<const SYSTEM: SystemId> {
    pub vsync_pending: bool,
    pub gekko: Gekko,
    pub scheduler: Scheduler<SYSTEM>,
    pub mmio: Mmio<SYSTEM>,
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

    // Wii stuff.
    pub starlet: Starlet,
    pub hollywood: Hollywood,

    /// GX dispatches actions here.
    pub render_sink: Box<dyn RenderSink>,

    /// AID DMA pushes 8-frame stereo s16 blocks here.
    pub audio_sink: Box<dyn AudioSink>,

    #[cfg(feature = "hooks")]
    pub hook_host: Option<Box<dyn Host<SYSTEM> + Send>>,
    #[cfg(feature = "hooks")]
    pub hook_flags: HookFlags,
    #[cfg(feature = "hooks")]
    pub hook_filters: HookFilters,

    #[cfg(feature = "jit")]
    pub jit: Option<Box<crate::gekko::jit::JitEngine<SYSTEM>>>,
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    pub(crate) fn with_scheduler(entrypoint: u32, scheduler: Scheduler<SYSTEM>) -> Self {
        System {
            vsync_pending: false,
            gekko: Gekko::new(entrypoint),
            scheduler,
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

            starlet: Starlet::new(),
            hollywood: Hollywood::new(),

            render_sink: Box::new(EmptyRenderSink),
            audio_sink: Box::new(EmptyAudioSink),

            #[cfg(feature = "hooks")]
            hook_host: None,
            #[cfg(feature = "hooks")]
            hook_flags: HookFlags::empty(),
            #[cfg(feature = "hooks")]
            hook_filters: HookFilters::default(),

            #[cfg(feature = "jit")]
            jit: None,
        }
    }

    #[inline(always)]
    pub fn step_cpu(&mut self) {
        if self.gekko.msr.external_interrupt_enable() {
            // Deliver external interrupt when EE=1 and any enabled PI interrupt is pending
            if self.pi.interrupt_pending() {
                self.cause_external_interrupt();
                self.scheduler.cycles += 2;
                return;
            }

            if self.gekko.dec.interrupt_pending() {
                self.cause_decrementer_interrupt();
                self.scheduler.cycles += 2;
                return;
            }
        }

        // CPU pre-hook
        #[cfg(feature = "hooks")]
        if self.hook_flags.contains(HookFlags::CPU_PRE) {
            let pc = self.gekko.pc;
            if self.hook_filters.cpu_pre.matches(pc) {
                if let Some(mut host) = self.hook_host.take() {
                    host.on_cpu_pre(self);
                    self.sync_pending_hook_state(host.as_mut());
                    self.hook_host = Some(host);
                }
            }
        }

        // Fetch and execute next instruction
        self.gekko.cia = self.gekko.pc;
        self.gekko.nia = self.gekko.cia.wrapping_add(4);
        let instr = crate::gekko::instruction::Instruction(self.mmio.fetch_instruction(self.gekko.cia));
        crate::gekko::dispatch(self, instr);
        self.scheduler.cycles += 2; // TODO: Track properly?

        // CPU post-hook
        #[cfg(feature = "hooks")]
        if self.hook_flags.contains(HookFlags::CPU_POST) {
            let pc = self.gekko.cia;
            if self.hook_filters.cpu_post.matches(pc) {
                if let Some(mut host) = self.hook_host.take() {
                    host.on_cpu_post(self);
                    self.sync_pending_hook_state(host.as_mut());
                    self.hook_host = Some(host);
                }
            }
        }

        self.gekko.pc = self.gekko.nia;
    }

    /// Drain pending scheduler events, then execute one CPU instruction.
    #[inline(always)]
    pub fn step(&mut self) {
        self.drain_events();
        self.step_cpu();
    }

    pub fn run_until(&mut self, pc: u32, predicate: impl Fn(&Self) -> bool) {
        self.gekko.pc = pc;
        while !predicate(self) {
            self.step();
        }
    }

    #[inline(always)]
    pub fn prepare_frame(&mut self) {
        self.begin_frame();
        crate::flipper::si::refresh_interrupts(self);
    }

    pub fn run_until_vsync(&mut self) {
        self.prepare_frame();
        while !self.vsync_pending {
            self.scheduler.refresh_deadline();
            #[cfg(feature = "jit")]
            {
                self.run_until_deadline_jit();
            }
            #[cfg(not(feature = "jit"))]
            {
                while self.scheduler.cycles < self.scheduler.next_deadline() {
                    self.step_cpu();
                }
            }
            // Drain all events that are now due
            self.drain_events();
        }
    }

    /// JIT inner loop: runs compiled blocks back-to-back until
    /// `scheduler.cycles >= next_deadline`. Interrupts are checked at block
    /// boundaries.
    #[cfg(feature = "jit")]
    fn run_until_deadline_jit(&mut self) {
        let mut jit = match self.jit.take() {
            Some(jit) => jit,
            None => Box::new(crate::gekko::jit::JitEngine::<SYSTEM>::new()),
        };

        while self.scheduler.cycles < self.scheduler.next_deadline() {
            if self.gekko.msr.external_interrupt_enable() {
                if self.pi.interrupt_pending() {
                    self.cause_external_interrupt();
                    self.scheduler.cycles += 2;
                    continue;
                }

                if self.gekko.dec.interrupt_pending() {
                    self.cause_decrementer_interrupt();
                    self.scheduler.cycles += 2;
                    continue;
                }
            }

            jit.run_block(self);
        }

        self.jit = Some(jit);
    }

    pub fn frame_size(&self) -> (usize, usize) {
        let fmt = self.vi.dcr.video_format();
        (fmt.columns(), fmt.lines())
    }

    pub fn apply_host_input(&mut self, input: &HostInput) {
        match input {
            HostInput::Gc(pad) if SYSTEM == GC => {
                self.si.pad_state[0] = *pad;
            }
            HostInput::Wii {
                wiimote_buttons,
                nunchuk_buttons,
                nunchuk_stick_x,
                nunchuk_stick_y,
            } if SYSTEM == WII => {
                self.starlet.set_wiimote_buttons(*wiimote_buttons);
                self.starlet
                    .set_nunchuk(*nunchuk_buttons, *nunchuk_stick_x, *nunchuk_stick_y);
            }
            _ => unreachable!("invalid host input for system"),
        }
    }

    #[inline(always)]
    pub fn begin_frame(&mut self) {
        self.vsync_pending = false;
        self.si.update_polling();
    }

    #[inline(always)]
    pub fn drain_events(&mut self) {
        while let Some(f) = self.scheduler.poll() {
            f(self);
        }
    }

    pub fn load_image(&mut self, exe: &impl Executable) {
        let data = exe.data();

        // Copy TEXT sections to memory
        for section in exe.text_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                self.mmio.virt_write_u8(addr, value);
            }
        }

        // Copy DATA sections to memory
        for section in exe.data_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                self.mmio.virt_write_u8(addr, value);
            }
        }

        // Zero out the BSS section
        let (bss_start, bss_size) = exe.bss();
        for i in 0..bss_size {
            let addr = bss_start + i;
            self.mmio.virt_write_u8(addr, 0);
        }
    }

    #[cfg(feature = "hooks")]
    #[inline(always)]
    pub fn apply_hook_state(&mut self, state: HookState) {
        self.hook_flags = state.flags;
        self.hook_filters = state.filters;
    }

    #[cfg(feature = "hooks")]
    #[inline(always)]
    pub fn sync_pending_hook_state(&mut self, host: &mut dyn Host<SYSTEM>) {
        #[cfg(feature = "hooks-mut-traps")]
        match host.take_pending_hook_state() {
            Ok(Some(state)) => self.apply_hook_state(state),
            Ok(None) => {}
            Err(err) => tracing::error!(target: "script", error = %err, "failed to refresh script traps"),
        }

        #[cfg(not(feature = "hooks-mut-traps"))]
        let _ = host;
    }

    #[cfg(feature = "hooks")]
    pub fn set_hook_host(&mut self, host: Box<dyn Host<SYSTEM> + Send>) {
        self.apply_hook_state(host.hook_state());
        self.hook_host = Some(host);
    }

    #[cfg(feature = "hooks")]
    pub fn has_hook_host(&self) -> bool {
        self.hook_host.is_some()
    }

    #[cfg(not(feature = "hooks"))]
    pub fn has_hook_host(&self) -> bool {
        false
    }

    #[cfg(all(feature = "hooks", feature = "hooks-mut-traps"))]
    pub fn refresh_hook_traps(&mut self) -> Result<(), String> {
        let Some(mut host) = self.hook_host.take() else {
            return Ok(());
        };

        let refresh_result = host.force_refresh_traps();
        match refresh_result {
            Ok(state) => {
                self.apply_hook_state(state);
                self.hook_host = Some(host);
                Ok(())
            }
            Err(err) => {
                self.hook_host = Some(host);
                Err(err)
            }
        }
    }
}
