pub mod regs;
pub mod stack;

use crate::flipper::dsp::core::regs::{SignExtensionMode, StatusRegister};

/// DSP register indices for `Registers::read` / `Registers::write`.
pub mod reg {
    pub const AR0: u8 = 0;
    pub const AR1: u8 = 1;
    pub const AR2: u8 = 2;
    pub const AR3: u8 = 3;
    pub const IX0: u8 = 4;
    pub const IX1: u8 = 5;
    pub const IX2: u8 = 6;
    pub const IX3: u8 = 7;
    pub const WR0: u8 = 8;
    pub const WR1: u8 = 9;
    pub const WR2: u8 = 10;
    pub const WR3: u8 = 11;
    pub const ST0: u8 = 12;
    pub const ST1: u8 = 13;
    pub const ST2: u8 = 14;
    pub const ST3: u8 = 15;
    pub const AC0H: u8 = 16;
    pub const AC1H: u8 = 17;
    pub const CONFIG: u8 = 18;
    pub const SR: u8 = 19;
    pub const PRODL: u8 = 20;
    pub const PRODM1: u8 = 21;
    pub const PRODH: u8 = 22;
    pub const PRODM2: u8 = 23;
    pub const AX0L: u8 = 24;
    pub const AX1L: u8 = 25;
    pub const AX0H: u8 = 26;
    pub const AX1H: u8 = 27;
    pub const AC0L: u8 = 28;
    pub const AC1L: u8 = 29;
    pub const AC0M: u8 = 30;
    pub const AC1M: u8 = 31;
}

pub struct Registers {
    pub pc: u16,
    pub nia: u16,
    pub cia: u16,
    pub ar: [u16; 4],
    pub ix: [u16; 4],
    pub wr: [u16; 4],
    pub call_stack: stack::DspStack<32>,   // st0
    pub data_stack: stack::DspStack<32>,   // st1
    pub loop_addr: stack::DspStack<32>,    // st2
    pub loop_counter: stack::DspStack<32>, // st3
    pub ac0_high: u16,
    pub ac1_high: u16,
    pub config: u16,
    pub status: StatusRegister,
    pub product_low: u16,
    pub product_mid1: u16,
    pub product_high: u16,
    pub product_mid2: u16,
    pub ax: [u16; 2],
    pub axh: [u16; 2],
    pub ac0_low: u16,
    pub ac1_low: u16,
    pub ac0_mid: u16,
    pub ac1_mid: u16,

    /// Pre-instruction accumulator cache for extension opcodes.
    /// [0]=AC0L, [1]=AC1L, [2]=AC0M(sat), [3]=AC1M(sat), [4]=AC0M(raw), [5]=AC1M(raw)
    pub ext_ac_cache: [u16; 6],
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            pc: 0,
            nia: 0,
            cia: 0,
            ar: [0; 4],
            ix: [0; 4],
            wr: [0xFFFF; 4], // Fuck you seriously
            call_stack: stack::DspStack::default(),
            data_stack: stack::DspStack::default(),
            loop_addr: stack::DspStack::default(),
            loop_counter: stack::DspStack::default(),
            ac0_high: 0,
            ac1_high: 0,
            config: 0,
            status: StatusRegister::default(),
            product_low: 0,
            product_mid1: 0,
            product_high: 0,
            product_mid2: 0,
            ax: [0; 2],
            axh: [0; 2],
            ac0_low: 0,
            ac1_low: 0,
            ac0_mid: 0,
            ac1_mid: 0,
            ext_ac_cache: [0; 6],
        }
    }
}

impl Registers {
    /// Cache the pre-instruction accumulator values that extension opcodes may read.
    /// Must be called before the main instruction dispatch when an extension is present.
    #[inline(always)]
    pub fn cache_ext_ac(&mut self) {
        self.ext_ac_cache[0] = self.ac0_low;
        self.ext_ac_cache[1] = self.ac1_low;
        let sxm = self.sign_extended();
        self.ext_ac_cache[2] = if sxm {
            self.saturate_ac_mid(self.ac0_high, self.ac0_mid)
        } else {
            self.ac0_mid
        };
        self.ext_ac_cache[3] = if sxm {
            self.saturate_ac_mid(self.ac1_high, self.ac1_mid)
        } else {
            self.ac1_mid
        };
        self.ext_ac_cache[4] = self.ac0_mid;
        self.ext_ac_cache[5] = self.ac1_mid;
    }

    /// Increment address register by 1 with WR wrapping.
    #[inline(always)]
    pub fn increment_ar(&self, reg: usize) -> u16 {
        let ar = self.ar[reg] as u32;
        let wr = self.wr[reg] as u32;
        let nar = ar.wrapping_add(1);
        if (nar ^ ar) > ((wr | 1) << 1) {
            nar.wrapping_sub(wr + 1) as u16
        } else {
            nar as u16
        }
    }

    /// Decrement address register by 1 with WR wrapping.
    #[inline(always)]
    pub fn decrement_ar(&self, reg: usize) -> u16 {
        let ar = self.ar[reg] as u32;
        let wr = self.wr[reg] as u32;
        let nar = ar.wrapping_add(wr);
        if ((nar ^ ar) & ((wr | 1) << 1)) > wr {
            nar.wrapping_sub(wr + 1) as u16
        } else {
            nar as u16
        }
    }

    /// Decrease address register by signed IX with WR wrapping (ar -= ix).
    /// As per beanwii, uses the hrydgard/calc84maniac one's-complement algorithm:
    /// subtract = add(!ix) + 1 with WR-modular wrap.
    #[inline(always)]
    pub fn decrease_ar_ix(&self, reg: usize, ix: i16) -> u16 {
        let ar = self.ar[reg];
        let wr = self.wr[reg];
        let value = !ix;
        let n = (16 - wr.leading_zeros()).max(1);
        let mask = 1u16.checked_shl(n).map_or(!0, |r| r - 1);
        let carry = ((ar & mask) as u32 + (value as u16 & mask) as u32 + 1) > mask as u32;
        let mut result = ar.wrapping_add_signed(value).wrapping_add(1);
        let vp1 = value.wrapping_add(1);
        if vp1 > 0 || vp1 == i16::MIN {
            if carry {
                result = result.wrapping_sub(wr.wrapping_add(1));
            }
        } else {
            let low_sum = result & mask;
            let low_not_wrap = (!wr) & mask;
            let carry_again = low_sum < low_not_wrap;
            if !carry || carry_again {
                result = result.wrapping_add(wr.wrapping_add(1));
            }
        }
        result
    }

    /// Increase address register by signed IX with WR wrapping.
    #[inline(always)]
    pub fn increase_ar(&self, reg: usize, ix: i16) -> u16 {
        let ar = self.ar[reg] as u32;
        let wr = self.wr[reg] as u32;
        let mx = (wr | 1) << 1;
        let nar = ar.wrapping_add(ix as i32 as u32);
        let dar = (nar ^ ar ^ ix as i32 as u32) & mx;
        if ix >= 0 {
            if dar > wr {
                nar.wrapping_sub(wr + 1) as u16
            } else {
                nar as u16
            }
        } else if (((nar.wrapping_add(wr + 1)) ^ nar) & dar) <= wr {
            nar.wrapping_add(wr + 1) as u16
        } else {
            nar as u16
        }
    }

    /// Get 40-bit accumulator as i64 (sign-extended from bit 39).
    #[inline(always)]
    pub fn ac(&self, idx: u8) -> i64 {
        let (high, mid, low) = match idx {
            0 => (self.ac0_high, self.ac0_mid, self.ac0_low),
            1 => (self.ac1_high, self.ac1_mid, self.ac1_low),
            _ => unreachable!(),
        };
        let raw = ((high as u64 & 0xFF) << 32) | ((mid as u64) << 16) | (low as u64);
        ((raw as i64) << 24) >> 24
    }

    #[inline(always)]
    pub fn ac_mid(&self, idx: u8) -> u16 {
        match idx {
            0 => self.ac0_mid,
            1 => self.ac1_mid,
            _ => unreachable!(),
        }
    }

    /// Write a 40-bit value (masked to 40 bits) into accumulator `idx` (0 or 1).
    #[inline(always)]
    pub fn set_ac(&mut self, idx: u8, val: i64) {
        let v = val as u64 & 0xFF_FFFF_FFFF;
        let high = (((v >> 32) as u8 as i8) as i16) as u16;
        let mid = (v >> 16) as u16;
        let low = v as u16;
        match idx {
            0 => {
                self.ac0_high = high;
                self.ac0_mid = mid;
                self.ac0_low = low;
            }
            1 => {
                self.ac1_high = high;
                self.ac1_mid = mid;
                self.ac1_low = low;
            }
            _ => unreachable!(),
        }
    }

    /// Update flags for 16-bit logic operations: S, Z from 16-bit result; AS32 from full ac; TB from 16-bit result.
    #[inline(always)]
    pub fn update_flags_logic(&mut self, result16: u16, ac_full: i64) {
        let sign = result16 & 0x8000 != 0;
        let zero = result16 == 0;
        let r40 = ac_full as u64 & 0xFF_FFFF_FFFF;
        let upper9 = (r40 >> 31) & 0x1FF;
        let above_s32 = upper9 != 0 && upper9 != 0x1FF;
        let tb = (result16 >> 14) == 0 || (result16 >> 14) == 3;

        self.status.set_s(sign);
        self.status.set_z(zero);
        self.status.set_as32(above_s32);
        self.status.set_tb(tb);
    }

    /// Update flags based on a 40-bit accumulator result: TB, S32, S, AZ.
    #[inline(always)]
    pub fn update_flags_ac(&mut self, result: i64) {
        let r40 = result as u64 & 0xFF_FFFF_FFFF;
        let sign = (r40 >> 39) & 1 != 0;
        let zero = r40 == 0;
        let upper9 = (r40 >> 31) & 0x1FF;
        let above_s32 = upper9 != 0 && upper9 != 0x1FF;
        let tb = (r40 & 0xC000_0000) == 0 || (r40 & 0xC000_0000) == 0xC000_0000;

        self.status.set_s(sign);
        self.status.set_z(zero);
        self.status.set_as32(above_s32);
        self.status.set_tb(tb);
    }

    /// Update flags based on a 40-bit result of addition.
    /// Sets: OS, TB, S32, S, AZ, O, C.
    #[inline(always)]
    pub fn update_flags_add(&mut self, a: i64, b: i64, result: i64) {
        let r40 = result as u64 & 0xFF_FFFF_FFFF;
        let a40 = a as u64 & 0xFF_FFFF_FFFF;

        let sign = (r40 >> 39) & 1 != 0;
        let zero = r40 == 0;
        let carry = r40 < a40;
        let a_sign = (a as u64 >> 39) & 1;
        let b_sign = (b as u64 >> 39) & 1;
        let r_sign = (r40 >> 39) & 1;
        let overflow = (a_sign == b_sign) && (r_sign != a_sign);
        let upper9 = (r40 >> 31) & 0x1FF;
        let above_s32 = upper9 != 0 && upper9 != 0x1FF;
        let tb = (r40 & 0xC000_0000) == 0 || (r40 & 0xC000_0000) == 0xC000_0000;

        self.status.set_s(sign);
        self.status.set_z(zero);
        self.status.set_c(carry);
        self.status.set_o(overflow);
        if overflow {
            self.status.set_os(true);
        }
        self.status.set_as32(above_s32);
        self.status.set_tb(tb);
    }

    /// Update flags based on a 40-bit result of subtraction.
    /// Sets: OS, TB, S32, S, AZ, O, C.
    #[inline(always)]
    pub fn update_flags_sub(&mut self, a: i64, b: i64, result: i64) {
        let r40 = result as u64 & 0xFF_FFFF_FFFF;
        let a40 = a as u64 & 0xFF_FFFF_FFFF;

        let sign = (r40 >> 39) & 1 != 0;
        let zero = r40 == 0;
        // Carry for subtraction: A >= result (unsigned 40-bit), i.e. no borrow
        let carry = a40 >= r40;
        // Overflow: sign of A != sign of B, and sign of result != sign of A
        let a_sign = (a as u64 >> 39) & 1;
        let b_sign = (b as u64 >> 39) & 1;
        let r_sign = (r40 >> 39) & 1;
        let overflow = (a_sign != b_sign) && (r_sign != a_sign);
        // Above s32: upper 9 bits (39:31) are not all the same
        let upper9 = (r40 >> 31) & 0x1FF;
        let above_s32 = upper9 != 0 && upper9 != 0x1FF;
        // Top two bits equal
        let tb = (r40 & 0xC000_0000) == 0 || (r40 & 0xC000_0000) == 0xC000_0000;

        self.status.set_s(sign);
        self.status.set_z(zero);
        self.status.set_c(carry);
        self.status.set_o(overflow);
        if overflow {
            self.status.set_os(true);
        }
        self.status.set_as32(above_s32);
        self.status.set_tb(tb);
    }

    #[inline(always)]
    pub fn sign_extended(&self) -> bool {
        self.status.sxm() == SignExtensionMode::Bits40
    }

    #[inline(always)]
    pub fn read<const ALLOW_SATURATION: bool>(&mut self, index: u8) -> u16 {
        match index {
            0 => self.ar[0],
            1 => self.ar[1],
            2 => self.ar[2],
            3 => self.ar[3],
            4 => self.ix[0],
            5 => self.ix[1],
            6 => self.ix[2],
            7 => self.ix[3],
            8 => self.wr[0],
            9 => self.wr[1],
            10 => self.wr[2],
            11 => self.wr[3],
            12 => self.call_stack.pop(),
            13 => self.data_stack.pop(),
            14 => self.loop_addr.pop(),
            15 => self.loop_counter.pop(),
            16 => self.ac0_high,
            17 => self.ac1_high,
            18 => self.config,
            19 => {
                let raw: u16 = self.status.into();
                raw & !0x0100 // bit 8 always reads as 0
            }
            20 => self.product_low,
            21 => self.product_mid1,
            22 => self.product_high & 0xFF,
            23 => self.product_mid2,
            24 => self.ax[0],
            25 => self.ax[1],
            26 => self.axh[0],
            27 => self.axh[1],
            28 => self.ac0_low,
            29 => self.ac1_low,
            30 => {
                if ALLOW_SATURATION && self.sign_extended() {
                    return self.saturate_ac_mid(self.ac0_high, self.ac0_mid);
                }
                self.ac0_mid
            }
            31 => {
                if ALLOW_SATURATION && self.sign_extended() {
                    return self.saturate_ac_mid(self.ac1_high, self.ac1_mid);
                }
                self.ac1_mid
            }
            _ => {
                tracing::error!(index, "DSP read: invalid register index");
                0
            }
        }
    }

    /// Saturate $acX.m: if $acX.h is not the sign extension of $acX.m,
    /// return 0x7FFF (positive) or 0x8000 (negative).
    #[inline(always)]
    fn saturate_ac_mid(&self, high: u16, mid: u16) -> u16 {
        let sign_ext = if mid & 0x8000 != 0 { 0xFFFF } else { 0 };
        if high != sign_ext {
            if high & 0x80 != 0 { 0x8000 } else { 0x7FFF }
        } else {
            mid
        }
    }

    /// Get the 40-bit product register value.
    #[inline(always)]
    pub fn product(&self) -> i64 {
        let ph = (self.product_high as u8) as i8 as i64;
        let pm1 = self.product_mid1 as i64;
        let pm2 = self.product_mid2 as i64;
        let pl = self.product_low as i64;
        (ph << 32) + ((pm1 + pm2) << 16) + pl
    }

    /// Write a value to the product register.
    #[inline(always)]
    pub fn write_product(&mut self, val: i64) {
        self.product_high = (((val >> 32) as u8 as i8) as i16) as u16;
        self.product_mid1 = (val >> 16) as u16;
        self.product_low = val as u16;
        self.product_mid2 = 0;
    }

    /// Compute carry and overflow flags from the product register's internal structure.
    #[inline(always)]
    pub fn product_flags(&self) -> (bool, bool) {
        let mid_carry = ((self.product_mid1 as u32 + self.product_mid2 as u32) >> 16) as u16;
        let ph = self.product_high as u8 as u16;
        let carry = ph + mid_carry > 0xFF;
        let overflow = ph == 0x7F && mid_carry != 0;
        (carry, overflow)
    }

    #[inline(always)]
    pub fn write<const ALLOW_SIGN_EXTENSION: bool>(&mut self, index: u8, value: u16) {
        match index {
            0 => self.ar[0] = value,
            1 => self.ar[1] = value,
            2 => self.ar[2] = value,
            3 => self.ar[3] = value,
            4 => self.ix[0] = value,
            5 => self.ix[1] = value,
            6 => self.ix[2] = value,
            7 => self.ix[3] = value,
            8 => self.wr[0] = value,
            9 => self.wr[1] = value,
            10 => self.wr[2] = value,
            11 => self.wr[3] = value,
            12 => self.call_stack.push(value),
            13 => self.data_stack.push(value),
            14 => self.loop_addr.push(value),
            15 => self.loop_counter.push(value),
            // The high parts of the 40-bit accumulators (acX.h) are sign-extended 8-bit registers. Writes to the upper
            // 8 bits are ignored, and the upper 8 bits read the same as the 7th bit. For instance, 0x007F reads back as
            // 0x007F, but 0x0080 reads back as 0xFF80.
            16 => self.ac0_high = ((value as i8) as i16) as u16,
            17 => self.ac1_high = ((value as i8) as i16) as u16,
            18 => self.config = value,
            19 => self.status = StatusRegister::from(value),
            20 => self.product_low = value,
            21 => self.product_mid1 = value,
            22 => self.product_high = value,
            23 => self.product_mid2 = value,
            24 => self.ax[0] = value,
            25 => self.ax[1] = value,
            26 => self.axh[0] = value,
            27 => self.axh[1] = value,
            28 => self.ac0_low = value,
            29 => self.ac1_low = value,
            30 => {
                self.ac0_mid = value;
                if ALLOW_SIGN_EXTENSION && self.sign_extended() {
                    self.ac0_high = if value & 0x8000 != 0 { 0xFFFF } else { 0 };
                    self.ac0_low = 0;
                }
            }
            31 => {
                self.ac1_mid = value;
                if ALLOW_SIGN_EXTENSION && self.sign_extended() {
                    self.ac1_high = if value & 0x8000 != 0 { 0xFFFF } else { 0 };
                    self.ac1_low = 0;
                }
            }
            _ => {
                tracing::error!(index, "DSP write: invalid register index");
            }
        }
    }
}
