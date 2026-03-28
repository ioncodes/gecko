// Dequantize lookup table (64 entries, indexed by 6-bit unsigned scale)
// Scale 0-31: 1.0 / 2^i, Scale 32-63: 2^(64-i) (signed interpretation: -32..-1)
const DEQUANT_TABLE: [f32; 64] = {
    let mut table = [0.0f32; 64];
    let mut i = 0u32;
    while i < 32 {
        table[i as usize] = 1.0 / (1u64 << i) as f32;
        i += 1;
    }
    while i < 64 {
        table[i as usize] = (1u64 << (64 - i)) as f32;
        i += 1;
    }
    table
};

// Quantize lookup table (inverse of dequantize)
// Scale 0-31: 2^i, Scale 32-63: 1.0 / 2^(64-i)
const QUANT_TABLE: [f32; 64] = {
    let mut table = [0.0f32; 64];
    let mut i = 0u32;
    while i < 32 {
        table[i as usize] = (1u64 << i) as f32;
        i += 1;
    }
    while i < 64 {
        table[i as usize] = 1.0 / (1u64 << (64 - i)) as f32;
        i += 1;
    }
    table
};

fn gqr_ld_type(gqr: u32) -> u8 {
    ((gqr >> 16) & 0x7) as u8
}
fn gqr_ld_scale(gqr: u32) -> u8 {
    ((gqr >> 24) & 0x3f) as u8
}
fn gqr_st_type(gqr: u32) -> u8 {
    (gqr & 0x7) as u8
}
fn gqr_st_scale(gqr: u32) -> u8 {
    ((gqr >> 8) & 0x3f) as u8
}

fn quant_element_size(qtype: u8) -> u32 {
    match qtype {
        0 => 4,     // f32
        4 | 6 => 1, // u8 / s8
        5 | 7 => 2, // u16 / s16
        _ => 4,
    }
}

fn dequantize(ctx: &mut crate::gamecube::GameCube, addr: u32, ld_type: u8, ld_scale: u8) -> f64 {
    let scale = DEQUANT_TABLE[ld_scale as usize];
    match ld_type {
        0 => ctx.read_f32(addr),
        4 => (ctx.read_u8(addr) as f32 * scale) as f64,
        5 => (ctx.read_u16(addr) as f32 * scale) as f64,
        6 => (ctx.read_u8(addr) as i8 as f32 * scale) as f64,
        7 => (ctx.read_u16(addr) as i16 as f32 * scale) as f64,
        _ => 0.0,
    }
}

fn quantize(ctx: &mut crate::gamecube::GameCube, addr: u32, value: f64, st_type: u8, st_scale: u8) {
    let scale = QUANT_TABLE[st_scale as usize];
    match st_type {
        0 => ctx.write_f32(addr, value),
        4 => {
            let v = (value as f32 * scale).clamp(u8::MIN as f32, u8::MAX as f32) as u8;
            ctx.write_u8(addr, v);
        }
        5 => {
            let v = (value as f32 * scale).clamp(u16::MIN as f32, u16::MAX as f32) as u16;
            ctx.write_u16(addr, v);
        }
        6 => {
            let v = (value as f32 * scale).clamp(i8::MIN as f32, i8::MAX as f32) as i8;
            ctx.write_u8(addr, v as u8);
        }
        7 => {
            let v = (value as f32 * scale).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            ctx.write_u16(addr, v as u16);
        }
        _ => {}
    }
}

fn psq_load(ctx: &mut crate::gamecube::GameCube, fd: u8, addr: u32, w: bool, gqr: u32) {
    let ld_type = gqr_ld_type(gqr);
    let ld_scale = gqr_ld_scale(gqr);
    let ps0 = dequantize(ctx, addr, ld_type, ld_scale);
    let ps1 = if w {
        1.0
    } else {
        let elem_size = quant_element_size(ld_type);
        dequantize(ctx, addr.wrapping_add(elem_size), ld_type, ld_scale)
    };
    ctx.cpu.write_fpr(fd, ps0);
    ctx.cpu.write_ps1(fd, ps1);
}

fn psq_store(ctx: &mut crate::gamecube::GameCube, fs: u8, addr: u32, w: bool, gqr: u32) {
    let st_type = gqr_st_type(gqr);
    let st_scale = gqr_st_scale(gqr);
    let ps0 = ctx.cpu.read_fpr(fs);
    quantize(ctx, addr, ps0, st_type, st_scale);
    if !w {
        let ps1 = ctx.cpu.read_ps1(fs);
        let elem_size = quant_element_size(st_type);
        quantize(ctx, addr.wrapping_add(elem_size), ps1, st_type, st_scale);
    }
}

#[inline(always)]
pub fn store_load_psq<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    use crate::cpu::lut::*;

    match OP {
        // D-form loads
        OP_PSQ_L | OP_PSQ_LU => {
            let ea = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add_signed(instr.disp_psq());
            let gqr = ctx.cpu.spr.read_gqr(instr.psq_i());
            psq_load(ctx, instr.fd(), ea, instr.psq_w(), gqr);
            if OP == OP_PSQ_LU {
                ctx.cpu.write_gpr(instr.ra(), ea);
            }
        }
        // D-form stores
        OP_PSQ_ST | OP_PSQ_STU => {
            let ea = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add_signed(instr.disp_psq());
            let gqr = ctx.cpu.spr.read_gqr(instr.psq_i());
            psq_store(ctx, instr.fs(), ea, instr.psq_w(), gqr);
            if OP == OP_PSQ_STU {
                ctx.cpu.write_gpr(instr.ra(), ea);
            }
        }
        // X-form loads
        OP_PSQ_LX | OP_PSQ_LUX => {
            let ea = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let gqr = ctx.cpu.spr.read_gqr(instr.psq_ix());
            psq_load(ctx, instr.fd(), ea, instr.psq_wx(), gqr);
            if OP == OP_PSQ_LUX {
                ctx.cpu.write_gpr(instr.ra(), ea);
            }
        }
        // X-form stores
        OP_PSQ_STX | OP_PSQ_STUX => {
            let ea = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let gqr = ctx.cpu.spr.read_gqr(instr.psq_ix());
            psq_store(ctx, instr.fs(), ea, instr.psq_wx(), gqr);
            if OP == OP_PSQ_STUX {
                ctx.cpu.write_gpr(instr.ra(), ea);
            }
        }
        _ => unreachable!(),
    }
}
