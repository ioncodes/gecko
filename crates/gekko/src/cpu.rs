pub mod condition;
pub mod interpreter;
pub mod irq;
pub mod msr;
pub mod semantics;
pub mod spr;
pub mod sr;

use crate::cpu::condition::ConditionRegister;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/gekko_lut.rs"));
}

pub const IPL_RESET_VECTOR: u32 = 0xFFF0_0100;

pub struct Cpu {
    pub gprs: [u32; 32],
    pub fprs: [f64; 32], // PS0
    pub ps1s: [f64; 32], // PS1 (paired single slot 1)
    pub pc: u32,
    pub cr: ConditionRegister,
    pub fpscr: u32, // TODO: FP Status and Control Register
    pub spr: spr::Spr,
    pub msr: msr::Msr,
    pub sr: [sr::Sr; 16], // Segment Registers
    // These are used during instruction execution to track the current
    // and next PC values. In essence, by writing to `next_pc`, instructions
    // can change the flow of execution (e.g. for branches and jumps).
    pub cia: u32,                  // Current Instruction Address
    pub nia: u32,                  // Next Instruction Address
    pub reserve_addr: Option<u32>, // lwarx/stwcx. reservation address
}

impl Cpu {
    pub fn new(initial_pc: u32) -> Self {
        Cpu {
            gprs: [0; 32],
            fprs: [0.0; 32],
            ps1s: [1.0; 32],
            pc: initial_pc,
            cia: initial_pc,
            nia: initial_pc.wrapping_add(4),
            cr: ConditionRegister::new(),
            spr: spr::Spr::default(),
            msr: msr::Msr::default(),
            fpscr: 0,
            sr: [sr::Sr::default(); 16],
            reserve_addr: None,
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

    #[inline(always)]
    pub fn read_fpr(&self, index: u8) -> f64 {
        self.fprs[index as usize]
    }

    #[inline(always)]
    pub fn write_fpr(&mut self, index: u8, value: f64) {
        self.fprs[index as usize] = value;
    }

    #[inline(always)]
    pub fn read_ps1(&self, index: u8) -> f64 {
        self.ps1s[index as usize]
    }

    #[inline(always)]
    pub fn write_ps1(&mut self, index: u8, value: f64) {
        self.ps1s[index as usize] = value;
    }

    #[inline]
    pub fn update_cr0(&mut self, val: u32) {
        let so = self.spr.xer.summary_overflow(); // SO is copied from XER[SO]
        self.cr.set_cr0(
            condition::ConditionField::new()
                .with_lt((val as i32) < 0)
                .with_gt((val as i32) > 0)
                .with_eq(val == 0)
                .with_so(so),
        );
    }

    /// Update CR1 with FPSCR[0:3] (used by Rc=1 FP instructions)
    #[inline]
    pub fn update_cr1(&mut self) {
        let cr1 = condition::ConditionField::from((self.fpscr >> 28) as u8);
        self.cr.set_field(1, cr1);
    }

    /// Read GPR with the PowerPC "rA|0" convention: returns 0 when index is 0
    #[inline(always)]
    pub fn read_gpr_or_zero(&self, index: u8) -> u32 {
        if index == 0 { 0 } else { self.gprs[index as usize] }
    }

    /// Get XER carry bit
    #[inline(always)]
    pub fn xer_ca(&self) -> u32 {
        self.spr.xer.carry() as u32
    }

    /// Set XER carry bit
    #[inline(always)]
    pub fn set_xer_ca(&mut self, ca: bool) {
        self.spr.xer = self.spr.xer.with_carry(ca);
    }
}
