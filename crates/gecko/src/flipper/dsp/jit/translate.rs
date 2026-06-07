use cranelift_codegen::ir::{InstBuilder, MemFlagsData, Value, types};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;

use crate::flipper::dsp::instruction::{GcDspExt, Instruction};
use crate::flipper::dsp::lut::*;
use crate::system::SystemId;

use super::abi;
use super::translator::ExternFuncs;

pub struct TranslatorCtx<'a, 'b> {
    pub builder: &'a mut FunctionBuilder<'b>,
    pub module: &'a mut cranelift_jit::JITModule,
    pub extern_funcs: ExternFuncs,
    pub sys_ptr: Value,
    pub pc: u16,
    pub size: u8,
}

impl<'a, 'b> TranslatorCtx<'a, 'b> {
    #[inline]
    fn iconst16(&mut self, v: u16) -> Value {
        self.builder.ins().iconst(types::I16, v as i64)
    }

    #[inline]
    fn load_u16(&mut self, offset: i32) -> Value {
        self.builder
            .ins()
            .load(types::I16, MemFlagsData::trusted(), self.sys_ptr, offset)
    }

    #[inline]
    fn store_u16(&mut self, val: Value, offset: i32) {
        self.builder
            .ins()
            .store(MemFlagsData::trusted(), val, self.sys_ptr, offset);
    }

    fn simple_reg_offset(slot: u8) -> Option<i32> {
        Some(
            (match slot {
                0..=3 => abi::dsp_ar_base_offset() + (slot as usize) * 2,
                4..=7 => abi::dsp_ix_base_offset() + (slot as usize - 4) * 2,
                8..=11 => abi::dsp_wr_base_offset() + (slot as usize - 8) * 2,
                12..=15 => return None,
                16 => abi::dsp_ac0_high_offset(),
                17 => abi::dsp_ac1_high_offset(),
                18 => abi::dsp_config_offset(),
                19 => abi::dsp_status_offset(),
                20 => abi::dsp_product_low_offset(),
                21 => abi::dsp_product_mid1_offset(),
                22 => abi::dsp_product_high_offset(),
                23 => abi::dsp_product_mid2_offset(),
                24 | 25 => abi::dsp_ax_base_offset() + (slot as usize - 24) * 2,
                26 | 27 => abi::dsp_axh_base_offset() + (slot as usize - 26) * 2,
                28 => abi::dsp_ac0_low_offset(),
                29 => abi::dsp_ac1_low_offset(),
                30..=31 => return None,
                _ => return None,
            }) as i32,
        )
    }

    pub fn try_emit_reg_read(&mut self, slot: u8) -> Option<Value> {
        let off = Self::simple_reg_offset(slot)?;
        let raw = self.load_u16(off);
        let v = match slot {
            19 => self.builder.ins().band_imm(raw, !0x0100i64 & 0xFFFF),
            22 => self.builder.ins().band_imm(raw, 0xFF),
            _ => raw,
        };

        Some(v)
    }

    pub fn try_emit_reg_write(&mut self, slot: u8, value: Value) -> bool {
        let Some(off) = Self::simple_reg_offset(slot) else {
            return false;
        };

        let to_store = match slot {
            16 | 17 => {
                let trunc = self.builder.ins().ireduce(types::I8, value);
                self.builder.ins().sextend(types::I16, trunc)
            }
            _ => value,
        };
        self.store_u16(to_store, off);

        true
    }

    fn load_csr(&mut self) -> Value {
        let off = abi::dsp_csr_offset() as i32;
        self.load_u16(off)
    }

    fn store_csr(&mut self, val: Value) {
        let off = abi::dsp_csr_offset() as i32;
        self.store_u16(val, off);
    }

    const SR_BIT_C: u8 = 0;
    const SR_BIT_O: u8 = 1;
    const SR_BIT_Z: u8 = 2;
    const SR_BIT_S: u8 = 3;
    const SR_BIT_AS32: u8 = 4;
    const SR_BIT_TB: u8 = 5;
    const SR_BIT_LZ: u8 = 6;

    fn load_sr(&mut self) -> Value {
        let off = abi::dsp_status_offset() as i32;
        self.load_u16(off)
    }

    fn sr_bit(&mut self, sr: Value, bit: u8) -> Value {
        let shifted = self.builder.ins().ushr_imm(sr, bit as i64);
        self.builder.ins().band_imm(shifted, 1)
    }

    fn ac_offsets(d: u8) -> (i32, i32, i32) {
        if d == 0 {
            (
                abi::dsp_ac0_high_offset() as i32,
                abi::dsp_ac0_mid_offset() as i32,
                abi::dsp_ac0_low_offset() as i32,
            )
        } else {
            (
                abi::dsp_ac1_high_offset() as i32,
                abi::dsp_ac1_mid_offset() as i32,
                abi::dsp_ac1_low_offset() as i32,
            )
        }
    }

    pub fn read_ac_i64(&mut self, d: u8) -> Value {
        let (h_off, m_off, l_off) = Self::ac_offsets(d);
        let high = self.load_u16(h_off);
        let mid = self.load_u16(m_off);
        let low = self.load_u16(l_off);

        let high64 = self.builder.ins().sextend(types::I64, high);
        let high_byte = self.builder.ins().band_imm(high64, 0xFFi64);
        let mid64 = self.builder.ins().uextend(types::I64, mid);
        let low64 = self.builder.ins().uextend(types::I64, low);
        let h_shl = self.builder.ins().ishl_imm(high_byte, 32);
        let m_shl = self.builder.ins().ishl_imm(mid64, 16);
        let hm = self.builder.ins().bor(h_shl, m_shl);
        let merged = self.builder.ins().bor(hm, low64);

        let shl = self.builder.ins().ishl_imm(merged, 24);
        self.builder.ins().sshr_imm(shl, 24)
    }

    pub fn write_ac_i64(&mut self, d: u8, val: Value) {
        let (h_off, m_off, l_off) = Self::ac_offsets(d);
        let mid_shift = self.builder.ins().sshr_imm(val, 16);
        let mid = self.builder.ins().ireduce(types::I16, mid_shift);
        let low = self.builder.ins().ireduce(types::I16, val);
        let high_shift = self.builder.ins().sshr_imm(val, 32);

        let high_byte = self.builder.ins().ireduce(types::I8, high_shift);
        let high = self.builder.ins().sextend(types::I16, high_byte);
        self.store_u16(high, h_off);
        self.store_u16(mid, m_off);
        self.store_u16(low, l_off);
    }

    pub fn read_product_i64(&mut self) -> Value {
        let ph = self.load_u16(abi::dsp_product_high_offset() as i32);
        let pm1 = self.load_u16(abi::dsp_product_mid1_offset() as i32);
        let pm2 = self.load_u16(abi::dsp_product_mid2_offset() as i32);
        let pl = self.load_u16(abi::dsp_product_low_offset() as i32);

        let ph_u8 = self.builder.ins().band_imm(ph, 0xFF);
        let ph_shl = self.builder.ins().ishl_imm(ph_u8, 8);
        let ph_i16 = self.builder.ins().sshr_imm(ph_shl, 8);
        let ph_i64 = self.builder.ins().sextend(types::I64, ph_i16);
        let ph_shifted = self.builder.ins().ishl_imm(ph_i64, 32);

        let pm1_64 = self.builder.ins().uextend(types::I64, pm1);
        let pm2_64 = self.builder.ins().uextend(types::I64, pm2);
        let mid_sum = self.builder.ins().iadd(pm1_64, pm2_64);
        let mid_shifted = self.builder.ins().ishl_imm(mid_sum, 16);

        let pl_64 = self.builder.ins().uextend(types::I64, pl);

        let part1 = self.builder.ins().iadd(ph_shifted, mid_shifted);
        self.builder.ins().iadd(part1, pl_64)
    }

    pub fn write_product_i64(&mut self, val: Value) {
        let high_shift = self.builder.ins().sshr_imm(val, 32);
        let high_i8 = self.builder.ins().ireduce(types::I8, high_shift);
        let high = self.builder.ins().sextend(types::I16, high_i8);
        self.store_u16(high, abi::dsp_product_high_offset() as i32);

        let pm1_shift = self.builder.ins().sshr_imm(val, 16);
        let pm1 = self.builder.ins().ireduce(types::I16, pm1_shift);
        self.store_u16(pm1, abi::dsp_product_mid1_offset() as i32);

        let pl = self.builder.ins().ireduce(types::I16, val);
        self.store_u16(pl, abi::dsp_product_low_offset() as i32);

        let zero = self.iconst16(0);
        self.store_u16(zero, abi::dsp_product_mid2_offset() as i32);
    }

    pub fn multiply_i64(&mut self, a_i16: Value, b_i16: Value) -> Value {
        let a32 = self.builder.ins().sextend(types::I32, a_i16);
        let b32 = self.builder.ins().sextend(types::I32, b_i16);
        let mul32 = self.builder.ins().imul(a32, b32);
        let mul64 = self.builder.ins().sextend(types::I64, mul32);

        let status = self.load_u16(abi::dsp_status_offset() as i32);
        let am_shift = self.builder.ins().ushr_imm(status, 13);
        let am = self.builder.ins().band_imm(am_shift, 1);

        use cranelift_codegen::ir::condcodes::IntCC;

        let am_set = self.builder.ins().icmp_imm(IntCC::NotEqual, am, 0);
        let shifted = self.builder.ins().ishl_imm(mul64, 1);
        self.builder.ins().select(am_set, mul64, shifted)
    }

    pub fn multiply_accumulate_i64(&mut self, a_i16: Value, b_i16: Value, add: bool) {
        let mul = self.multiply_i64(a_i16, b_i16);
        let prod = self.read_product_i64();
        let result = if add {
            self.builder.ins().iadd(prod, mul)
        } else {
            self.builder.ins().isub(prod, mul)
        };

        self.write_product_i64(result);
    }

    pub fn mulx_operands_u16(&mut self, s: u8, t: u8) -> (Value, Value) {
        let a_off = if s != 0 {
            abi::dsp_axh_base_offset()
        } else {
            abi::dsp_ax_base_offset()
        } as i32;

        let b_off = if t != 0 {
            (abi::dsp_axh_base_offset() + 2) as i32
        } else {
            (abi::dsp_ax_base_offset() + 2) as i32
        };

        (self.load_u16(a_off), self.load_u16(b_off))
    }

    pub fn multiply_mulx(&mut self, s: u8, t: u8) {
        use cranelift_codegen::ir::condcodes::IntCC;

        let (a, b) = self.mulx_operands_u16(s, t);
        let a_sext = self.builder.ins().sextend(types::I64, a);
        let b_sext = self.builder.ins().sextend(types::I64, b);
        let a_zext = self.builder.ins().uextend(types::I64, a);
        let b_zext = self.builder.ins().uextend(types::I64, b);

        let signed_mul = self.builder.ins().imul(a_sext, b_sext);
        let unsigned_mul = match (s, t) {
            (0, 0) => self.builder.ins().imul(a_zext, b_zext),
            (0, 1) => self.builder.ins().imul(a_zext, b_sext),
            (1, 0) => self.builder.ins().imul(a_sext, b_zext),
            _ => signed_mul,
        };

        let status = self.load_u16(abi::dsp_status_offset() as i32);
        let su_shift = self.builder.ins().ushr_imm(status, 15);
        let su = self.builder.ins().band_imm(su_shift, 1);
        let su_set = self.builder.ins().icmp_imm(IntCC::NotEqual, su, 0);
        let mul = self.builder.ins().select(su_set, unsigned_mul, signed_mul);

        let am_shift = self.builder.ins().ushr_imm(status, 13);
        let am = self.builder.ins().band_imm(am_shift, 1);
        let am_set = self.builder.ins().icmp_imm(IntCC::NotEqual, am, 0);
        let shifted = self.builder.ins().ishl_imm(mul, 1);
        let result = self.builder.ins().select(am_set, mul, shifted);

        self.write_product_i64(result);
    }

    pub fn mulc_operands_i16(&mut self, s: u8, t: u8) -> (Value, Value) {
        let mid_off = if s != 0 {
            abi::dsp_ac1_mid_offset()
        } else {
            abi::dsp_ac0_mid_offset()
        } as i32;

        let axh_off = (abi::dsp_axh_base_offset() + (t as usize) * 2) as i32;
        (self.load_u16(mid_off), self.load_u16(axh_off))
    }

    pub fn read_status_bit(&mut self, bit: u32) -> Value {
        let status = self.load_u16(abi::dsp_status_offset() as i32);
        let shifted = self.builder.ins().ushr_imm(status, bit as i64);
        self.builder.ins().band_imm(shifted, 1)
    }

    pub fn set_status_bit(&mut self, bit: u32, value: Value) {
        let off = abi::dsp_status_offset() as i32;
        let status = self.load_u16(off);
        let mask = !(1u16 << bit);
        let cleared = self.builder.ins().band_imm(status, mask as i64);
        let v_clean = self.builder.ins().band_imm(value, 1);
        let v_shifted = self.builder.ins().ishl_imm(v_clean, bit as i64);
        let new_status = self.builder.ins().bor(cleared, v_shifted);
        self.store_u16(new_status, off);
    }

    pub fn set_status_bit_const(&mut self, bit: u32, value: bool) {
        let off = abi::dsp_status_offset() as i32;
        let status = self.load_u16(off);
        let new_status = if value {
            self.builder.ins().bor_imm(status, 1i64 << bit)
        } else {
            self.builder.ins().band_imm(status, !(1i64 << bit))
        };
        self.store_u16(new_status, off);
    }

    pub fn or_status_bit(&mut self, bit: u32, value: Value) {
        let off = abi::dsp_status_offset() as i32;
        let status = self.load_u16(off);
        let v_clean = self.builder.ins().band_imm(value, 1);
        let v_shifted = self.builder.ins().ishl_imm(v_clean, bit as i64);
        let new_status = self.builder.ins().bor(status, v_shifted);
        self.store_u16(new_status, off);
    }

    pub fn product_flags_i16(&mut self) -> (Value, Value) {
        use cranelift_codegen::ir::condcodes::IntCC;

        let pm1 = self.load_u16(abi::dsp_product_mid1_offset() as i32);
        let pm2 = self.load_u16(abi::dsp_product_mid2_offset() as i32);
        let ph = self.load_u16(abi::dsp_product_high_offset() as i32);

        let pm1_32 = self.builder.ins().uextend(types::I32, pm1);
        let pm2_32 = self.builder.ins().uextend(types::I32, pm2);
        let sum_32 = self.builder.ins().iadd(pm1_32, pm2_32);
        let mid_carry_32 = self.builder.ins().ushr_imm(sum_32, 16);
        let mid_carry = self.builder.ins().ireduce(types::I16, mid_carry_32);

        let ph_u8 = self.builder.ins().band_imm(ph, 0xFF);

        let ph_32 = self.builder.ins().uextend(types::I32, ph_u8);
        let mc_32 = self.builder.ins().uextend(types::I32, mid_carry);
        let total_32 = self.builder.ins().iadd(ph_32, mc_32);
        let carry_32 = self.builder.ins().icmp_imm(IntCC::UnsignedGreaterThan, total_32, 0xFF);
        let carry = self.builder.ins().uextend(types::I16, carry_32);

        let ph_eq_7f = self.builder.ins().icmp_imm(IntCC::Equal, ph_u8, 0x7F);
        let mc_nonzero = self.builder.ins().icmp_imm(IntCC::NotEqual, mid_carry, 0);
        let overflow_b8 = self.builder.ins().band(ph_eq_7f, mc_nonzero);
        let overflow = self.builder.ins().uextend(types::I16, overflow_b8);

        (carry, overflow)
    }

    pub fn apply_product_oc(&mut self, carry: Value, overflow: Value) {
        self.set_status_bit(1, overflow);
        self.or_status_bit(7, overflow);
        self.set_status_bit(0, carry);
    }

    pub fn move_prod_to_ac(&mut self, r: u8) {
        let (carry, overflow) = self.product_flags_i16();
        let prod = self.read_product_i64();
        self.write_ac_i64(r, prod);
        self.emit_update_flags_ac(prod);
        self.apply_product_oc(carry, overflow);
    }

    pub fn round_half_to_even(&mut self, val: Value) -> (Value, Value) {
        use cranelift_codegen::ir::condcodes::IntCC;

        let val40 = self.builder.ins().band_imm(val, 0xFF_FFFF_FFFF_i64);
        let lower = self.builder.ins().band_imm(val40, 0xFFFF);
        let bit16 = self.builder.ins().band_imm(val40, 0x10000);
        let bit16_set = self.builder.ins().icmp_imm(IntCC::NotEqual, bit16, 0);

        let lower_gt = self.builder.ins().icmp_imm(IntCC::UnsignedGreaterThan, lower, 0x8000);
        let lower_eq = self.builder.ins().icmp_imm(IntCC::Equal, lower, 0x8000);
        let lower_eq_and_bit16 = self.builder.ins().band(lower_eq, bit16_set);
        let round_up = self.builder.ins().bor(lower_gt, lower_eq_and_bit16);

        let truncated = self.builder.ins().band_imm(val40, !0xFFFF_i64);
        let sum = self.builder.ins().iadd_imm(val40, 0x10000);
        let rounded = self.builder.ins().band_imm(sum, !0xFFFF_i64);
        let sum_overflow = self
            .builder
            .ins()
            .icmp_imm(IntCC::UnsignedGreaterThan, sum, 0xFF_FFFF_FFFF_i64);
        let sum_overflow_i16 = self.builder.ins().uextend(types::I16, sum_overflow);
        let zero_i16 = self.iconst16(0);

        let result = self.builder.ins().select(round_up, rounded, truncated);
        let carry_out = self.builder.ins().select(round_up, sum_overflow_i16, zero_i16);

        (result, carry_out)
    }

    pub fn move_prod_to_ac_zero(&mut self, r: u8) {
        let (carry, overflow) = self.product_flags_i16();
        let raw = self.read_product_i64();
        let (prod, rounding_carry) = self.round_half_to_even(raw);
        self.write_ac_i64(r, prod);
        self.emit_update_flags_ac(prod);

        let combined_carry = self.builder.ins().bor(carry, rounding_carry);
        self.apply_product_oc(combined_carry, overflow);
    }

    pub fn add_prod_to_ac(&mut self, r: u8) {
        let a = self.read_ac_i64(r);
        let (pc, po) = self.product_flags_i16();
        let b = self.read_product_i64();
        let result = self.builder.ins().iadd(a, b);
        let os_before = self.read_status_bit(7);
        self.write_ac_i64(r, result);
        self.emit_update_flags_add(a, b, result);
        self.set_status_bit(7, os_before);

        let c_now = self.read_status_bit(0);
        let o_now = self.read_status_bit(1);
        let c_xor = self.builder.ins().bxor(c_now, pc);
        let o_xor = self.builder.ins().bxor(o_now, po);
        self.set_status_bit(0, c_xor);
        self.set_status_bit(1, o_xor);
        self.or_status_bit(7, o_xor);
    }

    pub fn read_ac_mid(&mut self, d: u8) -> Value {
        let (_, m_off, _) = Self::ac_offsets(d);
        self.load_u16(m_off)
    }

    pub fn write_ac_mid(&mut self, d: u8, val: Value) {
        let (_, m_off, _) = Self::ac_offsets(d);
        self.store_u16(val, m_off);
    }

    fn store_status_bits(&mut self, bits: Value, clear_mask: i64) {
        let off = abi::dsp_status_offset() as i32;
        let sr = self.load_u16(off);
        let cleared = self.builder.ins().band_imm(sr, clear_mask);
        let new_sr = self.builder.ins().bor(cleared, bits);
        self.store_u16(new_sr, off);
    }

    fn flags40_as32_pos(&mut self, r40: Value) -> Value {
        use cranelift_codegen::ir::condcodes::IntCC;

        let upper9_raw = self.builder.ins().ushr_imm(r40, 31);
        let upper9 = self.builder.ins().band_imm(upper9_raw, 0x1FF);
        let ne0 = self.builder.ins().icmp_imm(IntCC::NotEqual, upper9, 0);
        let ne1ff = self.builder.ins().icmp_imm(IntCC::NotEqual, upper9, 0x1FF);
        let as32 = self.builder.ins().band(ne0, ne1ff);
        let as32_16 = self.builder.ins().uextend(types::I16, as32);
        self.builder.ins().ishl_imm(as32_16, 4)
    }

    fn flags40_szat(&mut self, r40: Value) -> Value {
        use cranelift_codegen::ir::condcodes::IntCC;

        let sign64 = self.builder.ins().ushr_imm(r40, 39);
        let sign16 = self.builder.ins().ireduce(types::I16, sign64);
        let s_pos = self.builder.ins().ishl_imm(sign16, 3);

        let z = self.builder.ins().icmp_imm(IntCC::Equal, r40, 0);
        let z16 = self.builder.ins().uextend(types::I16, z);
        let z_pos = self.builder.ins().ishl_imm(z16, 2);

        let top2 = self.builder.ins().band_imm(r40, 0xC000_0000_i64);
        let tb0 = self.builder.ins().icmp_imm(IntCC::Equal, top2, 0);
        let tb3 = self.builder.ins().icmp_imm(IntCC::Equal, top2, 0xC000_0000_i64);
        let tb = self.builder.ins().bor(tb0, tb3);
        let tb16 = self.builder.ins().uextend(types::I16, tb);
        let tb_pos = self.builder.ins().ishl_imm(tb16, 5);

        let as32_pos = self.flags40_as32_pos(r40);

        let sz = self.builder.ins().bor(s_pos, z_pos);
        let at = self.builder.ins().bor(as32_pos, tb_pos);
        self.builder.ins().bor(sz, at)
    }

    fn flags40_carry_overflow(&mut self, a: Value, b: Value, a40: Value, r40: Value, sub: bool) -> Value {
        use cranelift_codegen::ir::condcodes::IntCC;

        let carry = if sub {
            self.builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, a40, r40)
        } else {
            self.builder.ins().icmp(IntCC::UnsignedLessThan, r40, a40)
        };
        let c_pos = self.builder.ins().uextend(types::I16, carry);

        let a_shift = self.builder.ins().ushr_imm(a, 39);
        let a_sign = self.builder.ins().band_imm(a_shift, 1);
        let b_shift = self.builder.ins().ushr_imm(b, 39);
        let b_sign = self.builder.ins().band_imm(b_shift, 1);
        let r_sign = self.builder.ins().ushr_imm(r40, 39);

        let ab = if sub {
            self.builder.ins().icmp(IntCC::NotEqual, a_sign, b_sign)
        } else {
            self.builder.ins().icmp(IntCC::Equal, a_sign, b_sign)
        };
        let ra = self.builder.ins().icmp(IntCC::NotEqual, r_sign, a_sign);
        let overflow = self.builder.ins().band(ab, ra);
        let o16 = self.builder.ins().uextend(types::I16, overflow);
        let o_pos = self.builder.ins().ishl_imm(o16, 1);
        let os_pos = self.builder.ins().ishl_imm(o16, 7);

        let co = self.builder.ins().bor(c_pos, o_pos);
        self.builder.ins().bor(co, os_pos)
    }

    pub fn emit_update_flags_logic(&mut self, result16: Value, ac_full: Value) {
        use cranelift_codegen::ir::condcodes::IntCC;

        let sign = self.builder.ins().ushr_imm(result16, 15);
        let s_pos = self.builder.ins().ishl_imm(sign, 3);

        let z = self.builder.ins().icmp_imm(IntCC::Equal, result16, 0);
        let z16 = self.builder.ins().uextend(types::I16, z);
        let z_pos = self.builder.ins().ishl_imm(z16, 2);

        let top2 = self.builder.ins().ushr_imm(result16, 14);
        let tb0 = self.builder.ins().icmp_imm(IntCC::Equal, top2, 0);
        let tb3 = self.builder.ins().icmp_imm(IntCC::Equal, top2, 3);
        let tb = self.builder.ins().bor(tb0, tb3);
        let tb16 = self.builder.ins().uextend(types::I16, tb);
        let tb_pos = self.builder.ins().ishl_imm(tb16, 5);

        let r40 = self.builder.ins().band_imm(ac_full, 0xFF_FFFF_FFFF_i64);
        let as32_pos = self.flags40_as32_pos(r40);

        let sz = self.builder.ins().bor(s_pos, z_pos);
        let at = self.builder.ins().bor(as32_pos, tb_pos);
        let bits = self.builder.ins().bor(sz, at);
        self.store_status_bits(bits, !0x003C_i64);
    }

    pub fn emit_update_flags_add(&mut self, a: Value, b: Value, result: Value) {
        let r40 = self.builder.ins().band_imm(result, 0xFF_FFFF_FFFF_i64);
        let a40 = self.builder.ins().band_imm(a, 0xFF_FFFF_FFFF_i64);

        let szat = self.flags40_szat(r40);
        let co = self.flags40_carry_overflow(a, b, a40, r40, false);
        let bits = self.builder.ins().bor(szat, co);
        self.store_status_bits(bits, !0x003F_i64);
    }

    pub fn emit_update_flags_sub(&mut self, a: Value, b: Value, result: Value) {
        let r40 = self.builder.ins().band_imm(result, 0xFF_FFFF_FFFF_i64);
        let a40 = self.builder.ins().band_imm(a, 0xFF_FFFF_FFFF_i64);

        let szat = self.flags40_szat(r40);
        let co = self.flags40_carry_overflow(a, b, a40, r40, true);
        let bits = self.builder.ins().bor(szat, co);
        self.store_status_bits(bits, !0x003F_i64);
    }

    pub fn emit_update_flags_ac(&mut self, ac_full: Value) {
        let r40 = self.builder.ins().band_imm(ac_full, 0xFF_FFFF_FFFF_i64);
        let bits = self.flags40_szat(r40);
        self.store_status_bits(bits, !0x003C_i64);
    }

    pub fn emit_read_dmem(&mut self, addr_u32: Value) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.read_dmem, self.builder.func);
        let inst = self.builder.ins().call(f, &[self.sys_ptr, addr_u32]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_write_dmem(&mut self, addr_u32: Value, value_u32: Value) {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.write_dmem, self.builder.func);
        self.builder.ins().call(f, &[self.sys_ptr, addr_u32, value_u32]);
    }

    pub fn emit_inc_ar(&mut self, reg: u32) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.inc_ar, self.builder.func);
        let reg_v = self.builder.ins().iconst(types::I32, reg as i64);
        let inst = self.builder.ins().call(f, &[self.sys_ptr, reg_v]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_dec_ar(&mut self, reg: u32) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.dec_ar, self.builder.func);
        let reg_v = self.builder.ins().iconst(types::I32, reg as i64);
        let inst = self.builder.ins().call(f, &[self.sys_ptr, reg_v]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_increase_ar(&mut self, reg: u32, ix_value: Value) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.increase_ar, self.builder.func);
        let reg_v = self.builder.ins().iconst(types::I32, reg as i64);
        let ix_u32 = self.builder.ins().uextend(types::I32, ix_value);
        let inst = self.builder.ins().call(f, &[self.sys_ptr, reg_v, ix_u32]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_decrease_ar_ix(&mut self, reg: u32, ix_value: Value) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.decrease_ar_ix, self.builder.func);
        let reg_v = self.builder.ins().iconst(types::I32, reg as i64);
        let ix_u32 = self.builder.ins().uextend(types::I32, ix_value);
        let inst = self.builder.ins().call(f, &[self.sys_ptr, reg_v, ix_u32]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_dynamic_shift(&mut self, d_const: u32, shift_val: Value, mode_const: u32) {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.dynamic_shift, self.builder.func);
        let d_v = self.builder.ins().iconst(types::I32, d_const as i64);
        let shift_u32 = self.builder.ins().uextend(types::I32, shift_val);
        let mode_v = self.builder.ins().iconst(types::I32, mode_const as i64);
        self.builder.ins().call(f, &[self.sys_ptr, d_v, shift_u32, mode_v]);
    }

    pub fn emit_read_imem(&mut self, addr_u32: Value) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.read_imem, self.builder.func);
        let inst = self.builder.ins().call(f, &[self.sys_ptr, addr_u32]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_write_ac_mid_sxm(&mut self, idx: u32, value: Value) {
        use cranelift_codegen::ir::condcodes::IntCC;
        let (mid_off, high_off, low_off) = if idx == 0 {
            (
                abi::dsp_ac0_mid_offset() as i32,
                abi::dsp_ac0_high_offset() as i32,
                abi::dsp_ac0_low_offset() as i32,
            )
        } else {
            (
                abi::dsp_ac1_mid_offset() as i32,
                abi::dsp_ac1_high_offset() as i32,
                abi::dsp_ac1_low_offset() as i32,
            )
        };

        self.store_u16(value, mid_off);

        let status_off = abi::dsp_status_offset() as i32;
        let status = self.load_u16(status_off);
        let sxm_shifted = self.builder.ins().ushr_imm(status, 14);
        let sxm = self.builder.ins().band_imm(sxm_shifted, 1);
        let sxm_set = self.builder.ins().icmp_imm(IntCC::NotEqual, sxm, 0);

        let sxm_block = self.builder.create_block();
        let join_block = self.builder.create_block();
        self.builder.ins().brif(sxm_set, sxm_block, &[], join_block, &[]);

        self.builder.switch_to_block(sxm_block);
        self.builder.seal_block(sxm_block);

        let sign_ext_high = self.builder.ins().sshr_imm(value, 15);
        self.store_u16(sign_ext_high, high_off);
        let zero = self.iconst16(0);
        self.store_u16(zero, low_off);
        self.builder.ins().jump(join_block, &[]);

        self.builder.switch_to_block(join_block);
        self.builder.seal_block(join_block);
    }

    pub fn emit_call_stack_push(&mut self, value: Value) {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.call_stack_push, self.builder.func);
        let v_u32 = self.builder.ins().uextend(types::I32, value);
        self.builder.ins().call(f, &[self.sys_ptr, v_u32]);
    }

    pub fn emit_call_stack_pop(&mut self) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.call_stack_pop, self.builder.func);
        let inst = self.builder.ins().call(f, &[self.sys_ptr]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_data_stack_pop(&mut self) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.data_stack_pop, self.builder.func);
        let inst = self.builder.ins().call(f, &[self.sys_ptr]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_read_reg_full(&mut self, slot: u32) -> Value {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.read_reg_full, self.builder.func);
        let slot_v = self.builder.ins().iconst(types::I32, slot as i64);
        let inst = self.builder.ins().call(f, &[self.sys_ptr, slot_v]);
        self.builder.inst_results(inst)[0]
    }

    pub fn emit_write_reg_full(&mut self, slot: u32, value: Value) {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.write_reg_full, self.builder.func);
        let slot_v = self.builder.ins().iconst(types::I32, slot as i64);
        let value_u32 = self.builder.ins().uextend(types::I32, value);
        self.builder.ins().call(f, &[self.sys_ptr, slot_v, value_u32]);
    }

    pub fn emit_read_reg(&mut self, slot: u8) -> Value {
        if let Some(v) = self.try_emit_reg_read(slot) {
            return v;
        }

        let v32 = self.emit_read_reg_full(slot as u32);
        self.builder.ins().ireduce(types::I16, v32)
    }

    pub fn emit_write_reg(&mut self, slot: u8, value: Value) {
        if !self.try_emit_reg_write(slot, value) {
            self.emit_write_reg_full(slot as u32, value);
        }
    }

    pub fn emit_loop_setup(&mut self, end_addr_p1: u16, counter: Value, call_stack_val: u16) {
        let f = self
            .module
            .declare_func_in_func(self.extern_funcs.loop_setup, self.builder.func);
        let end_v = self.builder.ins().iconst(types::I32, end_addr_p1 as i64);
        let counter_u32 = self.builder.ins().uextend(types::I32, counter);
        let csv_v = self.builder.ins().iconst(types::I32, call_stack_val as i64);
        self.builder.ins().call(f, &[self.sys_ptr, end_v, counter_u32, csv_v]);
    }

    pub fn emit_clear_oc(&mut self) {
        let sr_off = abi::dsp_status_offset() as i32;
        let sr = self.load_u16(sr_off);
        let mask = !((1i64 << Self::SR_BIT_O) | (1i64 << Self::SR_BIT_C)) & 0xFFFF;
        let cleared = self.builder.ins().band_imm(sr, mask);
        self.store_u16(cleared, sr_off);
    }

    pub fn emit_set_lz(&mut self, lz: Value) {
        let sr_off = abi::dsp_status_offset() as i32;
        let sr = self.load_u16(sr_off);
        let bit_pos = Self::SR_BIT_LZ;
        let mask = !(1i64 << bit_pos) & 0xFFFF;
        let cleared = self.builder.ins().band_imm(sr, mask);
        let lz1 = self.builder.ins().band_imm(lz, 1);
        let shifted = self.builder.ins().ishl_imm(lz1, bit_pos as i64);
        let updated = self.builder.ins().bor(cleared, shifted);
        self.store_u16(updated, sr_off);
    }

    fn emit_eval_condition(&mut self, cond: u8) -> Value {
        let sr = self.load_sr();
        let c = self.sr_bit(sr, Self::SR_BIT_C);
        let o = self.sr_bit(sr, Self::SR_BIT_O);
        let z = self.sr_bit(sr, Self::SR_BIT_Z);
        let s = self.sr_bit(sr, Self::SR_BIT_S);
        let as32 = self.sr_bit(sr, Self::SR_BIT_AS32);
        let tb = self.sr_bit(sr, Self::SR_BIT_TB);
        let lz = self.sr_bit(sr, Self::SR_BIT_LZ);

        let and = |b: &mut FunctionBuilder, x: Value, y: Value| b.ins().band(x, y);
        let or = |b: &mut FunctionBuilder, x: Value, y: Value| b.ins().bor(x, y);
        let not = |b: &mut FunctionBuilder, x: Value| b.ins().bxor_imm(x, 1);
        let eqb = |b: &mut FunctionBuilder, x: Value, y: Value| {
            let x = b.ins().bxor(x, y);
            b.ins().bxor_imm(x, 1)
        };
        let neb = |b: &mut FunctionBuilder, x: Value, y: Value| b.ins().bxor(x, y);

        match cond {
            0b0000 => eqb(self.builder, o, s),
            0b0001 => neb(self.builder, o, s),
            0b0010 => {
                let ge = eqb(self.builder, o, s);
                let nz = not(self.builder, z);
                and(self.builder, ge, nz)
            }
            0b0011 => {
                let lt = neb(self.builder, o, s);
                or(self.builder, lt, z)
            }
            0b0100 => not(self.builder, z),
            0b0101 => z,
            0b0110 => not(self.builder, c),
            0b0111 => c,
            0b1000 => not(self.builder, as32),
            0b1001 => as32,
            0b1010 => {
                let either = or(self.builder, as32, tb);
                let nz = not(self.builder, z);
                and(self.builder, either, nz)
            }
            0b1011 => {
                let nas = not(self.builder, as32);
                let ntb = not(self.builder, tb);
                let neither = and(self.builder, nas, ntb);
                or(self.builder, neither, z)
            }
            0b1100 => not(self.builder, lz),
            0b1101 => lz,
            0b1110 => o,
            _ => self.builder.ins().iconst(types::I16, 1),
        }
    }
}

#[cold]
#[inline(never)]
pub fn invalid(_ctx: &mut TranslatorCtx, instr: Instruction) {
    panic!("DSP JIT translate: opcode {:#06x} hit invalid handler", instr.0);
}

#[cold]
#[inline(never)]
pub fn invalid_ext(_ctx: &mut TranslatorCtx, instr: GcDspExt) {
    panic!("DSP JIT translate: ext opcode {:#04x} hit invalid_ext handler", instr.0);
}

#[inline(always)]
pub fn ext_mv<const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: GcDspExt) {
    let d = instr.d_4_5();
    let s = instr.s_6_7();
    let cache_off = (abi::dsp_ext_ac_cache_base_offset() + (s as usize) * 2) as i32;
    let value = ctx.load_u16(cache_off);
    let dst = 24u8 + d;
    ctx.try_emit_reg_write(dst, value);
}

#[inline(always)]
pub fn ext_nop<const SYSTEM: SystemId>(_ctx: &mut TranslatorCtx, _instr: GcDspExt) {}

#[inline(always)]
pub fn control<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_NOP => {}
        OP_HALT => {
            let csr = ctx.load_csr();
            let updated = ctx.builder.ins().bor_imm(csr, 1i64 << 2);
            ctx.store_csr(updated);
        }
        OP_IFCC => {
            let cond_v = ctx.emit_eval_condition(instr.cond());
            let skip_block = ctx.builder.create_block();
            let end_block = ctx.builder.create_block();
            ctx.builder.ins().brif(cond_v, end_block, &[], skip_block, &[]);

            ctx.builder.switch_to_block(skip_block);
            ctx.builder.seal_block(skip_block);
            let nia_off = abi::dsp_nia_offset_max() as i32;
            let nia = ctx.load_u16(nia_off);
            let new_nia = ctx.builder.ins().iadd_imm(nia, 1);
            ctx.store_u16(new_nia, nia_off);
            ctx.builder.ins().jump(end_block, &[]);

            ctx.builder.switch_to_block(end_block);
            ctx.builder.seal_block(end_block);
        }
        OP_JCC => {
            let cond_v = ctx.emit_eval_condition(instr.cond());
            let take_block = ctx.builder.create_block();
            let end_block = ctx.builder.create_block();
            ctx.builder.ins().brif(cond_v, take_block, &[], end_block, &[]);

            ctx.builder.switch_to_block(take_block);
            ctx.builder.seal_block(take_block);
            let target = ctx.iconst16(instr.addr());
            let nia_off = abi::dsp_nia_offset_max() as i32;
            ctx.store_u16(target, nia_off);
            ctx.builder.ins().jump(end_block, &[]);

            ctx.builder.switch_to_block(end_block);
            ctx.builder.seal_block(end_block);
        }
        OP_CALLCC => {
            let cond_v = ctx.emit_eval_condition(instr.cond());
            let take_block = ctx.builder.create_block();
            let end_block = ctx.builder.create_block();
            ctx.builder.ins().brif(cond_v, take_block, &[], end_block, &[]);

            ctx.builder.switch_to_block(take_block);
            ctx.builder.seal_block(take_block);

            let nia = ctx.load_u16(abi::dsp_nia_offset_max() as i32);
            ctx.emit_call_stack_push(nia);

            let target = ctx.iconst16(instr.addr());
            ctx.store_u16(target, abi::dsp_nia_offset_max() as i32);
            ctx.builder.ins().jump(end_block, &[]);

            ctx.builder.switch_to_block(end_block);
            ctx.builder.seal_block(end_block);
        }

        OP_RETCC => {
            let cond_v = ctx.emit_eval_condition(instr.cond());
            let take_block = ctx.builder.create_block();
            let end_block = ctx.builder.create_block();
            ctx.builder.ins().brif(cond_v, take_block, &[], end_block, &[]);

            ctx.builder.switch_to_block(take_block);
            ctx.builder.seal_block(take_block);
            let popped = ctx.emit_call_stack_pop();
            let popped16 = ctx.builder.ins().ireduce(types::I16, popped);
            ctx.store_u16(popped16, abi::dsp_nia_offset_max() as i32);
            ctx.builder.ins().jump(end_block, &[]);

            ctx.builder.switch_to_block(end_block);
            ctx.builder.seal_block(end_block);
        }
        OP_RTICC => {
            let cond_v = ctx.emit_eval_condition(instr.cond());
            let take_block = ctx.builder.create_block();
            let end_block = ctx.builder.create_block();
            ctx.builder.ins().brif(cond_v, take_block, &[], end_block, &[]);

            ctx.builder.switch_to_block(take_block);
            ctx.builder.seal_block(take_block);
            let new_status = ctx.emit_data_stack_pop();
            let new_status16 = ctx.builder.ins().ireduce(types::I16, new_status);
            ctx.store_u16(new_status16, abi::dsp_status_offset() as i32);

            let popped = ctx.emit_call_stack_pop();
            let popped16 = ctx.builder.ins().ireduce(types::I16, popped);
            ctx.store_u16(popped16, abi::dsp_nia_offset_max() as i32);
            ctx.builder.ins().jump(end_block, &[]);

            ctx.builder.switch_to_block(end_block);
            ctx.builder.seal_block(end_block);
        }
        OP_JRCC => {
            let cond_v = ctx.emit_eval_condition(instr.cond());
            let take_block = ctx.builder.create_block();
            let end_block = ctx.builder.create_block();
            ctx.builder.ins().brif(cond_v, take_block, &[], end_block, &[]);

            ctx.builder.switch_to_block(take_block);
            ctx.builder.seal_block(take_block);
            let slot = instr.reg_8_10();
            let val = if let Some(v) = ctx.try_emit_reg_read(slot) {
                v
            } else {
                let v32 = ctx.emit_read_reg_full(slot as u32);
                ctx.builder.ins().ireduce(types::I16, v32)
            };
            ctx.store_u16(val, abi::dsp_nia_offset_max() as i32);
            ctx.builder.ins().jump(end_block, &[]);

            ctx.builder.switch_to_block(end_block);
            ctx.builder.seal_block(end_block);
        }
        OP_CALLRCC => {
            let cond_v = ctx.emit_eval_condition(instr.cond());
            let take_block = ctx.builder.create_block();
            let end_block = ctx.builder.create_block();
            ctx.builder.ins().brif(cond_v, take_block, &[], end_block, &[]);

            ctx.builder.switch_to_block(take_block);
            ctx.builder.seal_block(take_block);

            let slot = instr.reg_8_10();
            let target = if let Some(v) = ctx.try_emit_reg_read(slot) {
                v
            } else {
                let v32 = ctx.emit_read_reg_full(slot as u32);
                ctx.builder.ins().ireduce(types::I16, v32)
            };
            let nia = ctx.load_u16(abi::dsp_nia_offset_max() as i32);
            ctx.emit_call_stack_push(nia);
            ctx.store_u16(target, abi::dsp_nia_offset_max() as i32);
            ctx.builder.ins().jump(end_block, &[]);

            ctx.builder.switch_to_block(end_block);
            ctx.builder.seal_block(end_block);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn loops<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    let natural_nia = ctx.pc.wrapping_add(ctx.size as u16);
    let (end_addr_p1, call_stack_val) = match OP {
        OP_LOOP_REG | OP_LOOPI => (natural_nia.wrapping_add(1), natural_nia),
        OP_BLOOP | OP_BLOOPI => (instr.addr().wrapping_add(1), natural_nia),
        _ => unreachable!("DSP JIT: unexpected loop OP"),
    };

    let counter = match OP {
        OP_LOOPI | OP_BLOOPI => ctx.iconst16(instr.imm_8_15_u8() as u16),
        OP_LOOP_REG | OP_BLOOP => {
            let reg = instr.reg_11_15();
            match ctx.try_emit_reg_read(reg) {
                Some(v) => v,
                None => {
                    let v32 = ctx.emit_read_reg_full(reg as u32);
                    ctx.builder.ins().ireduce(types::I16, v32)
                }
            }
        }
        _ => unreachable!(),
    };

    ctx.emit_loop_setup(end_addr_p1, counter, call_stack_val);
}

#[inline(always)]
pub fn addr_reg<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_DAR => {
            let d = instr.d_14_15();
            let new_ar = ctx.emit_dec_ar(d as u32);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            let ar_off = (abi::dsp_ar_base_offset() + (d as usize) * 2) as i32;
            ctx.store_u16(new_ar16, ar_off);
        }
        OP_IAR => {
            let d = instr.d_14_15();
            let new_ar = ctx.emit_inc_ar(d as u32);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            let ar_off = (abi::dsp_ar_base_offset() + (d as usize) * 2) as i32;
            ctx.store_u16(new_ar16, ar_off);
        }
        OP_SUBARN => {
            let d = instr.d_14_15();
            let ix_off = (abi::dsp_ix_base_offset() + (d as usize) * 2) as i32;
            let ix = ctx.load_u16(ix_off);
            let new_ar = ctx.emit_decrease_ar_ix(d as u32, ix);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            let ar_off = (abi::dsp_ar_base_offset() + (d as usize) * 2) as i32;
            ctx.store_u16(new_ar16, ar_off);
        }
        OP_ADDARN => {
            let s = instr.s_12_13();
            let d = instr.d_14_15();
            let ix_off = (abi::dsp_ix_base_offset() + (s as usize) * 2) as i32;
            let ix = ctx.load_u16(ix_off);
            let new_ar = ctx.emit_increase_ar(d as u32, ix);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            let ar_off = (abi::dsp_ar_base_offset() + (d as usize) * 2) as i32;
            ctx.store_u16(new_ar16, ar_off);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn load_store<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_LRI => {
            let dst = instr.d_11_15();
            let imm = instr.imm_16_31();
            let v = ctx.iconst16(imm);

            if dst == 30 || dst == 31 {
                ctx.emit_write_ac_mid_sxm((dst - 30) as u32, v);
            } else {
                ctx.emit_write_reg(dst, v);
            }
        }
        OP_MRR => {
            let src = instr.src();
            let dst = instr.dst();
            let v = ctx.emit_read_reg(src);

            if dst == 30 || dst == 31 {
                ctx.emit_write_ac_mid_sxm((dst - 30) as u32, v);
            } else {
                ctx.emit_write_reg(dst, v);
            }
        }
        OP_LR => {
            let dst = instr.d_11_15();
            let imm = instr.imm_16_31();
            let addr = ctx.builder.ins().iconst(types::I32, imm as i64);
            let val_u32 = ctx.emit_read_dmem(addr);
            let val = ctx.builder.ins().ireduce(types::I16, val_u32);

            if dst == 30 || dst == 31 {
                ctx.emit_write_ac_mid_sxm((dst - 30) as u32, val);
            } else {
                ctx.emit_write_reg(dst, val);
            }
        }
        OP_SR => {
            let src = instr.d_11_15();
            let v = ctx.emit_read_reg(src);
            let imm = instr.imm_16_31();
            let addr = ctx.builder.ins().iconst(types::I32, imm as i64);
            let v_u32 = ctx.builder.ins().uextend(types::I32, v);
            ctx.emit_write_dmem(addr, v_u32);
        }
        OP_SI => {
            let addr_low = instr.mem_8_15_u16();
            let addr = (0xFF00u32 | (addr_low as u32)) as i64;
            let addr_v = ctx.builder.ins().iconst(types::I32, addr);
            let val = ctx.builder.ins().iconst(types::I32, instr.imm_16_31() as i64);
            ctx.emit_write_dmem(addr_v, val);
        }
        OP_LRR | OP_LRRD | OP_LRRI | OP_LRRN => {
            let s = instr.s_9_10();
            let dst = instr.d_11_15();
            let ar_off = (abi::dsp_ar_base_offset() + (s as usize) * 2) as i32;
            let ar_u16 = ctx.load_u16(ar_off);
            let addr = ctx.builder.ins().uextend(types::I32, ar_u16);
            let new_ar_i32: Option<Value> = match OP {
                OP_LRRD => Some(ctx.emit_dec_ar(s as u32)),
                OP_LRRI => Some(ctx.emit_inc_ar(s as u32)),
                OP_LRRN => {
                    let ix_off = (abi::dsp_ix_base_offset() + (s as usize) * 2) as i32;
                    let ix = ctx.load_u16(ix_off);
                    Some(ctx.emit_increase_ar(s as u32, ix))
                }
                _ => None,
            };

            let val_u32 = ctx.emit_read_dmem(addr);
            let val = ctx.builder.ins().ireduce(types::I16, val_u32);
            if dst == 30 || dst == 31 {
                ctx.emit_write_ac_mid_sxm((dst - 30) as u32, val);
            } else {
                ctx.emit_write_reg(dst, val);
            }

            if let Some(ar_v) = new_ar_i32 {
                if dst as usize != s as usize {
                    let ar_v16 = ctx.builder.ins().ireduce(types::I16, ar_v);
                    ctx.store_u16(ar_v16, ar_off);
                }
            }
        }
        OP_SRR | OP_SRRD | OP_SRRI | OP_SRRN => {
            let ar_idx = instr.s_9_10();
            let src = instr.d_11_15();
            let v = ctx.emit_read_reg(src);
            let new_ar_i32: Option<Value> = match OP {
                OP_SRRD => Some(ctx.emit_dec_ar(ar_idx as u32)),
                OP_SRRI => Some(ctx.emit_inc_ar(ar_idx as u32)),
                OP_SRRN => {
                    let ix_off = (abi::dsp_ix_base_offset() + (ar_idx as usize) * 2) as i32;
                    let ix = ctx.load_u16(ix_off);
                    Some(ctx.emit_increase_ar(ar_idx as u32, ix))
                }
                _ => None,
            };

            let ar_off = (abi::dsp_ar_base_offset() + (ar_idx as usize) * 2) as i32;
            let ar_u16 = ctx.load_u16(ar_off);
            let addr = ctx.builder.ins().uextend(types::I32, ar_u16);
            let v_u32 = ctx.builder.ins().uextend(types::I32, v);
            ctx.emit_write_dmem(addr, v_u32);

            if let Some(ar_v) = new_ar_i32 {
                let ar_v16 = ctx.builder.ins().ireduce(types::I16, ar_v);
                ctx.store_u16(ar_v16, ar_off);
            }
        }
        OP_LRS => {
            let dst = 24u8 + instr.reg_5_7();
            let mem_low = instr.mem_8_15_u16();
            let cfg = ctx.load_u16(abi::dsp_config_offset() as i32);
            let cfg_shl = ctx.builder.ins().ishl_imm(cfg, 8);
            let addr_u16 = ctx.builder.ins().bor_imm(cfg_shl, mem_low as i64);
            let addr = ctx.builder.ins().uextend(types::I32, addr_u16);
            let val_u32 = ctx.emit_read_dmem(addr);
            let val = ctx.builder.ins().ireduce(types::I16, val_u32);

            if dst >= 30 {
                ctx.emit_write_ac_mid_sxm((dst - 30) as u32, val);
            } else {
                ctx.try_emit_reg_write(dst, val);
            }
        }
        OP_SRS => {
            let src = 28u8 + instr.reg_6_7();
            let v = ctx.emit_read_reg(src);
            let mem_low = instr.mem_8_15_u16();

            let cfg = ctx.load_u16(abi::dsp_config_offset() as i32);
            let cfg_shl = ctx.builder.ins().ishl_imm(cfg, 8);
            let addr_u16 = ctx.builder.ins().bor_imm(cfg_shl, mem_low as i64);
            let addr = ctx.builder.ins().uextend(types::I32, addr_u16);
            let v_u32 = ctx.builder.ins().uextend(types::I32, v);
            ctx.emit_write_dmem(addr, v_u32);
        }
        OP_SRSH => {
            let src = if instr.s_7_7() != 0 { 17u8 } else { 16u8 };
            let v = ctx.try_emit_reg_read(src).expect("AC*H is simple");
            let mem_low = instr.mem_8_15_u16();

            let cfg = ctx.load_u16(abi::dsp_config_offset() as i32);
            let cfg_shl = ctx.builder.ins().ishl_imm(cfg, 8);
            let addr_u16 = ctx.builder.ins().bor_imm(cfg_shl, mem_low as i64);
            let addr = ctx.builder.ins().uextend(types::I32, addr_u16);
            let v_u32 = ctx.builder.ins().uextend(types::I32, v);
            ctx.emit_write_dmem(addr, v_u32);
        }
        OP_ILRR | OP_ILRRD | OP_ILRRI | OP_ILRRN => {
            let src = instr.s_14_15() as usize;
            let d = instr.d_7_7();
            let ar_off = (abi::dsp_ar_base_offset() + src * 2) as i32;
            let ar = ctx.load_u16(ar_off);
            let addr = ctx.builder.ins().uextend(types::I32, ar);
            let new_ar: Option<Value> = match OP {
                OP_ILRRD => Some(ctx.emit_dec_ar(src as u32)),
                OP_ILRRI => Some(ctx.emit_inc_ar(src as u32)),
                OP_ILRRN => {
                    let ix_off = (abi::dsp_ix_base_offset() + src * 2) as i32;
                    let ix = ctx.load_u16(ix_off);
                    Some(ctx.emit_increase_ar(src as u32, ix))
                }
                _ => None,
            };

            let val_u32 = ctx.emit_read_imem(addr);
            let val = ctx.builder.ins().ireduce(types::I16, val_u32);
            ctx.emit_write_ac_mid_sxm(d as u32, val);

            if let Some(ar_v) = new_ar {
                let ar_v16 = ctx.builder.ins().ireduce(types::I16, ar_v);
                ctx.store_u16(ar_v16, ar_off);
            }
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn imm_alu<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_LRIS => {
            let dst = 24u8 + (instr.reg_5_7() as u8);
            let imm_signed = instr.imm_8_15_i16();
            let v = ctx.iconst16(imm_signed as u16);
            ctx.emit_write_reg(dst, v);
        }
        OP_ANDI => {
            let d = instr.d_7_7();
            let imm = instr.imm_16_31();
            let mid = ctx.read_ac_mid(d);
            let result = ctx.builder.ins().band_imm(mid, imm as i64);
            ctx.write_ac_mid(d, result);

            let ac_full = ctx.read_ac_i64(d);
            ctx.emit_update_flags_logic(result, ac_full);
            ctx.emit_clear_oc();
        }
        OP_ORI => {
            let d = instr.d_7_7();
            let imm = instr.imm_16_31();
            let mid = ctx.read_ac_mid(d);
            let result = ctx.builder.ins().bor_imm(mid, imm as i64);
            ctx.write_ac_mid(d, result);

            let ac_full = ctx.read_ac_i64(d);
            ctx.emit_update_flags_logic(result, ac_full);
            ctx.emit_clear_oc();
        }
        OP_XORI => {
            let d = instr.d_7_7();
            let imm = instr.imm_16_31();
            let mid = ctx.read_ac_mid(d);
            let result = ctx.builder.ins().bxor_imm(mid, imm as i64);
            ctx.write_ac_mid(d, result);

            let ac_full = ctx.read_ac_i64(d);
            ctx.emit_update_flags_logic(result, ac_full);
            ctx.emit_clear_oc();
        }
        OP_ANDF => {
            let d = instr.d_7_7();
            let imm = instr.imm_16_31();
            let mid = ctx.read_ac_mid(d);
            let masked = ctx.builder.ins().band_imm(mid, imm as i64);
            let lz = ctx
                .builder
                .ins()
                .icmp_imm(cranelift_codegen::ir::condcodes::IntCC::Equal, masked, 0);
            let lz_i16 = ctx.builder.ins().uextend(types::I16, lz);
            ctx.emit_set_lz(lz_i16);
        }
        OP_ANDCF => {
            let d = instr.d_7_7();
            let imm = instr.imm_16_31();
            let mid = ctx.read_ac_mid(d);
            let masked = ctx.builder.ins().band_imm(mid, imm as i64);
            let imm_v = ctx.iconst16(imm);
            let lz = ctx
                .builder
                .ins()
                .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, masked, imm_v);
            let lz_i16 = ctx.builder.ins().uextend(types::I16, lz);
            ctx.emit_set_lz(lz_i16);
        }
        OP_ADDI => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);

            let b = (instr.imm_16_31() as i16 as i64) << 16;
            let b_v = ctx.builder.ins().iconst(types::I64, b);
            let result = ctx.builder.ins().iadd(a, b_v);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b_v, new_ac);
        }
        OP_ADDIS => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = (instr.imm_8_15_i16() as i64) << 16;
            let b_v = ctx.builder.ins().iconst(types::I64, b);
            let result = ctx.builder.ins().iadd(a, b_v);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b_v, new_ac);
        }
        OP_CMPI => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = (instr.imm_16_31() as i16 as i64) << 16;
            let b_v = ctx.builder.ins().iconst(types::I64, b);
            let result = ctx.builder.ins().isub(a, b_v);
            ctx.emit_update_flags_sub(a, b_v, result);
        }
        OP_CMPIS => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = (instr.imm_8_15_i16() as i64) << 16;
            let b_v = ctx.builder.ins().iconst(types::I64, b);
            let result = ctx.builder.ins().isub(a, b_v);
            ctx.emit_update_flags_sub(a, b_v, result);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn add_sub<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    fn read_simple_signed_i64(ctx: &mut TranslatorCtx, slot: u8) -> Value {
        let raw = ctx.try_emit_reg_read(slot).expect("simple slot");
        ctx.builder.ins().sextend(types::I64, raw)
    }

    fn read_axh_ax_combined(ctx: &mut TranslatorCtx, s: u8) -> Value {
        let axh_off = (abi::dsp_axh_base_offset() + (s as usize) * 2) as i32;
        let ax_off = (abi::dsp_ax_base_offset() + (s as usize) * 2) as i32;
        let axh = ctx.load_u16(axh_off);
        let ax = ctx.load_u16(ax_off);
        let axh_i64 = ctx.builder.ins().sextend(types::I64, axh);
        let axh_shl = ctx.builder.ins().ishl_imm(axh_i64, 16);
        let ax_i64 = ctx.builder.ins().uextend(types::I64, ax);
        ctx.builder.ins().bor(axh_shl, ax_i64)
    }

    match OP {
        OP_ADDR => {
            let ss = instr.ss();
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let src = read_simple_signed_i64(ctx, 24 + ss);
            let b = ctx.builder.ins().ishl_imm(src, 16);
            let result = ctx.builder.ins().iadd(a, b);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b, new_ac);
        }
        OP_ADD => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = ctx.read_ac_i64(1 - d);
            let result = ctx.builder.ins().iadd(a, b);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b, new_ac);
        }
        OP_ADDAX => {
            let s = instr.s_6_6();
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = read_axh_ax_combined(ctx, s);
            let result = ctx.builder.ins().iadd(a, b);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b, new_ac);
        }
        OP_ADDAXL => {
            let s = instr.s_6_6();
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let ax_off = (abi::dsp_ax_base_offset() + (s as usize) * 2) as i32;
            let ax = ctx.load_u16(ax_off);
            let b = ctx.builder.ins().uextend(types::I64, ax);
            let result = ctx.builder.ins().iadd(a, b);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b, new_ac);
        }
        OP_SUBR => {
            let ss = instr.ss();
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let src = read_simple_signed_i64(ctx, 24 + ss);
            let b = ctx.builder.ins().ishl_imm(src, 16);
            let result = ctx.builder.ins().isub(a, b);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_sub(a, b, new_ac);
        }
        OP_SUB => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = ctx.read_ac_i64(1 - d);
            let result = ctx.builder.ins().isub(a, b);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_sub(a, b, new_ac);
        }
        OP_SUBAX => {
            let s = instr.s_6_6();
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = read_axh_ax_combined(ctx, s);
            let result = ctx.builder.ins().isub(a, b);
            ctx.write_ac_i64(d, result);
            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_sub(a, b, new_ac);
        }
        OP_ADDP => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let (pc, po) = ctx.product_flags_i16();
            let b = ctx.read_product_i64();
            let result = ctx.builder.ins().iadd(a, b);
            let os_before = ctx.read_status_bit(7);
            ctx.write_ac_i64(d, result);
            ctx.emit_update_flags_add(a, b, result);
            ctx.set_status_bit(7, os_before);

            let c_now = ctx.read_status_bit(0);
            let o_now = ctx.read_status_bit(1);
            let c_xor = ctx.builder.ins().bxor(c_now, pc);
            let o_xor = ctx.builder.ins().bxor(o_now, po);
            ctx.set_status_bit(0, c_xor);
            ctx.set_status_bit(1, o_xor);
            ctx.or_status_bit(7, o_xor);
        }
        OP_SUBP => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let (pc, po) = ctx.product_flags_i16();
            let b = ctx.read_product_i64();
            let result = ctx.builder.ins().isub(a, b);
            let os_before = ctx.read_status_bit(7);
            ctx.write_ac_i64(d, result);
            ctx.emit_update_flags_sub(a, b, result);
            ctx.set_status_bit(7, os_before);

            let c_now = ctx.read_status_bit(0);
            let o_now = ctx.read_status_bit(1);

            let one = ctx.iconst16(1);
            let not_pc = ctx.builder.ins().isub(one, pc);
            let c_xor = ctx.builder.ins().bxor(c_now, not_pc);
            let o_xor = ctx.builder.ins().bxor(o_now, po);
            ctx.set_status_bit(0, c_xor);
            ctx.set_status_bit(1, o_xor);
            ctx.or_status_bit(7, o_xor);
        }
        OP_ADDPAXZ => {
            use cranelift_codegen::ir::condcodes::IntCC;

            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let (pc, po) = ctx.product_flags_i16();
            let a = ctx.read_product_i64();
            let axh_off = (abi::dsp_axh_base_offset() + s * 2) as i32;
            let axh16 = ctx.load_u16(axh_off);
            let axh_i64 = ctx.builder.ins().sextend(types::I64, axh16);
            let b = ctx.builder.ins().ishl_imm(axh_i64, 16);

            let val40 = ctx.builder.ins().band_imm(a, 0xFF_FFFF_FFFF_i64);
            let lower = ctx.builder.ins().band_imm(val40, 0xFFFF);
            let sum_pre = ctx.builder.ins().iadd(val40, b);
            let sum40 = ctx.builder.ins().band_imm(sum_pre, 0xFF_FFFF_FFFF_i64);
            let bit16 = ctx.builder.ins().band_imm(sum40, 0x10000);
            let bit16_set = ctx.builder.ins().icmp_imm(IntCC::NotEqual, bit16, 0);
            let lower_gt = ctx.builder.ins().icmp_imm(IntCC::UnsignedGreaterThan, lower, 0x8000);
            let lower_eq = ctx.builder.ins().icmp_imm(IntCC::Equal, lower, 0x8000);
            let lower_eq_and_bit16 = ctx.builder.ins().band(lower_eq, bit16_set);
            let round_up = ctx.builder.ins().bor(lower_gt, lower_eq_and_bit16);

            let truncated = ctx.builder.ins().band_imm(val40, !0xFFFF_i64);
            let plus = ctx.builder.ins().iadd_imm(val40, 0x10000);
            let rounded_path = ctx.builder.ins().band_imm(plus, !0xFFFF_i64);
            let plus_overflow = ctx
                .builder
                .ins()
                .icmp_imm(IntCC::UnsignedGreaterThan, plus, 0xFF_FFFF_FFFF_i64);
            let plus_overflow_i16 = ctx.builder.ins().uextend(types::I16, plus_overflow);
            let zero_i16 = ctx.iconst16(0);

            let rounded = ctx.builder.ins().select(round_up, rounded_path, truncated);
            let rounding_carry = ctx.builder.ins().select(round_up, plus_overflow_i16, zero_i16);

            let result_pre = ctx.builder.ins().iadd(rounded, b);
            let result = ctx.builder.ins().band_imm(result_pre, !0xFFFF_i64);

            let os_before = ctx.read_status_bit(7);
            ctx.write_ac_i64(d, result);
            ctx.emit_update_flags_add(rounded, b, result);

            let c_in = ctx.builder.ins().bor(pc, rounding_carry);
            ctx.set_status_bit(7, os_before);

            let c_now = ctx.read_status_bit(0);
            let o_now = ctx.read_status_bit(1);
            let c_xor = ctx.builder.ins().bxor(c_now, c_in);
            let o_xor = ctx.builder.ins().bxor(o_now, po);
            ctx.set_status_bit(0, c_xor);
            ctx.set_status_bit(1, o_xor);
            ctx.or_status_bit(7, o_xor);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn move_ops<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_MOVR => {
            let ss = instr.ss();
            let d = instr.d_7_7();

            let raw = ctx.try_emit_reg_read(24 + ss).expect("simple slot");
            let signed = ctx.builder.ins().sextend(types::I64, raw);
            let val = ctx.builder.ins().ishl_imm(signed, 16);
            ctx.write_ac_i64(d, val);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_ac(new_ac);
            ctx.emit_clear_oc();
        }
        OP_MOVAX => {
            let s = instr.s_6_6();
            let d = instr.d_7_7();
            let axh_off = (abi::dsp_axh_base_offset() + (s as usize) * 2) as i32;
            let ax_off = (abi::dsp_ax_base_offset() + (s as usize) * 2) as i32;
            let axh = ctx.load_u16(axh_off);
            let ax = ctx.load_u16(ax_off);
            let axh_i64 = ctx.builder.ins().sextend(types::I64, axh);
            let axh_shl = ctx.builder.ins().ishl_imm(axh_i64, 16);
            let ax_i64 = ctx.builder.ins().uextend(types::I64, ax);
            let val = ctx.builder.ins().bor(axh_shl, ax_i64);
            ctx.write_ac_i64(d, val);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_ac(new_ac);
            ctx.emit_clear_oc();
        }
        OP_MOV => {
            let d = instr.d_7_7();
            let val = ctx.read_ac_i64(1 - d);
            ctx.write_ac_i64(d, val);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_ac(new_ac);
            ctx.emit_clear_oc();
        }
        OP_MOVP => {
            let d = instr.d_7_7();
            ctx.move_prod_to_ac(d);
        }
        OP_MOVPZ => {
            let d = instr.d_7_7();
            ctx.move_prod_to_ac_zero(d);
        }
        OP_MOVNP => {
            use cranelift_codegen::ir::condcodes::IntCC;

            let d = instr.d_7_7();
            let prod = ctx.read_product_i64();
            let (carry, overflow) = ctx.product_flags_i16();
            let val = ctx.builder.ins().ineg(prod);
            ctx.write_ac_i64(d, val);
            ctx.emit_update_flags_ac(val);

            ctx.set_status_bit(1, overflow);
            ctx.or_status_bit(7, overflow);

            let prod40 = ctx.builder.ins().band_imm(prod, 0xFF_FFFF_FFFF_i64);
            let prod_zero = ctx.builder.ins().icmp_imm(IntCC::Equal, prod40, 0);
            let prod_zero_i16 = ctx.builder.ins().uextend(types::I16, prod_zero);
            let one = ctx.iconst16(1);
            let not_carry = ctx.builder.ins().isub(one, carry);
            let c_val = ctx.builder.ins().bxor(not_carry, prod_zero_i16);
            ctx.set_status_bit(0, c_val);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn inc_dec<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_INCM => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b = 0x10000i64;
            let b_v = ctx.builder.ins().iconst(types::I64, b);
            let result = ctx.builder.ins().iadd(a, b_v);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b_v, new_ac);
        }
        OP_INC => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b_v = ctx.builder.ins().iconst(types::I64, 1);
            let result = ctx.builder.ins().iadd(a, b_v);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_add(a, b_v, new_ac);
        }
        OP_DECM => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b_v = ctx.builder.ins().iconst(types::I64, 0x10000);
            let result = ctx.builder.ins().isub(a, b_v);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_sub(a, b_v, new_ac);
        }
        OP_DEC => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let b_v = ctx.builder.ins().iconst(types::I64, 1);
            let result = ctx.builder.ins().isub(a, b_v);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_sub(a, b_v, new_ac);
        }
        OP_NEG => {
            let d = instr.d_7_7();
            let a = ctx.read_ac_i64(d);
            let zero = ctx.builder.ins().iconst(types::I64, 0);
            let result = ctx.builder.ins().isub(zero, a);
            ctx.write_ac_i64(d, result);

            let new_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_sub(zero, a, new_ac);
        }
        OP_ABS_AC => {
            use cranelift_codegen::ir::condcodes::IntCC;

            let d = instr.d_4_4();
            let a = ctx.read_ac_i64(d);
            let was_negative = ctx.builder.ins().icmp_imm(IntCC::SignedLessThan, a, 0);
            let neg = ctx.builder.ins().ineg(a);

            let new_ac = ctx.builder.ins().select(was_negative, neg, a);
            ctx.write_ac_i64(d, new_ac);

            let stored_ac = ctx.read_ac_i64(d);
            ctx.emit_update_flags_ac(stored_ac);

            let still_neg = ctx.builder.ins().icmp_imm(IntCC::SignedLessThan, stored_ac, 0);
            let o_b8 = ctx.builder.ins().band(was_negative, still_neg);
            let o_i16 = ctx.builder.ins().uextend(types::I16, o_b8);
            ctx.set_status_bit(1, o_i16);
            ctx.or_status_bit(7, o_i16);
            ctx.set_status_bit_const(0, false);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn cmp_test<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_CMP => {
            let a = ctx.read_ac_i64(0);
            let b = ctx.read_ac_i64(1);
            let result = ctx.builder.ins().isub(a, b);
            ctx.emit_update_flags_sub(a, b, result);
        }
        OP_CMPAXH => {
            let s = instr.s_4_4();
            let r = instr.s_3_3();
            let a = ctx.read_ac_i64(s);
            let axh_off = (abi::dsp_axh_base_offset() + (r as usize) * 2) as i32;
            let axh = ctx.load_u16(axh_off);
            let axh_signed = ctx.builder.ins().sextend(types::I64, axh);
            let b = ctx.builder.ins().ishl_imm(axh_signed, 16);
            let result = ctx.builder.ins().isub(a, b);
            ctx.emit_update_flags_sub(a, b, result);
        }
        OP_TST => {
            let r = instr.r_4_4();
            let ac = ctx.read_ac_i64(r);
            ctx.emit_update_flags_ac(ac);
            ctx.emit_clear_oc();
        }
        OP_TSTAXH => {
            let r = instr.r_7_7();
            let axh_off = (abi::dsp_axh_base_offset() + (r as usize) * 2) as i32;
            let axh = ctx.load_u16(axh_off);
            let axh_signed = ctx.builder.ins().sextend(types::I64, axh);
            let val = ctx.builder.ins().ishl_imm(axh_signed, 16);
            ctx.emit_update_flags_ac(val);

            let sr_off = abi::dsp_status_offset() as i32;
            let sr = ctx.load_u16(sr_off);
            let mask = !((1i64 << 4) | (1 << 1) | 1) & 0xFFFF;
            let cleared = ctx.builder.ins().band_imm(sr, mask);
            ctx.store_u16(cleared, sr_off);
        }
        OP_NX_0 | OP_NX_1 => {}
        OP_CLR => {
            let r = instr.r_4_4();
            let zero64 = ctx.builder.ins().iconst(types::I64, 0);
            ctx.write_ac_i64(r, zero64);

            let sr_off = abi::dsp_status_offset() as i32;
            let sr = ctx.load_u16(sr_off);
            let mask = !((1i64 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5)) & 0xFFFF;
            let cleared = ctx.builder.ins().band_imm(sr, mask);
            let updated = ctx.builder.ins().bor_imm(cleared, (1i64 << 5) | (1 << 2));
            ctx.store_u16(updated, sr_off);
        }
        OP_CLRP => {
            let pl = ctx.iconst16(0x0000);
            let pm1 = ctx.iconst16(0xFFF0);
            let ph = ctx.iconst16(0x00FF);
            let pm2 = ctx.iconst16(0x0010);
            ctx.store_u16(pl, abi::dsp_product_low_offset() as i32);
            ctx.store_u16(pm1, abi::dsp_product_mid1_offset() as i32);
            ctx.store_u16(ph, abi::dsp_product_high_offset() as i32);
            ctx.store_u16(pm2, abi::dsp_product_mid2_offset() as i32);
        }
        OP_TSTPROD => {
            let (pc, po) = ctx.product_flags_i16();
            let prod = ctx.read_product_i64();
            ctx.emit_update_flags_ac(prod);
            ctx.set_status_bit(1, po);
            ctx.or_status_bit(7, po);
            ctx.set_status_bit(0, pc);
        }
        OP_CLRL => {
            let r = instr.r_7_7();
            let ac = ctx.read_ac_i64(r);
            let (rounded, carry) = ctx.round_half_to_even(ac);
            ctx.write_ac_i64(r, rounded);
            ctx.emit_update_flags_ac(rounded);
            ctx.set_status_bit_const(1, false);
            ctx.set_status_bit(0, carry);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn logic<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    fn emit_logic_with_axh(ctx: &mut TranslatorCtx, d: u8, s: u8, kind: LogicKind) {
        let mid = ctx.read_ac_mid(d);
        let axh_off = (abi::dsp_axh_base_offset() + (s as usize) * 2) as i32;
        let other = ctx.load_u16(axh_off);
        let result = match kind {
            LogicKind::And => ctx.builder.ins().band(mid, other),
            LogicKind::Or => ctx.builder.ins().bor(mid, other),
            LogicKind::Xor => ctx.builder.ins().bxor(mid, other),
        };
        ctx.write_ac_mid(d, result);

        let ac_full = ctx.read_ac_i64(d);
        ctx.emit_update_flags_logic(result, ac_full);
        ctx.emit_clear_oc();
    }

    fn emit_logic_with_other_ac_mid(ctx: &mut TranslatorCtx, d: u8, kind: LogicKind) {
        let mid = ctx.read_ac_mid(d);
        let other_d = 1 - d;
        let other = ctx.read_ac_mid(other_d);

        let result = match kind {
            LogicKind::And => ctx.builder.ins().band(mid, other),
            LogicKind::Or => ctx.builder.ins().bor(mid, other),
            LogicKind::Xor => ctx.builder.ins().bxor(mid, other),
        };
        ctx.write_ac_mid(d, result);

        let ac_full = ctx.read_ac_i64(d);
        ctx.emit_update_flags_logic(result, ac_full);
        ctx.emit_clear_oc();
    }

    match OP {
        OP_XORR => emit_logic_with_axh(ctx, instr.d_7_7(), instr.s_6_6(), LogicKind::Xor),
        OP_ANDR => emit_logic_with_axh(ctx, instr.d_7_7(), instr.s_6_6(), LogicKind::And),
        OP_ORR => emit_logic_with_axh(ctx, instr.d_7_7(), instr.s_6_6(), LogicKind::Or),
        OP_ANDC => emit_logic_with_other_ac_mid(ctx, instr.d_7_7(), LogicKind::And),
        OP_ORC => emit_logic_with_other_ac_mid(ctx, instr.d_7_7(), LogicKind::Or),
        OP_XORC => emit_logic_with_other_ac_mid(ctx, instr.d_7_7(), LogicKind::Xor),
        OP_NOT_AC => {
            let d = instr.d_7_7();
            let mid = ctx.read_ac_mid(d);
            let result = ctx.builder.ins().bnot(mid);
            ctx.write_ac_mid(d, result);

            let ac_full = ctx.read_ac_i64(d);
            ctx.emit_update_flags_logic(result, ac_full);
            ctx.emit_clear_oc();
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[derive(Clone, Copy)]
enum LogicKind {
    And,
    Or,
    Xor,
}

#[inline(always)]
pub fn shifts<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    fn finish(ctx: &mut TranslatorCtx, d: u8, val: Value) {
        ctx.write_ac_i64(d, val);
        let ac = ctx.read_ac_i64(d);
        ctx.emit_update_flags_ac(ac);
        ctx.emit_clear_oc();
    }

    match OP {
        OP_LSL | OP_ASL => {
            let r = instr.r_7_7();
            let i = instr.n() as i64;
            let ac = ctx.read_ac_i64(r);
            let masked = ctx.builder.ins().band_imm(ac, 0xFF_FFFF_FFFFi64);
            let shifted = ctx.builder.ins().ishl_imm(masked, i);
            finish(ctx, r, shifted);
        }
        OP_LSR => {
            let r = instr.r_7_7();
            let i = instr.n() as i64;

            if i != 0 {
                let ac = ctx.read_ac_i64(r);
                let masked = ctx.builder.ins().band_imm(ac, 0xFF_FFFF_FFFFi64);
                let shifted = ctx.builder.ins().ushr_imm(masked, 64 - i);
                finish(ctx, r, shifted);
            } else {
                let ac = ctx.read_ac_i64(r);
                ctx.emit_update_flags_ac(ac);
                ctx.emit_clear_oc();
            }
        }
        OP_ASR => {
            let r = instr.r_7_7();
            let i = instr.n() as i64;

            if i != 0 {
                let ac = ctx.read_ac_i64(r);
                let shifted = ctx.builder.ins().sshr_imm(ac, 64 - i);
                finish(ctx, r, shifted);
            } else {
                let ac = ctx.read_ac_i64(r);
                ctx.emit_update_flags_ac(ac);
                ctx.emit_clear_oc();
            }
        }
        OP_LSL16 => {
            let r = instr.r_7_7();
            let ac = ctx.read_ac_i64(r);
            let masked = ctx.builder.ins().band_imm(ac, 0xFF_FFFF_FFFFi64);
            let shifted = ctx.builder.ins().ishl_imm(masked, 16);
            finish(ctx, r, shifted);
        }
        OP_LSR16 => {
            let r = instr.r_7_7();
            let ac = ctx.read_ac_i64(r);
            let masked = ctx.builder.ins().band_imm(ac, 0xFF_FFFF_FFFFi64);
            let shifted = ctx.builder.ins().ushr_imm(masked, 16);
            finish(ctx, r, shifted);
        }
        OP_ASR16 => {
            let r = instr.r_4_4();
            let ac = ctx.read_ac_i64(r);
            let shifted = ctx.builder.ins().sshr_imm(ac, 16);
            finish(ctx, r, shifted);
        }
        OP_LSRN => {
            let shift = ctx.load_u16(abi::dsp_ac1_mid_offset() as i32);
            ctx.emit_dynamic_shift(0, shift, 0b01);
        }
        OP_ASRN => {
            let shift = ctx.load_u16(abi::dsp_ac1_mid_offset() as i32);
            ctx.emit_dynamic_shift(0, shift, 0b00);
        }
        OP_LSRNRX => {
            let s = instr.s_6_6();
            let d = instr.d_7_7();
            let axh_off = (abi::dsp_axh_base_offset() + (s as usize) * 2) as i32;
            let shift = ctx.load_u16(axh_off);
            ctx.emit_dynamic_shift(d as u32, shift, 0b11);
        }
        OP_ASRNRX => {
            let s = instr.s_6_6();
            let d = instr.d_7_7();
            let axh_off = (abi::dsp_axh_base_offset() + (s as usize) * 2) as i32;
            let shift = ctx.load_u16(axh_off);
            ctx.emit_dynamic_shift(d as u32, shift, 0b10);
        }
        OP_LSRNR => {
            let d = instr.d_7_7();
            let other = ctx.read_ac_mid(1 - d);
            ctx.emit_dynamic_shift(d as u32, other, 0b11);
        }
        OP_ASRNR => {
            let d = instr.d_7_7();
            let other = ctx.read_ac_mid(1 - d);
            ctx.emit_dynamic_shift(d as u32, other, 0b10);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn status<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    let sr_off = abi::dsp_status_offset() as i32;

    match OP {
        OP_SBCLR => {
            let idx = 6 + instr.bit();
            if idx == 13 {
                return;
            }

            let sr = ctx.load_u16(sr_off);
            let mask = !(1u16 << idx);
            let new_sr = ctx.builder.ins().band_imm(sr, mask as i64);
            ctx.store_u16(new_sr, sr_off);
        }
        OP_SBSET => {
            let idx = 6 + instr.bit();
            if idx == 13 || idx == 8 {
                return;
            }

            let sr = ctx.load_u16(sr_off);
            let bit = 1i64 << idx;
            let new_sr = ctx.builder.ins().bor_imm(sr, bit);
            ctx.store_u16(new_sr, sr_off);
        }
        OP_M2 => {
            let sr = ctx.load_u16(sr_off);
            let new_sr = ctx.builder.ins().band_imm(sr, !(1i64 << 13) & 0xFFFF);
            ctx.store_u16(new_sr, sr_off);
        }
        OP_M0 => {
            let sr = ctx.load_u16(sr_off);
            let new_sr = ctx.builder.ins().bor_imm(sr, 1i64 << 13);
            ctx.store_u16(new_sr, sr_off);
        }
        OP_CLR15 => {
            let sr = ctx.load_u16(sr_off);
            let new_sr = ctx.builder.ins().band_imm(sr, !(1i64 << 15) & 0xFFFF);
            ctx.store_u16(new_sr, sr_off);
        }
        OP_SET15 => {
            let sr = ctx.load_u16(sr_off);
            let new_sr = ctx.builder.ins().bor_imm(sr, 1i64 << 15);
            ctx.store_u16(new_sr, sr_off);
        }
        OP_SET16 => {
            let sr = ctx.load_u16(sr_off);
            let new_sr = ctx.builder.ins().band_imm(sr, !(1i64 << 14) & 0xFFFF);
            ctx.store_u16(new_sr, sr_off);
        }
        OP_SET40 => {
            let sr = ctx.load_u16(sr_off);
            let new_sr = ctx.builder.ins().bor_imm(sr, 1i64 << 14);
            ctx.store_u16(new_sr, sr_off);
        }
        _ => unreachable!("DSP JIT: unexpected OP routed to native translator"),
    }
}

#[inline(always)]
pub fn mul<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: Instruction) {
    match OP {
        OP_MUL => {
            let s = (instr.r_4_4() as usize) * 2;
            let ax_off = (abi::dsp_ax_base_offset() + s) as i32;
            let axh_off = (abi::dsp_axh_base_offset() + s) as i32;
            let a = ctx.load_u16(ax_off);
            let b = ctx.load_u16(axh_off);
            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        OP_MULAXH => {
            let axh_off = abi::dsp_axh_base_offset() as i32;
            let a = ctx.load_u16(axh_off);
            let p = ctx.multiply_i64(a, a);
            ctx.write_product_i64(p);
        }
        OP_MULX => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            ctx.multiply_mulx(s, t);
        }
        OP_MULC => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            let (a, b) = ctx.mulc_operands_i16(s, t);
            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        OP_MADD => {
            let s = (instr.s_7_7() as usize) * 2;
            let ax_off = (abi::dsp_ax_base_offset() + s) as i32;
            let axh_off = (abi::dsp_axh_base_offset() + s) as i32;
            let a = ctx.load_u16(ax_off);
            let b = ctx.load_u16(axh_off);
            ctx.multiply_accumulate_i64(a, b, true);
        }
        OP_MSUB => {
            let s = (instr.s_7_7() as usize) * 2;
            let ax_off = (abi::dsp_ax_base_offset() + s) as i32;
            let axh_off = (abi::dsp_axh_base_offset() + s) as i32;
            let a = ctx.load_u16(ax_off);
            let b = ctx.load_u16(axh_off);
            ctx.multiply_accumulate_i64(a, b, false);
        }
        OP_MADDC | OP_MSUBC => {
            let s = instr.s_6_6();
            let t = instr.t_7_7();
            let (a, b) = ctx.mulc_operands_i16(s, t);
            ctx.multiply_accumulate_i64(a, b, OP == OP_MADDC);
        }
        OP_MULMV => {
            let s = (instr.r_4_4() as usize) * 2;
            let r = instr.r_7_7();
            ctx.move_prod_to_ac(r);

            let a = ctx.load_u16((abi::dsp_ax_base_offset() + s) as i32);
            let b = ctx.load_u16((abi::dsp_axh_base_offset() + s) as i32);
            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        OP_MULAC => {
            let s = (instr.r_4_4() as usize) * 2;
            let r = instr.r_7_7();
            ctx.add_prod_to_ac(r);

            let a = ctx.load_u16((abi::dsp_ax_base_offset() + s) as i32);
            let b = ctx.load_u16((abi::dsp_axh_base_offset() + s) as i32);
            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        OP_MULXMV => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            let r = instr.r_7_7();
            ctx.move_prod_to_ac(r);
            ctx.multiply_mulx(s, t);
        }
        OP_MULXAC => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            let r = instr.r_7_7();
            ctx.add_prod_to_ac(r);
            ctx.multiply_mulx(s, t);
        }
        OP_MULCMV => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            let r = instr.r_7_7();
            let (a, b) = ctx.mulc_operands_i16(s, t);
            ctx.move_prod_to_ac(r);

            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        OP_MULCAC => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            let r = instr.r_7_7();
            let (a, b) = ctx.mulc_operands_i16(s, t);
            ctx.add_prod_to_ac(r);

            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        OP_MADDX | OP_MSUBX => {
            let s = instr.s_6_6();
            let t = instr.t_7_7();
            let (a, b) = ctx.mulx_operands_u16(s, t);
            ctx.multiply_accumulate_i64(a, b, OP == OP_MADDX);
        }
        OP_MULMVZ => {
            let s = (instr.r_4_4() as usize) * 2;
            let r = instr.r_7_7();
            ctx.move_prod_to_ac_zero(r);

            let a = ctx.load_u16((abi::dsp_ax_base_offset() + s) as i32);
            let b = ctx.load_u16((abi::dsp_axh_base_offset() + s) as i32);
            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        OP_MULXMVZ => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            let r = instr.r_7_7();
            ctx.move_prod_to_ac_zero(r);
            ctx.multiply_mulx(s, t);
        }
        OP_MULCMVZ => {
            let s = instr.s_3_3();
            let t = instr.t_4_4();
            let r = instr.r_7_7();
            let (a, b) = ctx.mulc_operands_i16(s, t);
            ctx.move_prod_to_ac_zero(r);

            let p = ctx.multiply_i64(a, b);
            ctx.write_product_i64(p);
        }
        _ => unreachable!("DSP JIT: unexpected mul OP"),
    }
}

#[inline(always)]
pub fn ext_addr<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: GcDspExt) {
    let r = instr.r_6_7() as usize;
    let ar_off = (abi::dsp_ar_base_offset() + r * 2) as i32;

    match OP {
        OP_EXT_DR => {
            let new_ar = ctx.emit_dec_ar(r as u32);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            ctx.store_u16(new_ar16, ar_off);
        }
        OP_EXT_IR => {
            let new_ar = ctx.emit_inc_ar(r as u32);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            ctx.store_u16(new_ar16, ar_off);
        }
        OP_EXT_NR => {
            let ix_off = (abi::dsp_ix_base_offset() + r * 2) as i32;
            let ix = ctx.load_u16(ix_off);
            let new_ar = ctx.emit_increase_ar(r as u32, ix);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            ctx.store_u16(new_ar16, ar_off);
        }
        _ => unreachable!("DSP JIT: unexpected ext OP routed to native translator"),
    }
}

#[inline(always)]
pub fn ext_load<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: GcDspExt) {
    let d = instr.d_2_4();
    let s = instr.s_6_7() as usize;
    let dst = 24u8 + d;

    let ar_off = (abi::dsp_ar_base_offset() + s * 2) as i32;
    let ar = ctx.load_u16(ar_off);
    let addr = ctx.builder.ins().uextend(types::I32, ar);
    let val_u32 = ctx.emit_read_dmem(addr);
    let val = ctx.builder.ins().ireduce(types::I16, val_u32);
    if dst >= 30 {
        ctx.emit_write_ac_mid_sxm((dst - 30) as u32, val);
    } else {
        ctx.try_emit_reg_write(dst, val);
    }

    match OP {
        OP_EXT_L => {
            let new_ar = ctx.emit_inc_ar(s as u32);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            ctx.store_u16(new_ar16, ar_off);
        }
        OP_EXT_LN => {
            let ix_off = (abi::dsp_ix_base_offset() + s * 2) as i32;
            let ix = ctx.load_u16(ix_off);
            let new_ar = ctx.emit_increase_ar(s as u32, ix);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            ctx.store_u16(new_ar16, ar_off);
        }
        _ => {}
    }
}

#[inline(always)]
pub fn ext_store<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: GcDspExt) {
    let d = instr.d_6_7() as usize;
    let s = instr.s_3_4() as usize;
    let cache_off = (abi::dsp_ext_ac_cache_base_offset() + s * 2) as i32;
    let value = ctx.load_u16(cache_off);
    let v_u32 = ctx.builder.ins().uextend(types::I32, value);
    let ar_off = (abi::dsp_ar_base_offset() + d * 2) as i32;
    let ar = ctx.load_u16(ar_off);
    let addr = ctx.builder.ins().uextend(types::I32, ar);
    ctx.emit_write_dmem(addr, v_u32);

    match OP {
        OP_EXT_S => {
            let new_ar = ctx.emit_inc_ar(d as u32);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            ctx.store_u16(new_ar16, ar_off);
        }
        OP_EXT_SN => {
            let ix_off = (abi::dsp_ix_base_offset() + d * 2) as i32;
            let ix = ctx.load_u16(ix_off);
            let new_ar = ctx.emit_increase_ar(d as u32, ix);
            let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
            ctx.store_u16(new_ar16, ar_off);
        }
        _ => {}
    }
}

#[inline(always)]
pub fn ext_load_store<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: GcDspExt) {
    let s = instr.s_7_7() as usize;
    let d = instr.d_2_3();
    let dst = 24u8 + d;

    let ar0_off = abi::dsp_ar_base_offset() as i32;
    let ar3_off = (abi::dsp_ar_base_offset() + 3 * 2) as i32;
    let ix0_off = abi::dsp_ix_base_offset() as i32;
    let ix3_off = (abi::dsp_ix_base_offset() + 3 * 2) as i32;
    let cache_off = (abi::dsp_ext_ac_cache_base_offset() + (4 + s) * 2) as i32;

    let is_ls = matches!(OP, OP_EXT_LS | OP_EXT_LSM | OP_EXT_LSN | OP_EXT_LSNM);
    if is_ls {
        let ar0 = ctx.load_u16(ar0_off);
        let load_addr = ctx.builder.ins().uextend(types::I32, ar0);
        let load_val_u32 = ctx.emit_read_dmem(load_addr);
        let load_val = ctx.builder.ins().ireduce(types::I16, load_val_u32);
        ctx.try_emit_reg_write(dst, load_val);

        let store_val = ctx.load_u16(cache_off);
        let store_val_u32 = ctx.builder.ins().uextend(types::I32, store_val);
        let ar3 = ctx.load_u16(ar3_off);
        let store_addr = ctx.builder.ins().uextend(types::I32, ar3);
        ctx.emit_write_dmem(store_addr, store_val_u32);
    } else {
        let store_val = ctx.load_u16(cache_off);
        let store_val_u32 = ctx.builder.ins().uextend(types::I32, store_val);
        let ar0 = ctx.load_u16(ar0_off);
        let store_addr = ctx.builder.ins().uextend(types::I32, ar0);
        ctx.emit_write_dmem(store_addr, store_val_u32);

        let ar3 = ctx.load_u16(ar3_off);
        let load_addr = ctx.builder.ins().uextend(types::I32, ar3);
        let load_val_u32 = ctx.emit_read_dmem(load_addr);
        let load_val = ctx.builder.ins().ireduce(types::I16, load_val_u32);
        ctx.try_emit_reg_write(dst, load_val);
    }

    let (a0_inc, a3_inc) = match OP {
        OP_EXT_LS | OP_EXT_SL => (true, true),
        OP_EXT_LSM | OP_EXT_SLM => (true, false),
        OP_EXT_LSN | OP_EXT_SLN => (false, true),
        OP_EXT_LSNM | OP_EXT_SLNM => (false, false),
        _ => unreachable!(),
    };

    if a0_inc {
        let new_ar = ctx.emit_inc_ar(0);
        let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
        ctx.store_u16(new_ar16, ar0_off);
    } else {
        let ix0 = ctx.load_u16(ix0_off);
        let new_ar = ctx.emit_increase_ar(0, ix0);
        let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
        ctx.store_u16(new_ar16, ar0_off);
    }

    if a3_inc {
        let new_ar = ctx.emit_inc_ar(3);
        let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
        ctx.store_u16(new_ar16, ar3_off);
    } else {
        let ix3 = ctx.load_u16(ix3_off);
        let new_ar = ctx.emit_increase_ar(3, ix3);
        let new_ar16 = ctx.builder.ins().ireduce(types::I16, new_ar);
        ctx.store_u16(new_ar16, ar3_off);
    }
}

#[inline(always)]
pub fn ext_ld<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: GcDspExt) {
    let d = instr.d_2_2();
    let r = instr.r_3_3();
    let s = instr.s_6_7() as usize;

    let ar_s_off = (abi::dsp_ar_base_offset() + s * 2) as i32;
    let ar_3_off = (abi::dsp_ar_base_offset() + 3 * 2) as i32;
    let ar_s = ctx.load_u16(ar_s_off);
    let ar_3 = ctx.load_u16(ar_3_off);

    let s_bank = ctx.builder.ins().ushr_imm(ar_s, 10);
    let three_bank = ctx.builder.ins().ushr_imm(ar_3, 10);
    let same_bank_i8 = ctx
        .builder
        .ins()
        .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, s_bank, three_bank);

    let addr0 = ctx.builder.ins().uextend(types::I32, ar_s);
    let value0_u32 = ctx.emit_read_dmem(addr0);
    let value0 = ctx.builder.ins().ireduce(types::I16, value0_u32);
    let d_reg = if d != 0 { 26u8 } else { 24u8 };
    ctx.try_emit_reg_write(d_reg, value0);

    let value1 = {
        let same_block = ctx.builder.create_block();
        let diff_block = ctx.builder.create_block();
        let join_block = ctx.builder.create_block();
        ctx.builder.append_block_param(join_block, types::I16);
        ctx.builder.ins().brif(same_bank_i8, same_block, &[], diff_block, &[]);

        ctx.builder.switch_to_block(same_block);
        ctx.builder.seal_block(same_block);
        ctx.builder.ins().jump(join_block, &[value0.into()]);

        ctx.builder.switch_to_block(diff_block);
        ctx.builder.seal_block(diff_block);
        let addr1 = ctx.builder.ins().uextend(types::I32, ar_3);
        let v1_u32 = ctx.emit_read_dmem(addr1);
        let v1 = ctx.builder.ins().ireduce(types::I16, v1_u32);
        ctx.builder.ins().jump(join_block, &[v1.into()]);

        ctx.builder.switch_to_block(join_block);
        ctx.builder.seal_block(join_block);
        ctx.builder.block_params(join_block)[0]
    };

    let r_reg = if r != 0 { 27u8 } else { 25u8 };
    ctx.try_emit_reg_write(r_reg, value1);

    let (s_inc, three_inc) = match OP {
        OP_EXT_LD_00 => (true, true),
        OP_EXT_LDN_01 => (false, true),
        OP_EXT_LDM_10 => (true, false),
        OP_EXT_LDNM_11 => (false, false),
        _ => unreachable!(),
    };

    let new_s = if s_inc {
        ctx.emit_inc_ar(s as u32)
    } else {
        let ix_off = (abi::dsp_ix_base_offset() + s * 2) as i32;
        let ix = ctx.load_u16(ix_off);
        ctx.emit_increase_ar(s as u32, ix)
    };
    let new_s16 = ctx.builder.ins().ireduce(types::I16, new_s);
    ctx.store_u16(new_s16, ar_s_off);

    let new_3 = if three_inc {
        ctx.emit_inc_ar(3)
    } else {
        let ix_off = (abi::dsp_ix_base_offset() + 3 * 2) as i32;
        let ix = ctx.load_u16(ix_off);
        ctx.emit_increase_ar(3, ix)
    };
    let new_316 = ctx.builder.ins().ireduce(types::I16, new_3);
    ctx.store_u16(new_316, ar_3_off);
}

#[inline(always)]
pub fn ext_ldax<const OP: u32, const SYSTEM: SystemId>(ctx: &mut TranslatorCtx, instr: GcDspExt) {
    let s = instr.d_2_2() as usize;
    let r = instr.r_3_3() as usize;

    let ar_s_off = (abi::dsp_ar_base_offset() + s * 2) as i32;
    let ar_3_off = (abi::dsp_ar_base_offset() + 3 * 2) as i32;
    let ar_s = ctx.load_u16(ar_s_off);
    let ar_3 = ctx.load_u16(ar_3_off);

    let s_bank = ctx.builder.ins().ushr_imm(ar_s, 10);
    let three_bank = ctx.builder.ins().ushr_imm(ar_3, 10);
    let same_bank_i8 = ctx
        .builder
        .ins()
        .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, s_bank, three_bank);

    let addr_s = ctx.builder.ins().uextend(types::I32, ar_s);
    let high_u32 = ctx.emit_read_dmem(addr_s);
    let high = ctx.builder.ins().ireduce(types::I16, high_u32);

    let low = {
        let same_block = ctx.builder.create_block();
        let diff_block = ctx.builder.create_block();
        let join_block = ctx.builder.create_block();
        ctx.builder.append_block_param(join_block, types::I16);
        ctx.builder.ins().brif(same_bank_i8, same_block, &[], diff_block, &[]);
        ctx.builder.switch_to_block(same_block);
        ctx.builder.seal_block(same_block);
        ctx.builder.ins().jump(join_block, &[high.into()]);
        ctx.builder.switch_to_block(diff_block);
        ctx.builder.seal_block(diff_block);
        let addr_3 = ctx.builder.ins().uextend(types::I32, ar_3);
        let lo_u32 = ctx.emit_read_dmem(addr_3);
        let lo = ctx.builder.ins().ireduce(types::I16, lo_u32);
        ctx.builder.ins().jump(join_block, &[lo.into()]);
        ctx.builder.switch_to_block(join_block);
        ctx.builder.seal_block(join_block);
        ctx.builder.block_params(join_block)[0]
    };

    let axh_off = (abi::dsp_axh_base_offset() + r * 2) as i32;
    let ax_off = (abi::dsp_ax_base_offset() + r * 2) as i32;
    ctx.store_u16(high, axh_off);
    ctx.store_u16(low, ax_off);

    let (s_inc, three_inc) = match OP {
        OP_EXT_LDAX => (true, true),
        OP_EXT_LDAXN => (false, true),
        OP_EXT_LDAXM => (true, false),
        OP_EXT_LDAXNM => (false, false),
        _ => unreachable!(),
    };

    let new_s = if s_inc {
        ctx.emit_inc_ar(s as u32)
    } else {
        let ix_off = (abi::dsp_ix_base_offset() + s * 2) as i32;
        let ix = ctx.load_u16(ix_off);
        ctx.emit_increase_ar(s as u32, ix)
    };
    let new_s16 = ctx.builder.ins().ireduce(types::I16, new_s);
    ctx.store_u16(new_s16, ar_s_off);

    let new_3 = if three_inc {
        ctx.emit_inc_ar(3)
    } else {
        let ix_off = (abi::dsp_ix_base_offset() + 3 * 2) as i32;
        let ix = ctx.load_u16(ix_off);
        ctx.emit_increase_ar(3, ix)
    };
    let new_316 = ctx.builder.ins().ireduce(types::I16, new_3);
    ctx.store_u16(new_316, ar_3_off);
}
