use cranelift_codegen::ir::{self, InstBuilder};
use cranelift_frontend::FunctionBuilder;

use super::VtxKey;
use super::builder::{MEMFLAGS, MEMFLAGS_RO, MEMFLAGS_RO_MOVABLE, array_offset, offset, xf_byte_off};
use crate::flipper::gx::constants::*;
use crate::flipper::gx::regs::{AttributeType, ColorCount, ColorFormat, ComponentFormat, NrmCount, PosCount, TexCount};

pub struct AttrCtx<'a, 'f> {
    pub bd: &'a mut FunctionBuilder<'f>,
    pub xf_mem_ptr: ir::Value,
    pub arrays_ptr: ir::Value,
    pub data_ptr: ir::Value,
    pub out_ptr: ir::Value,
    pub pointer_ty: ir::Type,
    pub key: VtxKey,
}

pub fn emit_vertex(ctx: &mut AttrCtx) {
    let key = ctx.key;
    let vcd_lo = key.vcd_lo();
    let vcd_hi = key.vcd_hi();
    let vat_a = key.vat_a();
    let vat_b = key.vat_b();
    let vat_c = key.vat_c();

    let pos_mtx_idx = read_pos_mtx_idx(ctx, vcd_lo);
    let tex_mtx_idx = read_tex_mtx_indices(ctx, vcd_lo);

    let pos_xyz = decode_position(ctx, vcd_lo.position(), vat_a);
    store_vec3(ctx.bd, ctx.out_ptr, offset::POSITION, &pos_xyz);

    let nrm_xyz = decode_normal(ctx, vcd_lo.normal(), vat_a);

    let color0 = decode_color(ctx, vcd_lo.color0(), vat_a.clr0_fmt(), vat_a.clr0_cnt(), 2);
    ctx.bd.ins().store(MEMFLAGS, color0, ctx.out_ptr, offset::COLOR0);

    let color1 = decode_color(ctx, vcd_lo.color1(), vat_a.clr1_fmt(), vat_a.clr1_cnt(), 3);
    ctx.bd.ins().store(MEMFLAGS, color1, ctx.out_ptr, offset::COLOR1);

    let tex_attrs = [
        vcd_hi.tex0(),
        vcd_hi.tex1(),
        vcd_hi.tex2(),
        vcd_hi.tex3(),
        vcd_hi.tex4(),
        vcd_hi.tex5(),
        vcd_hi.tex6(),
        vcd_hi.tex7(),
    ];
    let tex_fmts = [
        vat_a.tex0_fmt(),
        vat_b.tex1_fmt(),
        vat_b.tex2_fmt(),
        vat_b.tex3_fmt(),
        vat_b.tex4_fmt(),
        vat_c.tex5_fmt(),
        vat_c.tex6_fmt(),
        vat_c.tex7_fmt(),
    ];
    let tex_shifts = [
        vat_a.tex0_shift(),
        vat_b.tex1_shift(),
        vat_b.tex2_shift(),
        vat_b.tex3_shift(),
        vat_c.tex4_shift(),
        vat_c.tex5_shift(),
        vat_c.tex6_shift(),
        vat_c.tex7_shift(),
    ];
    let tex_cnts = [
        vat_a.tex0_cnt(),
        vat_b.tex1_cnt(),
        vat_b.tex2_cnt(),
        vat_b.tex3_cnt(),
        vat_b.tex4_cnt(),
        vat_c.tex5_cnt(),
        vat_c.tex6_cnt(),
        vat_c.tex7_cnt(),
    ];

    let mut raw_st: [Option<[ir::Value; 2]>; 8] = [None; 8];
    for i in 0..8 {
        if !matches!(tex_attrs[i], AttributeType::None) {
            raw_st[i] = Some(decode_texcoord(
                ctx,
                tex_attrs[i],
                tex_fmts[i],
                tex_shifts[i],
                tex_cnts[i],
                4 + i,
            ));
        }
    }

    let pos_view = xf_transform_3x4(ctx, pos_mtx_idx, &pos_xyz);
    store_vec3(ctx.bd, ctx.out_ptr, offset::POS_VIEW, &pos_view);

    let nrm_view = transform_and_normalize_normal(ctx, pos_mtx_idx, &nrm_xyz);
    store_vec3(ctx.bd, ctx.out_ptr, offset::NORMAL, &nrm_view);

    self::emit_texgens(ctx, &pos_xyz, &nrm_xyz, &tex_mtx_idx, &raw_st);
}

fn read_pos_mtx_idx(ctx: &mut AttrCtx, vcd_lo: crate::flipper::gx::regs::VcdLo) -> ir::Value {
    if vcd_lo.pos_nrm_mtx_idx() {
        let raw = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ctx.data_ptr, 0);
        let masked = ctx.bd.ins().band_imm(raw, 0x3F);
        let v = ctx.bd.ins().uextend(ir::types::I32, masked);
        ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, 1);
        v
    } else {
        // xf_mem[XF_MATRIX_INDEX_A].pos_mtx_idx() -> bits 0..=5
        let cell = ctx.bd.ins().load(
            ir::types::I32,
            MEMFLAGS_RO_MOVABLE,
            ctx.xf_mem_ptr,
            xf_byte_off(XF_MATRIX_INDEX_A),
        );
        ctx.bd.ins().band_imm(cell, 0x3F)
    }
}

fn read_tex_mtx_indices(ctx: &mut AttrCtx, vcd_lo: crate::flipper::gx::regs::VcdLo) -> [ir::Value; 8] {
    let flags = [
        vcd_lo.tex0_mtx_idx(),
        vcd_lo.tex1_mtx_idx(),
        vcd_lo.tex2_mtx_idx(),
        vcd_lo.tex3_mtx_idx(),
        vcd_lo.tex4_mtx_idx(),
        vcd_lo.tex5_mtx_idx(),
        vcd_lo.tex6_mtx_idx(),
        vcd_lo.tex7_mtx_idx(),
    ];
    // Defaults are packed in XF_MATRIX_INDEX_A (tex0..tex3, 6 bits each
    // starting at bit 6) and XF_MATRIX_INDEX_B (tex4..tex7).
    let cell_a = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO_MOVABLE,
        ctx.xf_mem_ptr,
        xf_byte_off(XF_MATRIX_INDEX_A),
    );
    let cell_b = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO_MOVABLE,
        ctx.xf_mem_ptr,
        xf_byte_off(XF_MATRIX_INDEX_B),
    );

    std::array::from_fn(|i| {
        if flags[i] {
            let raw = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ctx.data_ptr, 0);
            let v = ctx.bd.ins().uextend(ir::types::I32, raw);
            ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, 1);
            v
        } else {
            // MATRIX_INDEX_A: tex0 at bit 6, +6 per channel (tex0..tex3).
            // MATRIX_INDEX_B: tex4 at bit 0, +6 per channel (tex4..tex7).
            let (cell, shift) = if i < 4 {
                (cell_a, 6 + (i as i64) * 6)
            } else {
                (cell_b, ((i - 4) as i64) * 6)
            };

            let shifted = if shift > 0 {
                ctx.bd.ins().ushr_imm(cell, shift)
            } else {
                cell
            };

            ctx.bd.ins().band_imm(shifted, 0x3F)
        }
    })
}

fn attr_ptr_and_advance(
    ctx: &mut AttrCtx,
    attr: AttributeType,
    array_idx: usize,
    direct_size: usize,
    extra_index_slots: i64,
) -> ir::Value {
    match attr {
        AttributeType::Direct => {
            let p = ctx.data_ptr;
            ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, direct_size as i64);
            p
        }
        AttributeType::Index8 => {
            let raw = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ctx.data_ptr, 0);
            let idx = ctx.bd.ins().uextend(ir::types::I32, raw);
            ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, 1 + extra_index_slots);
            indexed_addr(ctx, array_idx, idx)
        }
        AttributeType::Index16 => {
            let raw = ctx.bd.ins().load(ir::types::I16, MEMFLAGS_RO, ctx.data_ptr, 0);
            let raw = ctx.bd.ins().bswap(raw);
            let idx = ctx.bd.ins().uextend(ir::types::I32, raw);
            ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, 2 * (1 + extra_index_slots));
            indexed_addr(ctx, array_idx, idx)
        }
        AttributeType::None => ctx.data_ptr,
    }
}

fn indexed_addr(ctx: &mut AttrCtx, array_idx: usize, idx: ir::Value) -> ir::Value {
    let entry_off = (array_idx as i32) * array_offset::SIZE;
    let host_base = ctx.bd.ins().load(
        ctx.pointer_ty,
        MEMFLAGS_RO,
        ctx.arrays_ptr,
        entry_off + array_offset::HOST_BASE,
    );

    let stride = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO,
        ctx.arrays_ptr,
        entry_off + array_offset::STRIDE,
    );
    let prod = ctx.bd.ins().imul(idx, stride);
    let prod_p = ctx.bd.ins().uextend(ctx.pointer_ty, prod);

    ctx.bd.ins().iadd(host_base, prod_p)
}

fn decode_component_at(
    ctx: &mut AttrCtx,
    ptr: ir::Value,
    byte_off: i32,
    fmt: ComponentFormat,
    recip: f32,
) -> ir::Value {
    match fmt {
        ComponentFormat::F32 => {
            let raw = ctx.bd.ins().load(ir::types::I32, MEMFLAGS_RO, ptr, byte_off);
            let raw = ctx.bd.ins().bswap(raw);
            ctx.bd.ins().bitcast(ir::types::F32, ir::MemFlagsData::new(), raw)
        }
        ComponentFormat::U16 | ComponentFormat::S16 => {
            let raw = ctx.bd.ins().load(ir::types::I16, MEMFLAGS_RO, ptr, byte_off);
            let raw = ctx.bd.ins().bswap(raw);
            let signed = matches!(fmt, ComponentFormat::S16);
            let ext = if signed {
                ctx.bd.ins().sextend(ir::types::I32, raw)
            } else {
                ctx.bd.ins().uextend(ir::types::I32, raw)
            };
            let f = if signed {
                ctx.bd.ins().fcvt_from_sint(ir::types::F32, ext)
            } else {
                ctx.bd.ins().fcvt_from_uint(ir::types::F32, ext)
            };
            let s = ctx.bd.ins().f32const(recip);
            ctx.bd.ins().fmul(f, s)
        }
        ComponentFormat::U8 | ComponentFormat::S8 => {
            let raw = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ptr, byte_off);
            let signed = matches!(fmt, ComponentFormat::S8);
            let ext = if signed {
                ctx.bd.ins().sextend(ir::types::I32, raw)
            } else {
                ctx.bd.ins().uextend(ir::types::I32, raw)
            };
            let f = if signed {
                ctx.bd.ins().fcvt_from_sint(ir::types::F32, ext)
            } else {
                ctx.bd.ins().fcvt_from_uint(ir::types::F32, ext)
            };
            let s = ctx.bd.ins().f32const(recip);
            ctx.bd.ins().fmul(f, s)
        }
    }
}

const LITTLE: ir::MemFlagsData = ir::MemFlagsData::new().with_endianness(ir::Endianness::Little);

const ZERO_LANE: u8 = 0xFF;

const SWZ_S16_VEC2: [u8; 16] = [
    ZERO_LANE, ZERO_LANE, 1, 0, ZERO_LANE, ZERO_LANE, 3, 2, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
    ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_S16_VEC3: [u8; 16] = [
    ZERO_LANE, ZERO_LANE, 1, 0, ZERO_LANE, ZERO_LANE, 3, 2, ZERO_LANE, ZERO_LANE, 5, 4, ZERO_LANE, ZERO_LANE,
    ZERO_LANE, ZERO_LANE,
];
const SWZ_U16_VEC2: [u8; 16] = [
    1, 0, ZERO_LANE, ZERO_LANE, 3, 2, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
    ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_U16_VEC3: [u8; 16] = [
    1, 0, ZERO_LANE, ZERO_LANE, 3, 2, ZERO_LANE, ZERO_LANE, 5, 4, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
    ZERO_LANE, ZERO_LANE,
];
const SWZ_S8_VEC2: [u8; 16] = [
    ZERO_LANE, ZERO_LANE, ZERO_LANE, 0, ZERO_LANE, ZERO_LANE, ZERO_LANE, 1, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
    ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_S8_VEC3: [u8; 16] = [
    ZERO_LANE, ZERO_LANE, ZERO_LANE, 0, ZERO_LANE, ZERO_LANE, ZERO_LANE, 1, ZERO_LANE, ZERO_LANE, ZERO_LANE, 2,
    ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_U8_VEC2: [u8; 16] = [
    0, ZERO_LANE, ZERO_LANE, ZERO_LANE, 1, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
    ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_U8_VEC3: [u8; 16] = [
    0, ZERO_LANE, ZERO_LANE, ZERO_LANE, 1, ZERO_LANE, ZERO_LANE, ZERO_LANE, 2, ZERO_LANE, ZERO_LANE, ZERO_LANE,
    ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_F32_VEC2: [u8; 16] = [
    3, 2, 1, 0, 7, 6, 5, 4, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_F32_VEC3: [u8; 16] = [
    3, 2, 1, 0, 7, 6, 5, 4, 11, 10, 9, 8, ZERO_LANE, ZERO_LANE, ZERO_LANE, ZERO_LANE,
];

fn swizzle_const(ctx: &mut AttrCtx, mask: &[u8; 16]) -> ir::Value {
    let handle = ctx.bd.func.dfg.constants.insert(ir::ConstantData::from(&mask[..]));
    ctx.bd.ins().vconst(ir::types::I8X16, handle)
}

fn extract3(ctx: &mut AttrCtx, v: ir::Value) -> [ir::Value; 3] {
    let x = ctx.bd.ins().extractlane(v, 0);
    let y = ctx.bd.ins().extractlane(v, 1);
    let z = ctx.bd.ins().extractlane(v, 2);

    [x, y, z]
}

fn decode_vec_simd(ctx: &mut AttrCtx, ptr: ir::Value, fmt: ComponentFormat, triplet: bool, recip: f32) -> ir::Value {
    if matches!(fmt, ComponentFormat::F32) {
        // swappedy swap swap swap
        let mask = if triplet { &SWZ_F32_VEC3 } else { &SWZ_F32_VEC2 };
        let lanes = self::shuffle_to_i32x4(ctx, ptr, mask);

        return ctx.bd.ins().bitcast(ir::types::F32X4, LITTLE, lanes);
    }

    let signed = matches!(fmt, ComponentFormat::S8 | ComponentFormat::S16);
    let wide16 = matches!(fmt, ComponentFormat::U16 | ComponentFormat::S16);

    let mask = match (wide16, signed, triplet) {
        (true, true, true) => &SWZ_S16_VEC3,
        (true, true, false) => &SWZ_S16_VEC2,
        (true, false, true) => &SWZ_U16_VEC3,
        (true, false, false) => &SWZ_U16_VEC2,
        (false, true, true) => &SWZ_S8_VEC3,
        (false, true, false) => &SWZ_S8_VEC2,
        (false, false, true) => &SWZ_U8_VEC3,
        (false, false, false) => &SWZ_U8_VEC2,
    };

    let lanes = self::shuffle_to_i32x4(ctx, ptr, mask);

    let floats = if signed {
        let bits = if wide16 { 16 } else { 8 };
        let ext = ctx.bd.ins().sshr_imm(lanes, 32 - bits);
        ctx.bd.ins().fcvt_from_sint(ir::types::F32X4, ext)
    } else {
        ctx.bd.ins().fcvt_from_uint(ir::types::F32X4, lanes)
    };

    let scale = self::splat_f32x4(ctx, recip);
    ctx.bd.ins().fmul(floats, scale)
}

fn decode_position(ctx: &mut AttrCtx, attr: AttributeType, vat_a: crate::flipper::gx::regs::VatA) -> [ir::Value; 3] {
    if matches!(attr, AttributeType::None) {
        let z = ctx.bd.ins().f32const(0.0);
        return [z, z, z];
    }

    let triplet = matches!(vat_a.pos_cnt(), PosCount::Xyz);
    let fmt = vat_a.pos_fmt();
    let direct = vat_a.pos_data_size();
    let recip = 1.0f32 / ((1u32 << vat_a.pos_shift()) as f32);

    let ptr = attr_ptr_and_advance(ctx, attr, ARRAY_POS, direct, 0);

    let vector = self::decode_vec_simd(ctx, ptr, fmt, triplet, recip);

    self::extract3(ctx, vector)
}

fn decode_normal(ctx: &mut AttrCtx, attr: AttributeType, vat_a: crate::flipper::gx::regs::VatA) -> [ir::Value; 3] {
    if matches!(attr, AttributeType::None) {
        let z = ctx.bd.ins().f32const(0.0);
        let one = ctx.bd.ins().f32const(1.0);
        return [z, z, one];
    }
    let fmt = vat_a.nrm_fmt();

    let direct = vat_a.nrm_data_size();
    let recip = match fmt {
        ComponentFormat::U8 | ComponentFormat::S8 => 1.0f32 / 64.0,
        ComponentFormat::U16 | ComponentFormat::S16 => 1.0f32 / 16384.0,
        ComponentFormat::F32 => 1.0f32,
    };

    let extra_index_slots = if vat_a.nrm_index3() && matches!(vat_a.nrm_cnt(), NrmCount::Nbt) {
        2
    } else {
        0
    };
    let ptr = attr_ptr_and_advance(ctx, attr, ARRAY_NRM, direct, extra_index_slots);

    let vector = self::decode_vec_simd(ctx, ptr, fmt, true, recip);

    self::extract3(ctx, vector)
}

const SWZ_COLOR_BYTES: [u8; 16] = [
    0, ZERO_LANE, ZERO_LANE, ZERO_LANE, 1, ZERO_LANE, ZERO_LANE, ZERO_LANE, 2, ZERO_LANE, ZERO_LANE, ZERO_LANE, 3,
    ZERO_LANE, ZERO_LANE, ZERO_LANE,
];
const SWZ_COLOR_U16: [u8; 16] = [
    1, 0, ZERO_LANE, ZERO_LANE, 1, 0, ZERO_LANE, ZERO_LANE, 1, 0, ZERO_LANE, ZERO_LANE, 1, 0, ZERO_LANE, ZERO_LANE,
];
const SWZ_COLOR_U24: [u8; 16] = [
    2, 1, 0, ZERO_LANE, 2, 1, 0, ZERO_LANE, 2, 1, 0, ZERO_LANE, 2, 1, 0, ZERO_LANE,
];

fn vconst_i32x4(ctx: &mut AttrCtx, lanes: [u32; 4]) -> ir::Value {
    let mut bytes = [0u8; 16];
    for (i, v) in lanes.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let handle = ctx.bd.func.dfg.constants.insert(ir::ConstantData::from(&bytes[..]));
    ctx.bd.ins().vconst(ir::types::I32X4, handle)
}

fn vconst_f32x4(ctx: &mut AttrCtx, lanes: [f32; 4]) -> ir::Value {
    let mut bytes = [0u8; 16];
    for (i, v) in lanes.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_bits().to_le_bytes());
    }
    let handle = ctx.bd.func.dfg.constants.insert(ir::ConstantData::from(&bytes[..]));
    ctx.bd.ins().vconst(ir::types::F32X4, handle)
}

fn splat_f32x4(ctx: &mut AttrCtx, v: f32) -> ir::Value {
    let c = ctx.bd.ins().f32const(v);
    ctx.bd.ins().splat(ir::types::F32X4, c)
}

fn shuffle_to_i32x4(ctx: &mut AttrCtx, ptr: ir::Value, mask: &[u8; 16]) -> ir::Value {
    let bytes = ctx.bd.ins().load(ir::types::I8X16, MEMFLAGS_RO, ptr, 0);
    let mask = self::swizzle_const(ctx, mask);
    let shuffled = ctx.bd.ins().swizzle(bytes, mask);
    ctx.bd.ins().bitcast(ir::types::I32X4, LITTLE, shuffled)
}

fn unpack_fields(ctx: &mut AttrCtx, lanes: ir::Value, mul: [u32; 4], shift: i64, and: [u32; 4]) -> ir::Value {
    let mul = self::vconst_i32x4(ctx, mul);
    let v = ctx.bd.ins().imul(lanes, mul);
    let v = ctx.bd.ins().ushr_imm(v, shift);
    let and = self::vconst_i32x4(ctx, and);
    ctx.bd.ins().band(v, and)
}

fn decode_color(
    ctx: &mut AttrCtx,
    attr: AttributeType,
    fmt: ColorFormat,
    cnt: ColorCount,
    array_idx: usize,
) -> ir::Value {
    if matches!(attr, AttributeType::None) {
        return self::splat_f32x4(ctx, 1.0);
    }

    let direct = fmt.data_size(cnt);
    let ptr = attr_ptr_and_advance(ctx, attr, array_idx, direct, 0);

    self::decode_color_bytes(ctx, ptr, fmt, cnt)
}

fn decode_color_bytes(ctx: &mut AttrCtx, ptr: ir::Value, fmt: ColorFormat, cnt: ColorCount) -> ir::Value {
    let has_alpha = matches!(cnt, ColorCount::Rgba);

    let (lanes, div, force_alpha_one) = match fmt {
        ColorFormat::Rgb565 => {
            let l = self::shuffle_to_i32x4(ctx, ptr, &SWZ_COLOR_U16);
            let l = self::unpack_fields(ctx, l, [1, 64, 2048, 0], 11, [0x1F, 0x3F, 0x1F, 0]);
            let d = self::vconst_f32x4(ctx, [31.0, 63.0, 31.0, 1.0]);
            (l, d, true)
        }
        ColorFormat::Rgba4 => {
            let l = self::shuffle_to_i32x4(ctx, ptr, &SWZ_COLOR_U16);
            let l = self::unpack_fields(ctx, l, [1, 16, 256, 4096], 12, [0xF, 0xF, 0xF, 0xF]);
            (l, self::splat_f32x4(ctx, 15.0), !has_alpha)
        }
        ColorFormat::Rgba6 => {
            let l = self::shuffle_to_i32x4(ctx, ptr, &SWZ_COLOR_U24);
            let l = self::unpack_fields(ctx, l, [1, 64, 4096, 262144], 18, [0x3F, 0x3F, 0x3F, 0x3F]);
            (l, self::splat_f32x4(ctx, 63.0), !has_alpha)
        }
        ColorFormat::Rgb8 | ColorFormat::Rgbx8 => {
            let l = self::shuffle_to_i32x4(ctx, ptr, &SWZ_COLOR_BYTES);
            (l, self::splat_f32x4(ctx, 255.0), true)
        }
        ColorFormat::Rgba8 => {
            let l = self::shuffle_to_i32x4(ctx, ptr, &SWZ_COLOR_BYTES);
            (l, self::splat_f32x4(ctx, 255.0), !has_alpha)
        }
    };

    let f = ctx.bd.ins().fcvt_from_uint(ir::types::F32X4, lanes);
    let rgba = ctx.bd.ins().fdiv(f, div);

    if force_alpha_one {
        let one = ctx.bd.ins().f32const(1.0);
        return ctx.bd.ins().insertlane(rgba, one, 3);
    }

    rgba
}

fn decode_texcoord(
    ctx: &mut AttrCtx,
    attr: AttributeType,
    fmt: ComponentFormat,
    shift: u8,
    cnt: TexCount,
    array_idx: usize,
) -> [ir::Value; 2] {
    let count = cnt.components();
    let direct = count * fmt.size();
    let recip = 1.0f32 / ((1u32 << shift) as f32);
    let ptr = attr_ptr_and_advance(ctx, attr, array_idx, direct, 0);

    if count == 2 {
        let vector = self::decode_vec_simd(ctx, ptr, fmt, false, recip);
        let s = ctx.bd.ins().extractlane(vector, 0);
        let t = ctx.bd.ins().extractlane(vector, 1);
        return [s, t];
    }

    let s = decode_component_at(ctx, ptr, 0, fmt, recip);
    let t = ctx.bd.ins().f32const(0.0);

    [s, t]
}

fn xf_transform_3x4(ctx: &mut AttrCtx, pos_mtx_idx: ir::Value, pos: &[ir::Value; 3]) -> [ir::Value; 3] {
    let base_addr = self::pos_mtx_base_addr(ctx, pos_mtx_idx);
    let m = self::load_matrix12(ctx, base_addr, MEMFLAGS_RO_MOVABLE);

    let r0 = self::dot3_affine(ctx, [m[0], m[1], m[2], m[3]], *pos);
    let r1 = self::dot3_affine(ctx, [m[4], m[5], m[6], m[7]], *pos);
    let r2 = self::dot3_affine(ctx, [m[8], m[9], m[10], m[11]], *pos);
    [r0, r1, r2]
}

fn transform_and_normalize_normal(ctx: &mut AttrCtx, pos_mtx_idx: ir::Value, nrm: &[ir::Value; 3]) -> [ir::Value; 3] {
    // nrm_mtx_base = XF_NRM_MTX_BASE + (pos_mtx_idx & 31) * 3 cells
    let masked = ctx.bd.ins().band_imm(pos_mtx_idx, 31);
    let cell_off = ctx.bd.ins().imul_imm(masked, 3 * 4);
    let cell_off = ctx.bd.ins().uextend(ctx.pointer_ty, cell_off);
    let base = ctx.bd.ins().iadd(ctx.xf_mem_ptr, cell_off);

    let nm = std::array::from_fn::<ir::Value, 9, _>(|i| {
        ctx.bd.ins().load(
            ir::types::F32,
            MEMFLAGS_RO_MOVABLE,
            base,
            (XF_NRM_MTX_BASE * 4 + i * 4) as i32,
        )
    });

    let row = |i: usize, ctx: &mut AttrCtx| -> ir::Value {
        let m0 = nm[i * 3 + 0];
        let m1 = nm[i * 3 + 1];
        let m2 = nm[i * 3 + 2];
        let p0 = ctx.bd.ins().fmul(m0, nrm[0]);
        let p1 = ctx.bd.ins().fmul(m1, nrm[1]);
        let p2 = ctx.bd.ins().fmul(m2, nrm[2]);
        let s = ctx.bd.ins().fadd(p0, p1);
        ctx.bd.ins().fadd(s, p2)
    };

    let nx = row(0, ctx);
    let ny = row(1, ctx);
    let nz = row(2, ctx);

    // length = sqrt(nx*nx + ny*ny + nz*nz)
    let nx2 = ctx.bd.ins().fmul(nx, nx);
    let ny2 = ctx.bd.ins().fmul(ny, ny);
    let nz2 = ctx.bd.ins().fmul(nz, nz);
    let s = ctx.bd.ins().fadd(nx2, ny2);
    let len_sq = ctx.bd.ins().fadd(s, nz2);
    let len = ctx.bd.ins().sqrt(len_sq);

    let zero = ctx.bd.ins().f32const(0.0);
    let eps = ctx.bd.ins().f32const(1e-10);
    let small = ctx.bd.ins().fcmp(ir::condcodes::FloatCC::LessThan, len, eps);

    let nx_n = ctx.bd.ins().fdiv(nx, len);
    let ny_n = ctx.bd.ins().fdiv(ny, len);
    let nz_n = ctx.bd.ins().fdiv(nz, len);

    let xs = ctx.bd.ins().select(small, zero, nx_n);
    let ys = ctx.bd.ins().select(small, zero, ny_n);
    let zs = ctx.bd.ins().select(small, zero, nz_n);
    [xs, ys, zs]
}

fn emit_texgens(
    ctx: &mut AttrCtx,
    pos: &[ir::Value; 3],
    nrm: &[ir::Value; 3],
    tex_mtx_idx: &[ir::Value; 8],
    raw_st: &[Option<[ir::Value; 2]>; 8],
) {
    let num_texgens = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO_MOVABLE,
        ctx.xf_mem_ptr,
        xf_byte_off(XF_NUM_TEXGENS),
    );

    let dual_cell = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO_MOVABLE,
        ctx.xf_mem_ptr,
        xf_byte_off(XF_DUAL_TEX_ENABLE),
    );
    let dual_en = ctx.bd.ins().icmp_imm(ir::condcodes::IntCC::NotEqual, dual_cell, 0);

    for tg in 0..8 {
        let active = ctx.bd.create_block();
        let passthru = ctx.bd.create_block();
        let done = ctx.bd.create_block();
        ctx.bd.append_block_param(done, ir::types::F32);
        ctx.bd.append_block_param(done, ir::types::F32);
        ctx.bd.append_block_param(done, ir::types::F32);

        // active when tg < num_texgens; the 8-slot unroll enforces .min(8)
        let tg_const = ctx.bd.ins().iconst(ir::types::I32, tg as i64);
        let cond = ctx
            .bd
            .ins()
            .icmp(ir::condcodes::IntCC::UnsignedLessThan, tg_const, num_texgens);
        ctx.bd.ins().brif(cond, active, &[], passthru, &[]);

        ctx.bd.switch_to_block(active);
        ctx.bd.seal_block(active);
        let a = self::emit_texgen_active(ctx, tg, pos, nrm, tex_mtx_idx, raw_st, dual_en);
        ctx.bd.ins().jump(
            done,
            &[
                ir::BlockArg::Value(a[0]),
                ir::BlockArg::Value(a[1]),
                ir::BlockArg::Value(a[2]),
            ],
        );

        // passthrough: present ? [s, t, 1] : [0, 0, 1]
        ctx.bd.switch_to_block(passthru);
        ctx.bd.seal_block(passthru);
        let one = ctx.bd.ins().f32const(1.0);
        let (ps, pt) = match raw_st[tg] {
            Some(st) => (st[0], st[1]),
            None => {
                let z = ctx.bd.ins().f32const(0.0);
                (z, z)
            }
        };
        ctx.bd.ins().jump(
            done,
            &[
                ir::BlockArg::Value(ps),
                ir::BlockArg::Value(pt),
                ir::BlockArg::Value(one),
            ],
        );

        ctx.bd.switch_to_block(done);
        ctx.bd.seal_block(done);
        let d = ctx.bd.block_params(done);
        let out = [d[0], d[1], d[2]];
        store_vec3(ctx.bd, ctx.out_ptr, offset::TEXCOORDS + (tg as i32) * 12, &out);
    }
}

fn emit_texgen_active(
    ctx: &mut AttrCtx,
    tg: usize,
    pos: &[ir::Value; 3],
    nrm: &[ir::Value; 3],
    tex_mtx_idx: &[ir::Value; 8],
    raw_st: &[Option<[ir::Value; 2]>; 8],
    dual_en: ir::Value,
) -> [ir::Value; 3] {
    let one = ctx.bd.ins().f32const(1.0);

    let tg_cell = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO_MOVABLE,
        ctx.xf_mem_ptr,
        xf_byte_off(XF_TEXGEN_BASE + tg),
    );

    let src_shift = ctx.bd.ins().ushr_imm(tg_cell, 7);
    let src_row = ctx.bd.ins().band_imm(src_shift, 0x1F);
    let src = self::texgen_source(ctx, src_row, pos, nrm, raw_st);

    // input = input_form == Abc1 ? [s0, s1, s2, 1] : [s0, s1, 1, 1]
    let is_abc1 = self::flag(ctx, tg_cell, 2);
    let input2 = ctx.bd.ins().select(is_abc1, src[2], one);
    let input = [src[0], src[1], input2, one];

    let base_addr = self::pos_mtx_base_addr(ctx, tex_mtx_idx[tg]);
    let m = self::load_matrix12(ctx, base_addr, MEMFLAGS_RO_MOVABLE);

    let s = self::dot4(ctx, [m[0], m[1], m[2], m[3]], input);
    let t = self::dot4(ctx, [m[4], m[5], m[6], m[7]], input);

    // projection St -> q = 1.0, Stq -> q = row2 . input
    let is_stq = self::flag(ctx, tg_cell, 1);
    let q_stq = self::dot4(ctx, [m[8], m[9], m[10], m[11]], input);
    let q = ctx.bd.ins().select(is_stq, q_stq, one);

    let (s, t, q) = self::emit_dual(ctx, tg, s, t, q, dual_en);

    self::emit_q_clamp(ctx, s, t, q)
}

// Pick the source row exactly as TexGenSrc::from_raw + select_texgen_source:
// 0->pos, 1->nrm, 2..=4->[0,0,1], 5..=12->tex[k-5], 13..=31->tex7.
fn texgen_source(
    ctx: &mut AttrCtx,
    src_row: ir::Value,
    pos: &[ir::Value; 3],
    nrm: &[ir::Value; 3],
    raw_st: &[Option<[ir::Value; 2]>; 8],
) -> [ir::Value; 3] {
    use cranelift_frontend::Switch;

    let zero = ctx.bd.ins().f32const(0.0);
    let one = ctx.bd.ins().f32const(1.0);

    let merge = ctx.bd.create_block();
    ctx.bd.append_block_param(merge, ir::types::F32);
    ctx.bd.append_block_param(merge, ir::types::F32);
    ctx.bd.append_block_param(merge, ir::types::F32);

    let pos_blk = ctx.bd.create_block();
    let nrm_blk = ctx.bd.create_block();
    let dead_blk = ctx.bd.create_block();
    let tex_blk: [ir::Block; 8] = std::array::from_fn(|k| {
        if raw_st[k].is_some() {
            ctx.bd.create_block()
        } else {
            dead_blk
        }
    });

    let mut switch = Switch::new();
    switch.set_entry(0, pos_blk);
    switch.set_entry(1, nrm_blk);
    switch.set_entry(2, dead_blk);
    switch.set_entry(3, dead_blk);
    switch.set_entry(4, dead_blk);
    for k in 0..8 {
        switch.set_entry((5 + k) as u128, tex_blk[k]);
    }
    // src_row >= 13 aliases to tex7 (TexGenSrc::from_raw last-variant fallback).
    switch.emit(ctx.bd, src_row, tex_blk[7]);

    let jump3 = |ctx: &mut AttrCtx, blk: ir::Block, v: [ir::Value; 3]| {
        ctx.bd.ins().jump(
            blk,
            &[
                ir::BlockArg::Value(v[0]),
                ir::BlockArg::Value(v[1]),
                ir::BlockArg::Value(v[2]),
            ],
        );
    };

    ctx.bd.switch_to_block(pos_blk);
    ctx.bd.seal_block(pos_blk);
    jump3(ctx, merge, *pos);

    ctx.bd.switch_to_block(nrm_blk);
    ctx.bd.seal_block(nrm_blk);
    jump3(ctx, merge, *nrm);

    ctx.bd.switch_to_block(dead_blk);
    ctx.bd.seal_block(dead_blk);
    jump3(ctx, merge, [zero, zero, one]);

    for k in 0..8 {
        if let Some(st) = raw_st[k] {
            ctx.bd.switch_to_block(tex_blk[k]);
            ctx.bd.seal_block(tex_blk[k]);
            jump3(ctx, merge, [st[0], st[1], one]);
        }
    }

    ctx.bd.switch_to_block(merge);
    ctx.bd.seal_block(merge);
    let p = ctx.bd.block_params(merge);
    [p[0], p[1], p[2]]
}

// Test a single config bit: (word >> shift) & 1 != 0.
fn flag(ctx: &mut AttrCtx, word: ir::Value, shift: i64) -> ir::Value {
    let shifted = ctx.bd.ins().ushr_imm(word, shift);
    let bit = ctx.bd.ins().band_imm(shifted, 1);
    ctx.bd.ins().icmp_imm(ir::condcodes::IntCC::NotEqual, bit, 0)
}

// Base address of a 3x4 matrix in xf_mem, indexed by a matrix index (cells of
// XF_POS_MTX_STRIDE each). Shared by the position transform and texgen.
fn pos_mtx_base_addr(ctx: &mut AttrCtx, idx: ir::Value) -> ir::Value {
    let off = ctx.bd.ins().imul_imm(idx, (XF_POS_MTX_STRIDE * 4) as i64);
    let off = ctx.bd.ins().uextend(ctx.pointer_ty, off);
    ctx.bd.ins().iadd(ctx.xf_mem_ptr, off)
}

// Load the 12 f32 cells of a 3x4 matrix.
fn load_matrix12(ctx: &mut AttrCtx, base_addr: ir::Value, flags: ir::MemFlagsData) -> [ir::Value; 12] {
    std::array::from_fn(|i| ctx.bd.ins().load(ir::types::F32, flags, base_addr, (i * 4) as i32))
}

// (m0*v0 + m1*v1) + m2*v2 + m3*v3, no fma.
fn dot4(ctx: &mut AttrCtx, m: [ir::Value; 4], v: [ir::Value; 4]) -> ir::Value {
    let p0 = ctx.bd.ins().fmul(m[0], v[0]);
    let p1 = ctx.bd.ins().fmul(m[1], v[1]);
    let p2 = ctx.bd.ins().fmul(m[2], v[2]);
    let p3 = ctx.bd.ins().fmul(m[3], v[3]);
    let a = ctx.bd.ins().fadd(p0, p1);
    let b = ctx.bd.ins().fadd(a, p2);
    ctx.bd.ins().fadd(b, p3)
}

// (m0*v0 + m1*v1) + m2*v2 + m3: the affine row form, translate in m3.
fn dot3_affine(ctx: &mut AttrCtx, m: [ir::Value; 4], v: [ir::Value; 3]) -> ir::Value {
    let p0 = ctx.bd.ins().fmul(m[0], v[0]);
    let p1 = ctx.bd.ins().fmul(m[1], v[1]);
    let p2 = ctx.bd.ins().fmul(m[2], v[2]);
    let a = ctx.bd.ins().fadd(p0, p1);
    let b = ctx.bd.ins().fadd(a, p2);
    ctx.bd.ins().fadd(b, m[3])
}

fn emit_dual(
    ctx: &mut AttrCtx,
    tg: usize,
    s: ir::Value,
    t: ir::Value,
    q: ir::Value,
    dual_en: ir::Value,
) -> (ir::Value, ir::Value, ir::Value) {
    let dual_on = ctx.bd.create_block();
    let dual_done = ctx.bd.create_block();
    ctx.bd.append_block_param(dual_done, ir::types::F32);
    ctx.bd.append_block_param(dual_done, ir::types::F32);
    ctx.bd.append_block_param(dual_done, ir::types::F32);

    ctx.bd.ins().brif(
        dual_en,
        dual_on,
        &[],
        dual_done,
        &[ir::BlockArg::Value(s), ir::BlockArg::Value(t), ir::BlockArg::Value(q)],
    );

    ctx.bd.switch_to_block(dual_on);
    ctx.bd.seal_block(dual_on);

    let one = ctx.bd.ins().f32const(1.0);
    let eps = ctx.bd.ins().f32const(f32::EPSILON);

    let dt_cell = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO,
        ctx.xf_mem_ptr,
        xf_byte_off(XF_DUALTEX_BASE + tg),
    );
    let norm = self::flag(ctx, dt_cell, 6);
    let post_idx = ctx.bd.ins().band_imm(dt_cell, 0x3F);

    // inv_q = q.abs() > EPSILON ? 1/q : 1.0 (1/q always computed; fdiv cannot trap)
    let absq = ctx.bd.ins().fabs(q);
    let big = ctx.bd.ins().fcmp(ir::condcodes::FloatCC::GreaterThan, absq, eps);
    let recip = ctx.bd.ins().fdiv(one, q);
    let inv_q = ctx.bd.ins().select(big, recip, one);

    let s_n = ctx.bd.ins().fmul(s, inv_q);
    let t_n = ctx.bd.ins().fmul(t, inv_q);
    let ns = ctx.bd.ins().select(norm, s_n, s);
    let nt = ctx.bd.ins().select(norm, t_n, t);
    let nq = ctx.bd.ins().select(norm, inv_q, q);

    // post matrix at base = (XF_POST_MTX_BASE + post_idx * 4) cells, in bytes
    let post_bytes = ctx.bd.ins().imul_imm(post_idx, 16);
    let post_bytes = ctx.bd.ins().iadd_imm(post_bytes, (XF_POST_MTX_BASE * 4) as i64);
    let post_bytes = ctx.bd.ins().uextend(ctx.pointer_ty, post_bytes);
    let post_addr = ctx.bd.ins().iadd(ctx.xf_mem_ptr, post_bytes);
    let pm = self::load_matrix12(ctx, post_addr, MEMFLAGS_RO);

    let nsv = [ns, nt, nq];
    let ps = self::dot3_affine(ctx, [pm[0], pm[1], pm[2], pm[3]], nsv);
    let pt = self::dot3_affine(ctx, [pm[4], pm[5], pm[6], pm[7]], nsv);
    let pq = self::dot3_affine(ctx, [pm[8], pm[9], pm[10], pm[11]], nsv);

    ctx.bd.ins().jump(
        dual_done,
        &[
            ir::BlockArg::Value(ps),
            ir::BlockArg::Value(pt),
            ir::BlockArg::Value(pq),
        ],
    );

    ctx.bd.switch_to_block(dual_done);
    ctx.bd.seal_block(dual_done);
    let d = ctx.bd.block_params(dual_done);
    (d[0], d[1], d[2])
}

// q ~= 0 special case: clamp(xy/2, -1, 1) with q forced to 0, else pass (s,t,q).
fn emit_q_clamp(ctx: &mut AttrCtx, s: ir::Value, t: ir::Value, q: ir::Value) -> [ir::Value; 3] {
    use ir::condcodes::FloatCC;

    let zero = ctx.bd.ins().f32const(0.0);
    let two = ctx.bd.ins().f32const(2.0);
    let eps = ctx.bd.ins().f32const(f32::EPSILON);

    let absq = ctx.bd.ins().fabs(q);
    let is_zero = ctx.bd.ins().fcmp(FloatCC::LessThan, absq, eps);

    let s_half = ctx.bd.ins().fdiv(s, two);
    let t_half = ctx.bd.ins().fdiv(t, two);
    let s_c = self::clamp_unit(ctx, s_half);
    let t_c = self::clamp_unit(ctx, t_half);

    let out_s = ctx.bd.ins().select(is_zero, s_c, s);
    let out_t = ctx.bd.ins().select(is_zero, t_c, t);
    let out_q = ctx.bd.ins().select(is_zero, zero, q);
    [out_s, out_t, out_q]
}

// Motherucker.
fn clamp_unit(ctx: &mut AttrCtx, x: ir::Value) -> ir::Value {
    use ir::condcodes::FloatCC;

    let neg1 = ctx.bd.ins().f32const(-1.0);
    let pos1 = ctx.bd.ins().f32const(1.0);

    let lt = ctx.bd.ins().fcmp(FloatCC::LessThan, x, neg1);
    let x1 = ctx.bd.ins().select(lt, neg1, x);
    let gt = ctx.bd.ins().fcmp(FloatCC::GreaterThan, x1, pos1);
    ctx.bd.ins().select(gt, pos1, x1)
}

#[inline(always)]
fn store_vec3(bd: &mut FunctionBuilder, base: ir::Value, off: i32, v: &[ir::Value; 3]) {
    bd.ins().store(MEMFLAGS, v[0], base, off);
    bd.ins().store(MEMFLAGS, v[1], base, off + 4);
    bd.ins().store(MEMFLAGS, v[2], base, off + 8);
}
