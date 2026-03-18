#[derive(PartialEq, Eq, Clone, Copy)]
pub enum EmulatorState {
    Running,
    Paused,
    Step,
    RunUntilVsync,
    RunUntilAddress(u32),
}

pub struct DebuggerUi {
    pub emulator_state: EmulatorState,
    pub show_cpu: bool,
    pub show_gx_state: bool,
    pub show_mmio: bool,
    pub show_exi: bool,
    pub show_irqs: bool,
    pub show_controls: bool,
    pub memory_base: u32,
    pub memory_addr_input: String,
}

impl Default for DebuggerUi {
    fn default() -> Self {
        DebuggerUi {
            emulator_state: EmulatorState::Paused,
            show_cpu: true,
            show_controls: true,
            show_gx_state: false,
            show_mmio: false,
            show_exi: false,
            show_irqs: false,
            memory_base: 0x8000_0000,
            memory_addr_input: "80000000".to_string(),
        }
    }
}
