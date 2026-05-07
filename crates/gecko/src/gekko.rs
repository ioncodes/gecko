pub mod condition;
pub mod dec;
pub mod fpscr;
#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod instruction;
pub mod interpreter;
pub mod irq;
#[cfg(feature = "jit")]
pub mod jit;
pub mod msr;
pub mod spr;
pub mod sr;

use crate::gekko::condition::ConditionRegister;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/gekko_lut.rs"));
}

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut_wii {
    include!(concat!(env!("OUT_DIR"), "/gekko_lut_wii.rs"));
}

pub const IPL_RESET_VECTOR: u32 = 0xFFF0_0100;

pub struct Gekko {
    pub gprs: [u32; 32],
    pub fprs: [f64; 32], // PS0
    pub ps1s: [f64; 32], // PS1 (paired single slot 1)
    pub pc: u32,
    pub cr: ConditionRegister,
    pub fpscr: fpscr::Fpscr,
    pub dec: dec::Decrementer,
    pub spr: spr::Spr,
    pub msr: msr::Msr,
    pub sr: [sr::Sr; 16], // Segment Registers
    // These are used during instruction execution to track the current
    // and next PC values. In essence, by writing to `next_pc`, instructions
    // can change the flow of execution (e.g. for branches and jumps).
    pub cia: u32, // Current Instruction Address
    pub nia: u32, // Next Instruction Address
    pub reserve_addr: u32,
}

impl Gekko {
    /// Sentinel used by `lwarx` / `stwcx.`
    pub const NO_RESERVATION: u32 = 0xFF69_1337;

    pub fn new(initial_pc: u32) -> Self {
        let mut spr = spr::Spr::default();
        spr.dec = u32::MAX;

        Gekko {
            gprs: [0; 32],
            fprs: [0.0; 32],
            ps1s: [1.0; 32],
            pc: initial_pc,
            cia: initial_pc,
            nia: initial_pc.wrapping_add(4),
            cr: ConditionRegister::new(),
            dec: dec::Decrementer::default(),
            spr,
            msr: msr::Msr::default(),
            fpscr: fpscr::Fpscr::default(),
            sr: [sr::Sr::default(); 16],
            reserve_addr: Self::NO_RESERVATION,
        }
    }

    #[inline(always)]
    pub fn read_gpr(&self, index: u8) -> u32 {
        debug_assert!(index < 32);
        unsafe { *self.gprs.get_unchecked(index as usize) }
    }

    #[inline(always)]
    pub fn write_gpr(&mut self, index: u8, value: u32) {
        debug_assert!(index < 32);
        unsafe { *self.gprs.get_unchecked_mut(index as usize) = value }
    }

    #[inline(always)]
    pub fn read_fpr(&self, index: u8) -> f64 {
        debug_assert!(index < 32);
        unsafe { *self.fprs.get_unchecked(index as usize) }
    }

    #[inline(always)]
    pub fn write_fpr(&mut self, index: u8, value: f64) {
        debug_assert!(index < 32);
        unsafe { *self.fprs.get_unchecked_mut(index as usize) = value }
    }

    #[inline(always)]
    pub fn read_ps1(&self, index: u8) -> f64 {
        debug_assert!(index < 32);
        unsafe { *self.ps1s.get_unchecked(index as usize) }
    }

    #[inline(always)]
    pub fn write_ps1(&mut self, index: u8, value: f64) {
        debug_assert!(index < 32);
        unsafe { *self.ps1s.get_unchecked_mut(index as usize) = value }
    }

    #[inline(always)]
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
    #[inline(always)]
    pub fn update_cr1(&mut self) {
        let cr1 = condition::ConditionField::from((self.fpscr.raw() >> 28) as u8);
        self.cr.set_field(1, cr1);
    }

    /// Recompute FPSCR[VX] and FPSCR[FEX] from the underlying bits.
    ///
    /// VX  = OR of all VXxxx bits.
    /// FEX = (VX & VE) | (OX & OE) | (UX & UE) | (ZX & ZE) | (XX & XE).
    #[inline(always)]
    pub fn recompute_fpscr_summary(&mut self) {
        let vx = self.fpscr.vxsnan()
            || self.fpscr.vxisi()
            || self.fpscr.vxidi()
            || self.fpscr.vxzdz()
            || self.fpscr.vximz()
            || self.fpscr.vxvc()
            || self.fpscr.vxsoft()
            || self.fpscr.vxsqrt()
            || self.fpscr.vxcvi();
        self.fpscr = self.fpscr.with_vx(vx);

        let fex = (self.fpscr.vx() && self.fpscr.ve())
            || (self.fpscr.ox() && self.fpscr.oe())
            || (self.fpscr.ux() && self.fpscr.ue())
            || (self.fpscr.zx() && self.fpscr.ze())
            || (self.fpscr.xx() && self.fpscr.xe());
        self.fpscr = self.fpscr.with_fex(fex);
    }

    /// Read GPR with the PowerPC "rA|0" convention: returns 0 when index is 0
    #[inline(always)]
    pub fn read_gpr_or_zero(&self, index: u8) -> u32 {
        debug_assert!(index < 32);
        if index == 0 {
            0
        } else {
            unsafe { *self.gprs.get_unchecked(index as usize) }
        }
    }
}

#[inline(always)]
pub fn dispatch<const SYSTEM: crate::system::SystemId>(
    ctx: &mut crate::system::System<SYSTEM>,
    instr: instruction::Instruction,
) {
    if SYSTEM == crate::system::GC {
        let ctx: &mut crate::gamecube::GameCube = unsafe { core::mem::transmute(ctx) };
        self::lut::dispatch(ctx, instr);
    } else {
        let ctx: &mut crate::wii::Wii = unsafe { core::mem::transmute(ctx) };
        self::lut_wii::dispatch(ctx, instr);
    }
}
