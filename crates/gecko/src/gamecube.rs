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
#[cfg(feature = "hooks")]
use crate::hooks::{HookFilters, HookFlags, HookState, Host};
use crate::host::{EmptyRenderSink, RenderSink};
#[cfg(feature = "idle-skip")]
use crate::idle::{IDLE_LOOP_MAX_INSTRS, IdleCheck, IdleDetector};
use crate::ipl::IPL_HLE;
use crate::mmio::Mmio;
use crate::scheduler::Scheduler;
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

    /// GX dispatches actions here.
    pub render_sink: Box<dyn RenderSink>,

    #[cfg(feature = "idle-skip")]
    idle: IdleDetector,

    #[cfg(feature = "hooks")]
    pub hook_host: Option<Box<dyn Host + Send>>,
    #[cfg(feature = "hooks")]
    pub hook_flags: HookFlags,
    #[cfg(feature = "hooks")]
    pub hook_filters: HookFilters,
}

impl GameCube {
    #[cfg(feature = "hooks")]
    #[inline(always)]
    pub fn apply_hook_state(&mut self, state: HookState) {
        self.hook_flags = state.flags;
        self.hook_filters = state.filters;
    }

    #[cfg(feature = "hooks")]
    #[inline(always)]
    pub fn sync_pending_hook_state(&mut self, host: &mut dyn Host) {
        #[cfg(feature = "hooks-mut-traps")]
        match host.take_pending_hook_state() {
            Ok(Some(state)) => self.apply_hook_state(state),
            Ok(None) => {}
            Err(err) => tracing::error!(target: "script", error = %err, "failed to refresh script traps"),
        }

        #[cfg(not(feature = "hooks-mut-traps"))]
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

            render_sink: Box::new(EmptyRenderSink),

            #[cfg(feature = "idle-skip")]
            idle: IdleDetector::new(),

            #[cfg(feature = "hooks")]
            hook_host: None,
            #[cfg(feature = "hooks")]
            hook_flags: HookFlags::empty(),
            #[cfg(feature = "hooks")]
            hook_filters: HookFilters::default(),
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

    pub fn with_ipl_hle(dvd: image::dvd::Dvd) -> Self {
        const APPLOADER_LOAD: u32 = 0x0120_0000;
        const IPL_LOAD: u32 = 0x0130_0000;
        const IPL_ENTRY: u32 = 0x8130_0000;
        const ARAM_SIZE: u32 = 16 * 1024 * 1024;

        let game_name = String::from_utf8_lossy(&dvd.header.game_name);
        let game_name = game_name.trim_end_matches('\0');
        tracing::info!("Game: {game_name}");

        let apploader_version = String::from_utf8_lossy(&dvd.apploader.timestamp);
        tracing::info!("Apploader: {apploader_version}");

        let mut emulator = Self::new(IPL_ENTRY);

        // BATs
        emulator.cpu.spr.dbat0u = 0x8000_1FFF;
        emulator.cpu.spr.dbat0l = 0x0000_0002;
        emulator.cpu.spr.dbat1u = 0xC000_1FFF;
        emulator.cpu.spr.dbat1l = 0x0000_002A;
        emulator.cpu.spr.dbat2u = 0x0000_1FFF;
        emulator.cpu.spr.dbat2l = 0x0000_0002;
        emulator.cpu.spr.dbat3u = 0xFFF0_001F;
        emulator.cpu.spr.dbat3l = 0xFFF0_0001;
        emulator.cpu.spr.ibat0u = 0x8000_1FFF;
        emulator.cpu.spr.ibat0l = 0x0000_0002;
        emulator.cpu.spr.ibat2u = 0x0000_1FFF;
        emulator.cpu.spr.ibat2l = 0x0000_0002;
        emulator.cpu.spr.ibat3u = 0xFFF0_001F;
        emulator.cpu.spr.ibat3l = 0xFFF0_0001;

        // DVD header fields to low memory
        emulator.mmio.ram[0x00..0x04].copy_from_slice(&dvd.header.game_code);
        emulator.mmio.ram[0x04..0x06].copy_from_slice(&dvd.header.maker_code);
        emulator.mmio.ram[0x06] = dvd.header.disk_id;
        emulator.mmio.ram[0x07] = dvd.header.version;
        emulator.mmio.ram[0x08] = dvd.header.audio_streaming;
        emulator.mmio.ram[0x09] = dvd.header.streaming_buffer_size;

        // System info
        emulator
            .mmio
            .phys_write_u32(0x28, crate::mmio::constants::RAM_SIZE as u32);
        emulator.mmio.phys_write_u32(0x2C, 1); // retail console
        emulator.mmio.phys_write_u32(0xD0, ARAM_SIZE);

        // Syscall stub? rfi
        emulator.mmio.phys_write_u32(0x0C00, 0x4C00_0064);

        // MSR: FP + address translation
        emulator.cpu.msr.set_floating_point_available(true);
        emulator.cpu.msr.set_data_address_translation(true);
        emulator.cpu.msr.set_instruction_address_translation(true);

        // ???
        emulator.cpu.spr.hid0 = 0x0011_C464;

        // Load apploader code into RAM
        let apploader_code_start = image::dvd::DVD_APPLOADER_OFFSET + 0x20;
        let apploader_size = (dvd.apploader.size.get() + dvd.apploader.trailer_size.get()) as usize;
        let apploader_entry = dvd.apploader.entrypoint.get();
        emulator.mmio.ram[APPLOADER_LOAD as usize..][..apploader_size]
            .copy_from_slice(&dvd.data()[apploader_code_start..][..apploader_size]);

        // Load custom IPL binary into RAM
        emulator.mmio.ram[IPL_LOAD as usize..][..IPL_HLE.len()].copy_from_slice(IPL_HLE);

        // R0 = apploader entry (read by IPL)
        emulator.cpu.gprs[0] = apploader_entry;

        // Insert the DVD
        emulator.insert_dvd(dvd);

        emulator
    }

    pub fn with_ipl(ipl: &[u8], skip: bool) -> Self {
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

        let mut ipl = ipl.to_vec();
        if skip {
            crate::ipl::apply_skip_patch(&mut ipl);
        }

        let mut emulator = GameCube::new(IPL_RESET_VECTOR);
        emulator.cpu.msr.set_ip(true);
        emulator.mmio.ipl = ipl.clone();
        emulator.exi.attach_device(
            ExiMacronix::CHANNEL,
            ExiMacronix::DEVICE,
            Box::new(ExiMacronix::new(ipl)),
        );
        // TODO: this makes 0x8130107C (NTSC BS2) exit the DVD state machine
        // as it forces it to enter "state 19"
        emulator.open_cover();
        emulator
    }

    #[inline(always)]
    pub fn step_cpu(&mut self) {
        if self.cpu.msr.external_interrupt_enable() {
            // Deliver external interrupt when EE=1 and any enabled PI interrupt is pending
            if self.pi.interrupt_pending() {
                self.cause_external_interrupt();
                self.scheduler.cycles += 2;
                return;
            }

            if self.cpu.dec.interrupt_pending() {
                self.cause_decrementer_interrupt();
                self.scheduler.cycles += 2;
                return;
            }
        }

        // CPU pre-hook
        #[cfg(feature = "hooks")]
        if self.hook_flags.contains(HookFlags::CPU_PRE) {
            let pc = self.cpu.pc;
            if self.hook_filters.cpu_pre.matches(pc) {
                if let Some(mut host) = self.hook_host.take() {
                    host.on_cpu_pre(self);
                    self.sync_pending_hook_state(host.as_mut());
                    self.hook_host = Some(host);
                }
            }
        }

        // Fetch and execute next instruction
        self.cpu.cia = self.cpu.pc;
        self.cpu.nia = self.cpu.cia.wrapping_add(4);
        let instr = Instruction(self.mmio.fetch_instruction(self.cpu.cia));
        cpu::lut::dispatch(self, instr);
        self.scheduler.cycles += 2; // TODO: Track properly?

        // CPU post-hook
        #[cfg(feature = "hooks")]
        if self.hook_flags.contains(HookFlags::CPU_POST) {
            let pc = self.cpu.cia;
            if self.hook_filters.cpu_post.matches(pc) {
                if let Some(mut host) = self.hook_host.take() {
                    host.on_cpu_post(self);
                    self.sync_pending_hook_state(host.as_mut());
                    self.hook_host = Some(host);
                }
            }
        }

        #[cfg(feature = "idle-skip")]
        match self.idle.check(self.cpu.cia, self.cpu.nia) {
            IdleCheck::Skip => {
                if let Some(deadline) = self.scheduler.next_deadline() {
                    self.scheduler.cycles = deadline;
                }
            }
            IdleCheck::Validate { start, end } => {
                let safe = self.is_polling_loop(start, end);
                self.idle.set_validated(safe);
                if safe {
                    if let Some(deadline) = self.scheduler.next_deadline() {
                        self.scheduler.cycles = deadline;
                    }
                }
            }
            IdleCheck::Continue => {}
        }

        self.cpu.pc = self.cpu.nia;
    }

    /// Drain pending scheduler events, then execute one CPU instruction.
    #[inline(always)]
    pub fn step(&mut self) {
        while let Some(f) = self.scheduler.poll() {
            f(self);
        }
        self.step_cpu();
    }

    #[inline(always)]
    pub fn prepare_frame(&mut self) {
        self.vsync_pending = false;
        self.si.update_polling();
        crate::flipper::si::refresh_interrupts(self);
    }

    pub fn run_until_vsync(&mut self) {
        self.prepare_frame();
        while !self.vsync_pending {
            self.scheduler.refresh_deadline();
            // Execute a slice of CPU instructions until the next event deadline
            while self.scheduler.cycles < self.scheduler.next_deadline() {
                self.step_cpu();
            }
            // Drain all events that are now due
            while let Some(f) = self.scheduler.poll() {
                f(self);
            }
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

    #[cfg(feature = "hooks")]
    pub fn set_hook_host(&mut self, host: Box<dyn Host + Send>) {
        self.apply_hook_state(host.hook_state());
        self.hook_host = Some(host);
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
