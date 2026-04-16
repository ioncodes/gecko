pub mod trace;
pub mod windows;

#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;

use gecko::gamecube::GameCube;
use gecko::scheduler::{CPU_CYCLES_PER_DSP_TICK, DSP_BATCH_SIZE, ScheduledFn, dsp_batch_handler};

/// Identify the DSP batch handler so the debugger can intercept it for per-instruction tracing.
#[inline(always)]
fn is_dsp_batch(f: ScheduledFn) -> bool {
    (f as usize) == (dsp_batch_handler as ScheduledFn as usize)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum EmulatorState {
    Running,
    Paused,
    Step,
    RunUntilVsync,
    RunUntilAddress(u32),
    RunUntilDsp,
}

#[derive(Debug, Clone, Copy)]
pub struct Breakpoint {
    pub addr: u32,
    pub enabled: bool,
}

pub struct Debugger {
    state: EmulatorState,
    breakpoints: Vec<Breakpoint>,
    #[cfg(not(target_arch = "wasm32"))]
    trace_writer: Option<Box<dyn Write>>,
    #[cfg(not(target_arch = "wasm32"))]
    dsp_trace_writer: Option<Box<dyn Write>>,
}

impl Debugger {
    pub fn new() -> Self {
        Debugger {
            state: EmulatorState::Paused,
            breakpoints: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            trace_writer: None,
            #[cfg(not(target_arch = "wasm32"))]
            dsp_trace_writer: None,
        }
    }

    pub fn breakpoints(&self) -> &[Breakpoint] {
        &self.breakpoints
    }

    pub fn add_breakpoint(&mut self, addr: u32) {
        if !self.breakpoints.iter().any(|b| b.addr == addr) {
            self.breakpoints.push(Breakpoint { addr, enabled: true });
        }
    }

    pub fn remove_breakpoint(&mut self, index: usize) {
        if index < self.breakpoints.len() {
            self.breakpoints.remove(index);
        }
    }

    pub fn toggle_breakpoint(&mut self, index: usize) {
        if let Some(bp) = self.breakpoints.get_mut(index) {
            bp.enabled = !bp.enabled;
        }
    }

    #[inline(always)]
    fn breakpoint_hit(&self, pc: u32) -> bool {
        self.breakpoints.iter().any(|b| b.enabled && b.addr == pc)
    }

    #[inline(always)]
    fn has_active_breakpoints(&self) -> bool {
        self.breakpoints.iter().any(|b| b.enabled)
    }

    pub fn state(&self) -> EmulatorState {
        self.state
    }

    pub fn set_state(&mut self, state: EmulatorState) {
        self.state = state;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn is_tracing(&self) -> bool {
        self.trace_writer.is_some()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn is_tracing(&self) -> bool {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_trace(&mut self, writer: Box<dyn Write>) {
        self.trace_writer = Some(writer);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn stop_trace(&mut self) {
        if let Some(mut w) = self.trace_writer.take() {
            let _ = w.flush();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn trace_step(&mut self, emulator: &GameCube) {
        if let Some(ref mut writer) = self.trace_writer {
            let line = trace::format_trace_line(emulator);
            let _ = writeln!(writer, "{}", line);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn trace_step(&mut self, _emulator: &GameCube) {}

    #[cfg(not(target_arch = "wasm32"))]
    pub fn is_dsp_tracing(&self) -> bool {
        self.dsp_trace_writer.is_some()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn is_dsp_tracing(&self) -> bool {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_dsp_trace(&mut self, writer: Box<dyn Write>) {
        self.dsp_trace_writer = Some(writer);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn stop_dsp_trace(&mut self) {
        if let Some(mut w) = self.dsp_trace_writer.take() {
            let _ = w.flush();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dsp_trace_step(&mut self, emulator: &GameCube) {
        if let Some(ref mut writer) = self.dsp_trace_writer {
            let line = trace::format_dsp_trace_line(&emulator.dsp);
            let _ = writeln!(writer, "{}", line);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn dsp_trace_step(&mut self, _emulator: &GameCube) {}

    /// Drain and process scheduler events, tracing DSP ticks when active.
    #[inline(always)]
    fn drain_events(&mut self, emulator: &mut GameCube) {
        while let Some(f) = emulator.scheduler.poll() {
            if is_dsp_batch(f) && self.is_dsp_tracing() {
                for _ in 0..DSP_BATCH_SIZE {
                    self.dsp_trace_step(emulator);
                    if !emulator.step_dsp_instruction() {
                        break;
                    }
                }
                gecko::flipper::dsp::refresh_interrupts(emulator);
                emulator
                    .scheduler
                    .schedule_in(CPU_CYCLES_PER_DSP_TICK * DSP_BATCH_SIZE, dsp_batch_handler);
            } else {
                f(emulator);
            }
        }
    }

    /// Drain and process scheduler events, returning `true` if a DSP tick was
    /// processed. Used by `RunUntilDsp` to detect when the DSP is about to
    /// execute.
    #[inline(always)]
    fn drain_events_until_dsp(&mut self, emulator: &mut GameCube) -> bool {
        let mut dsp_hit = false;
        while let Some(f) = emulator.scheduler.poll() {
            if is_dsp_batch(f) {
                if self.is_dsp_tracing() {
                    for _ in 0..DSP_BATCH_SIZE {
                        self.dsp_trace_step(emulator);
                        if !emulator.step_dsp_instruction() {
                            break;
                        }
                    }
                    gecko::flipper::dsp::refresh_interrupts(emulator);
                    emulator
                        .scheduler
                        .schedule_in(CPU_CYCLES_PER_DSP_TICK * DSP_BATCH_SIZE, dsp_batch_handler);
                } else {
                    f(emulator);
                }
                dsp_hit = true;
            } else {
                f(emulator);
            }
        }
        dsp_hit
    }

    /// Execute one frame's worth of emulation based on the current state.
    ///
    /// After execution, transient states (`Step`, `RunUntilVsync`, `RunUntilAddress`)
    /// automatically transition to `Paused`.
    pub fn tick(&mut self, emulator: &mut GameCube) {
        match self.state {
            EmulatorState::Running => {
                if self.is_tracing() || self.is_dsp_tracing() || self.has_active_breakpoints() {
                    emulator.prepare_frame();
                    while !emulator.vsync_pending {
                        self.drain_events(emulator);
                        self.trace_step(emulator);
                        emulator.step_cpu();
                        if self.breakpoint_hit(emulator.cpu.pc) {
                            self.state = EmulatorState::Paused;
                            return;
                        }
                    }
                } else {
                    emulator.run_until_vsync();
                }
            }
            EmulatorState::Step => {
                self.trace_step(emulator);
                emulator.step();
                self.state = EmulatorState::Paused;
            }
            EmulatorState::RunUntilVsync => {
                if self.is_tracing() || self.is_dsp_tracing() || self.has_active_breakpoints() {
                    emulator.prepare_frame();
                    while !emulator.vsync_pending {
                        self.drain_events(emulator);
                        self.trace_step(emulator);
                        emulator.step_cpu();
                        if self.breakpoint_hit(emulator.cpu.pc) {
                            self.state = EmulatorState::Paused;
                            return;
                        }
                    }
                } else {
                    emulator.run_until_vsync();
                }
                self.state = EmulatorState::Paused;
            }
            EmulatorState::RunUntilAddress(addr) => {
                while emulator.cpu.pc != addr {
                    self.drain_events(emulator);
                    self.trace_step(emulator);
                    emulator.step_cpu();
                    if self.breakpoint_hit(emulator.cpu.pc) {
                        self.state = EmulatorState::Paused;
                        return;
                    }
                }
                self.state = EmulatorState::Paused;
            }
            EmulatorState::RunUntilDsp => {
                loop {
                    let hit = self.drain_events_until_dsp(emulator);
                    if hit {
                        break;
                    }
                    self.trace_step(emulator);
                    emulator.step_cpu();
                    if self.breakpoint_hit(emulator.cpu.pc) {
                        self.state = EmulatorState::Paused;
                        return;
                    }
                }
                self.state = EmulatorState::Paused;
            }
            EmulatorState::Paused => {}
        }
    }
}

impl Default for Debugger {
    fn default() -> Self {
        Self::new()
    }
}
