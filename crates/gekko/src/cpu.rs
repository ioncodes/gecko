pub mod interpreter;
pub mod semantics;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/gekko_lut.rs"));
}

pub struct Cpu {
    pub gprs: [u32; 32],
    pub fprs: [f64; 32],
    pub current_pc: u32,
    pub next_pc: u32,
    pub pc: u32,
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            gprs: [0; 32],
            fprs: [0.0; 32],
            current_pc: 0x100,
            next_pc: 0x104,
            pc: 0x100,
        }
    }

    pub fn read_gpr(&self, reg: usize) -> u32 {
        self.gprs[reg]
    }

    pub fn write_gpr(&mut self, reg: usize, value: u32) {
        self.gprs[reg] = value;
    }

    pub fn read_fpr(&self, reg: usize) -> f64 {
        self.fprs[reg]
    }

    pub fn write_fpr(&mut self, reg: usize, value: f64) {
        self.fprs[reg] = value;
    }
}