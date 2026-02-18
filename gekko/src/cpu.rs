pub mod interpreter;

pub struct Cpu {
    pub gprs: [u32; 32],
    pub fprs: [f64; 32],
    pub pc: u32,
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            gprs: [0; 32],
            fprs: [0.0; 32],
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