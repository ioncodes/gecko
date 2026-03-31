use chapa::BitEnum;

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

pub struct DspStack<const N: usize> {
    data: [u16; N],
    ptr: u8,
}

impl<const N: usize> Default for DspStack<N> {
    fn default() -> Self {
        Self { data: [0; N], ptr: 0 }
    }
}

impl<const N: usize> DspStack<N> {
    #[inline(always)]
    pub fn top(&self) -> u16 {
        self.data[self.ptr as usize]
    }

    #[inline(always)]
    pub fn set_top(&mut self, value: u16) {
        self.data[self.ptr as usize] = value;
    }

    #[inline(always)]
    pub fn push(&mut self, value: u16) {
        self.ptr += 1;
        self.data[self.ptr as usize] = value;
    }

    #[inline(always)]
    pub fn pop(&mut self) -> u16 {
        let value = self.data[self.ptr as usize];
        self.ptr -= 1;
        value
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.ptr == 0
    }
}

#[derive(Default)]
pub struct Registers {
    pub pc: u16,
    pub nia: u16,
    pub cia: u16,
    pub ar: [u16; 4],
    pub ix: [u16; 4],
    pub wr: [u16; 4],
    pub call_stack: DspStack<8>,   // st0
    pub data_stack: DspStack<4>,   // st1
    pub loop_addr: DspStack<4>,    // st2
    pub loop_counter: DspStack<4>, // st3
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
}

impl Registers {
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

    /// Write a 40-bit value (masked to 40 bits) into accumulator `idx` (0 or 1).
    #[inline(always)]
    pub fn set_ac(&mut self, idx: u8, val: i64) {
        let v = val as u64 & 0xFF_FFFF_FFFF;
        let high = (v >> 32) as u16;
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

    /// Update flags based on a 40-bit accumulator result: TB, S32, S, AZ.
    #[inline(always)]
    pub fn update_flags_ac(&mut self, result: i64) {
        let r40 = result as u64 & 0xFF_FFFF_FFFF;
        let sign = (r40 >> 39) & 1 != 0;
        let zero = r40 == 0;
        let upper9 = (r40 >> 31) & 0x1FF;
        let above_s32 = upper9 != 0 && upper9 != 0x1FF;
        let tb = ((r40 >> 39) & 1) == ((r40 >> 38) & 1);

        self.status.set_s(sign);
        self.status.set_z(zero);
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
        let tb = ((r40 >> 39) & 1) == ((r40 >> 38) & 1);

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
    pub fn read<const ALLOW_SATURATION: bool>(&self, index: u8) -> u16 {
        match index {
            0..=3 => self.ar[index as usize],
            4..=7 => self.ix[(index - 4) as usize],
            8..=11 => self.wr[(index - 8) as usize],
            12 => self.call_stack.top(),
            13 => self.data_stack.top(),
            14 => self.loop_addr.top(),
            15 => self.loop_counter.top(),
            16 => self.ac0_high,
            17 => self.ac1_high,
            18 => self.config,
            19 => self.status.into(),
            20 => self.product_low,
            21 => self.product_mid1,
            22 => self.product_high,
            23 => self.product_mid2,
            24..=25 => self.ax[(index - 24) as usize],
            26..=27 => self.axh[(index - 26) as usize],
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
            _ => unreachable!(),
        }
    }

    /// Saturate $acX.m: if $acX.h is not the sign extension of $acX.m,
    /// return 0x7FFF (positive) or 0x8000 (negative).
    #[inline(always)]
    fn saturate_ac_mid(&self, high: u16, mid: u16) -> u16 {
        let sign_ext = if mid & 0x8000 != 0 { 0x00FF } else { 0 };
        if high != sign_ext {
            if high & 0x80 != 0 { 0x8000 } else { 0x7FFF }
        } else {
            mid
        }
    }

    #[inline(always)]
    pub fn write<const ALLOW_SIGN_EXTENSION: bool>(&mut self, index: u8, value: u16) {
        match index {
            0..=3 => self.ar[index as usize] = value,
            4..=7 => self.ix[(index - 4) as usize] = value,
            8..=11 => self.wr[(index - 8) as usize] = value,
            12 => self.call_stack.set_top(value),
            13 => self.data_stack.set_top(value),
            14 => self.loop_addr.set_top(value),
            15 => self.loop_counter.set_top(value),
            16 => self.ac0_high = value,
            17 => self.ac1_high = value,
            18 => self.config = value,
            19 => self.status = StatusRegister::from(value),
            20 => self.product_low = value,
            21 => self.product_mid1 = value,
            22 => self.product_high = value,
            23 => self.product_mid2 = value,
            24..=25 => self.ax[(index - 24) as usize] = value,
            26..=27 => self.axh[(index - 26) as usize] = value,
            28 => self.ac0_low = value,
            29 => self.ac1_low = value,
            30 => {
                self.ac0_mid = value;
                if ALLOW_SIGN_EXTENSION && self.sign_extended() {
                    self.ac0_high = if value & 0x8000 != 0 { 0x00FF } else { 0 };
                    self.ac0_low = 0;
                }
            }
            31 => {
                self.ac1_mid = value;
                if ALLOW_SIGN_EXTENSION && self.sign_extended() {
                    self.ac1_high = if value & 0x8000 != 0 { 0x00FF } else { 0 };
                    self.ac1_low = 0;
                }
            }
            _ => unreachable!(),
        }
    }
}

#[derive(BitEnum, PartialEq, PartialOrd)]
pub enum SignExtensionMode {
    Bits16,
    Bits40,
}

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct StatusRegister {
    #[bits(0, alias = "c")]
    pub carry: bool,
    #[bits(1, alias = "o")]
    pub overflow: bool,
    #[bits(2, alias = "z")]
    pub arithmetic_zero: bool,
    #[bits(3, alias = "s")]
    pub sign: bool,
    #[bits(4, alias = "as32")]
    pub above_s32: bool,
    #[bits(5, alias = "tb")]
    pub top_two_bits_equal: bool,
    #[bits(6, alias = "lz")]
    pub logical_zero: bool,
    #[bits(7, alias = "os")]
    pub overflow_sticky: bool,
    #[bits(9, alias = "ie")]
    pub interrupt_enable: bool,
    #[bits(11, alias = "eie")]
    pub external_interrupt_enable: bool,
    #[bits(13, alias = "am")]
    pub product_multiply_result_by_2: bool, // when AM = 0
    #[bits(14, alias = "sxm")]
    pub sign_extension_mode: SignExtensionMode,
    #[bits(15, alias = "su")]
    pub multiplication_operands_are_signed: bool,
}
