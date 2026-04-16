use std::fs::File;
use std::io::BufWriter;

use dbglib::Debugger;
use image::symbols::SymbolTable;

const CPU_TRACE_FILENAME: &str = "cpu_trace.log";
const DSP_TRACE_FILENAME: &str = "dsp_trace.log";
const DEFAULT_LUA_SCRIPT: &str = include_str!("../../../scripts/ipl_state_dump.lua");

pub struct DebuggerUi {
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
    pub show_lua: bool,
    pub show_breakpoints: bool,
    pub memory_base: u32,
    pub memory_addr_input: String,
    pub run_until_addr_input: String,
    pub breakpoint_addr_input: String,
    pub dvd_cover_open: Option<bool>,
    pub lua_source: String,
    pub lua_log: Vec<String>,
    pub lua_load_pending: bool,
}

impl Default for DebuggerUi {
    fn default() -> Self {
        DebuggerUi {
            debugger: Debugger::new(),
            symbols: None,
            show_cpu: true,
            show_dsp: false,
            show_controls: true,
            show_gx_state: false,
            show_mmio: false,
            show_dvd: false,
            show_exi: false,
            show_irqs: false,
            show_callstack: false,
            show_lua: false,
            show_breakpoints: false,
            memory_base: 0x8000_0000,
            memory_addr_input: "80000000".to_string(),
            run_until_addr_input: String::new(),
            breakpoint_addr_input: String::new(),
            dvd_cover_open: None,
            lua_source: DEFAULT_LUA_SCRIPT.to_string(),
            lua_log: Vec::new(),
            lua_load_pending: false,
        }
    }
}

impl DebuggerUi {
    pub fn start_trace(&mut self) {
        let file = File::create(CPU_TRACE_FILENAME).expect("failed to create CPU trace file");
        self.debugger.start_trace(Box::new(BufWriter::new(file)));
    }

    pub fn stop_trace(&mut self) {
        self.debugger.stop_trace();
    }

    pub fn start_dsp_trace(&mut self) {
        let file = File::create(DSP_TRACE_FILENAME).expect("failed to create DSP trace file");
        self.debugger.start_dsp_trace(Box::new(BufWriter::new(file)));
    }

    pub fn stop_dsp_trace(&mut self) {
        self.debugger.stop_dsp_trace();
    }
}
