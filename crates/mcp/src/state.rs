use std::sync::{Arc, Condvar, Mutex};

use backend_wgpu::GxRenderer;
use gecko::system::{GC, System, SystemId, WII};

use crate::sink::Introspection;

pub enum Backend {
    Gc(System<{ GC }>),
    Wii(System<{ WII }>),
}

impl Backend {
    pub fn system_id(&self) -> SystemId {
        match self {
            Self::Gc(_) => GC,
            Self::Wii(_) => WII,
        }
    }

    pub fn run_until_vsync(&mut self) {
        match self {
            Self::Gc(s) => s.run_until_vsync(),
            Self::Wii(s) => s.run_until_vsync(),
        }
    }

    pub fn step_cpu(&mut self) {
        match self {
            Self::Gc(s) => s.step_cpu(),
            Self::Wii(s) => s.step_cpu(),
        }
    }

    pub fn step(&mut self) {
        match self {
            Self::Gc(s) => s.step(),
            Self::Wii(s) => s.step(),
        }
    }

    pub fn pc(&self) -> u32 {
        match self {
            Self::Gc(s) => s.gekko.pc,
            Self::Wii(s) => s.gekko.pc,
        }
    }

    pub fn cycles(&self) -> u64 {
        match self {
            Self::Gc(s) => s.scheduler.cycles,
            Self::Wii(s) => s.scheduler.cycles,
        }
    }

    pub fn apply_host_input(&mut self, input: &gecko::HostInput) {
        match self {
            Self::Gc(s) => s.apply_host_input(input),
            Self::Wii(s) => s.apply_host_input(input),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Paused,
    Running,
}

pub struct EmuState {
    pub backend: Option<Backend>,
    pub run_mode: RunMode,
    pub game_name: String,
    pub game_code: String,
    pub max_run_until_cycles: u64,
}

impl EmuState {
    pub fn new() -> Self {
        Self {
            backend: None,
            run_mode: RunMode::Paused,
            game_name: String::new(),
            game_code: String::new(),
            max_run_until_cycles: 1_000_000_000,
        }
    }
}

pub struct Shared {
    pub state: Mutex<EmuState>,
    pub cv: Condvar,
    pub gx: Arc<Mutex<GxRenderer>>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub introspect: Arc<Mutex<Introspection>>,
    pub ipl: Vec<u8>,
    pub dsp_rom: Vec<u8>,
    pub coef_rom: Vec<u8>,
}

impl Shared {
    pub fn set_run_mode(self: &Arc<Self>, mode: RunMode) {
        self.state.lock().unwrap().run_mode = mode;
        self.cv.notify_all();
    }
}
