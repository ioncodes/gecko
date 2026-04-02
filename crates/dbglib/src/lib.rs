pub mod trace;
pub mod windows;

#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;

use gecko::gamecube::GameCube;
use gecko::scheduler::EventKind;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum EmulatorState {
    Running,
    Paused,
    Step,
    StepDsp,
    RunUntilVsync,
    RunUntilAddress(u32),
    RunUntilDsp,
}

pub struct Debugger {
    state: EmulatorState,
    #[cfg(not(target_arch = "wasm32"))]
    trace_writer: Option<Box<dyn Write>>,
    #[cfg(not(target_arch = "wasm32"))]
    dsp_trace_writer: Option<Box<dyn Write>>,
}

impl Debugger {
    pub fn new() -> Self {
        Debugger {
            state: EmulatorState::Paused,
            #[cfg(not(target_arch = "wasm32"))]
            trace_writer: None,
            #[cfg(not(target_arch = "wasm32"))]
            dsp_trace_writer: None,
        }
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
        while let Some(event) = emulator.scheduler.poll() {
            if event == EventKind::DspTick {
                self.dsp_trace_step(emulator);
            }
            emulator.process_event(event);
        }
    }

    /// Drain and process scheduler events, returning `true` if a DSP tick was
    /// processed. Used by `RunUntilDsp` to detect when the DSP is about to
    /// execute.
    #[inline(always)]
    fn drain_events_until_dsp(&mut self, emulator: &mut GameCube) -> bool {
        let mut dsp_hit = false;
        while let Some(event) = emulator.scheduler.poll() {
            if event == EventKind::DspTick {
                self.dsp_trace_step(emulator);
                dsp_hit = true;
            }
            emulator.process_event(event);
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
                if self.is_tracing() || self.is_dsp_tracing() {
                    emulator.prepare_frame();
                    while !emulator.vsync_pending {
                        self.drain_events(emulator);
                        self.trace_step(emulator);
                        emulator.step_cpu();
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
            EmulatorState::StepDsp => {
                self.dsp_trace_step(emulator);
                emulator.tick_dsp();
                self.state = EmulatorState::Paused;
            }
            EmulatorState::RunUntilVsync => {
                if self.is_tracing() || self.is_dsp_tracing() {
                    emulator.prepare_frame();
                    while !emulator.vsync_pending {
                        self.drain_events(emulator);
                        self.trace_step(emulator);
                        emulator.step_cpu();
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
