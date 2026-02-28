use crate::cpu::condition::ConditionRegister;

pub mod interpreter;
pub mod semantics;
pub mod condition;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/gekko_lut.rs"));
}

pub struct Cpu {
    pub gprs: [u32; 32],
    pub fprs: [f64; 32],
    pub pc: u32,
    pub lr: u32,
    pub ctr: u32,
    pub xer: u32,
    pub cr: ConditionRegister,

    // These are used during instruction execution to track the current
    // and next PC values. In essence, by writing to `next_pc`, instructions
    // can change the flow of execution (e.g. for branches and jumps).
    pub cia: u32, // Current Instruction Address
    pub nia: u32, // Next Instruction Address
}

impl Cpu {
    pub fn new(initial_pc: u32) -> Self {
        Cpu {
            gprs: [0; 32],
            fprs: [0.0; 32],
            pc: initial_pc,
            cia: initial_pc,
            nia: initial_pc.wrapping_add(4),
            lr: 0,
            ctr: 0,
            xer: 0,
            cr: ConditionRegister::new(),
        }
    }

    #[inline(always)]
    pub fn read_gpr(&self, index: u8) -> u32 {
        self.gprs[index as usize]
    }

    #[inline(always)]
    pub fn write_gpr(&mut self, index: u8, value: u32) {
        self.gprs[index as usize] = value;
    }
}
