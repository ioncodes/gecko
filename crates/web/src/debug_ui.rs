use dbglib::{Debugger, EmulatorState};
use gecko::gamecube::GameCube;
use image::symbols::SymbolTable;

pub struct DebugState {
    pub debugger: Debugger,
    pub symbols: Option<SymbolTable>,
    pub show_cpu: bool,
    pub show_dsp: bool,
    pub show_gx_state: bool,
    pub show_mmio: bool,
    pub show_dvd: bool,
    pub show_exi: bool,
    pub show_irqs: bool,
    pub show_controls: bool,
    pub show_callstack: bool,
    pub show_breakpoints: bool,
    pub memory_base: u32,
    pub memory_addr_input: String,
    pub run_until_addr_input: String,
    pub breakpoint_addr_input: String,
    pub dvd_cover_open: Option<bool>,
}

impl Default for DebugState {
    fn default() -> Self {
        let mut debugger = Debugger::new();
        debugger.set_state(EmulatorState::Running);
        DebugState {
            debugger,
            symbols: None,
            show_cpu: false,
            show_dsp: false,
            show_controls: true,
            show_gx_state: false,
            show_mmio: false,
            show_dvd: false,
            show_exi: false,
            show_irqs: false,
            show_callstack: false,
            show_breakpoints: false,
            memory_base: 0x8000_0000,
            memory_addr_input: "80000000".to_string(),
            run_until_addr_input: String::new(),
            breakpoint_addr_input: String::new(),
            dvd_cover_open: None,
        }
    }
}

impl DebugState {
    pub fn tick(&mut self, emulator: &mut GameCube) {
        if let Some(open) = self.dvd_cover_open.take() {
            if open {
                emulator.open_cover();
            } else {
                emulator.close_cover();
            }
        }
        self.debugger.tick(emulator);
    }

    pub fn show(&mut self, ctx: &egui::Context, emulator: &GameCube) {
        egui::Window::new("Debug")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.show_controls, "Controls");
                    ui.checkbox(&mut self.show_cpu, "CPU");
                    ui.checkbox(&mut self.show_callstack, "Call Stack");
                    ui.checkbox(&mut self.show_dsp, "DSP");
                    ui.checkbox(&mut self.show_gx_state, "GX");
                    ui.checkbox(&mut self.show_mmio, "MMIO");
                    ui.checkbox(&mut self.show_dvd, "DVD");
                    ui.checkbox(&mut self.show_exi, "EXI");
                    ui.checkbox(&mut self.show_irqs, "IRQ");
                    ui.checkbox(&mut self.show_breakpoints, "Breakpoints");
                });
            });

        if self.show_cpu {
            dbglib::windows::cpu::show_cpu(
                ctx,
                &mut self.show_cpu,
                &emulator.cpu,
                &emulator.mmio,
                self.symbols.as_ref(),
                self.debugger.breakpoints(),
            );
        }
        if self.show_callstack {
            dbglib::windows::callstack::show_callstack(
                ctx,
                &mut self.show_callstack,
                &emulator.cpu,
                &emulator.mmio,
                self.symbols.as_ref(),
            );
        }
        if self.show_dsp {
            dbglib::windows::dsp::show_dsp(ctx, &mut self.show_dsp, &emulator.dsp);
        }
        if self.show_controls {
            let mut start_trace = false;
            let mut stop_trace = false;
            let mut start_dsp_trace = false;
            let mut stop_dsp_trace = false;
            let tracing = self.debugger.is_tracing();
            let dsp_tracing = self.debugger.is_dsp_tracing();
            let mut state = self.debugger.state();
            dbglib::windows::controls::show_controls(
                ctx,
                &mut self.show_controls,
                &mut state,
                &mut self.run_until_addr_input,
                &mut self.dvd_cover_open,
                tracing,
                &mut start_trace,
                &mut stop_trace,
                dsp_tracing,
                &mut start_dsp_trace,
                &mut stop_dsp_trace,
            );
            self.debugger.set_state(state);
        }
        if self.show_gx_state {
            dbglib::windows::gx::show_gx(ctx, &mut self.show_gx_state, &emulator.gx, &emulator.mmio);
        }
        if self.show_mmio {
            dbglib::windows::mmio::show_mmio(
                ctx,
                &mut self.show_mmio,
                &mut self.memory_base,
                &mut self.memory_addr_input,
                &emulator.mmio,
            );
        }
        if self.show_dvd {
            dbglib::windows::dvd::show_dvd(ctx, &mut self.show_dvd, &emulator.di);
        }
        if self.show_exi {
            dbglib::windows::exi::show_exi(ctx, &mut self.show_exi, &emulator.exi);
        }
        if self.show_irqs {
            dbglib::windows::irq::show_irq(ctx, &mut self.show_irqs, &emulator.cpu, &emulator.pi);
        }
        if self.show_breakpoints {
            dbglib::windows::breakpoints::show_breakpoints(
                ctx,
                &mut self.show_breakpoints,
                &mut self.debugger,
                &mut self.breakpoint_addr_input,
            );
        }
    }
}
