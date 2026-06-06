use cranelift_codegen::ir::{self, InstBuilder, StackSlotData, StackSlotKind};
use cranelift_codegen::isa::TargetIsa;
use cranelift_frontend::FunctionBuilder;

use super::VtxKey;
use super::builder::{MEMFLAGS, MEMFLAGS_RO, array_offset, offset, xf_byte_off};
use crate::flipper::gx::constants::*;
use crate::flipper::gx::regs::{AttributeType, ColorCount, ColorFormat, ComponentFormat, PosCount, TexCount};

pub struct AttrCtx<'a, 'f> {
    pub bd: &'a mut FunctionBuilder<'f>,
    pub isa: &'a dyn TargetIsa,
    pub gp_ptr: ir::Value,
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

    let raw_pos_slot = ctx
        .bd
        .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 12, 2));
    let raw_nrm_slot = ctx
        .bd
        .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 12, 2));
    let raw_tex_slot = ctx
        .bd
        .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 64, 2));
    let tex_mtx_slot = ctx
        .bd
        .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 0));

    let pos_mtx_idx = read_pos_mtx_idx(ctx, vcd_lo);

    let tex_mtx_idx = read_tex_mtx_indices(ctx, vcd_lo);
    for (i, tmi) in tex_mtx_idx.iter().enumerate() {
        // Width must be I8: tex_mtx_slot is byte-addressed and storing
        // a wider value at byte offsets 0..7 would overlap and overflow.
        let byte = ctx.bd.ins().ireduce(ir::types::I8, *tmi);
        ctx.bd.ins().stack_store(byte, tex_mtx_slot, i as i32);
    }

    let pos_xyz = decode_position(ctx, vcd_lo.position(), vat_a);
    store_vec3(ctx.bd, ctx.out_ptr, offset::POSITION, &pos_xyz);
    ctx.bd.ins().stack_store(pos_xyz[0], raw_pos_slot, 0);
    ctx.bd.ins().stack_store(pos_xyz[1], raw_pos_slot, 4);
    ctx.bd.ins().stack_store(pos_xyz[2], raw_pos_slot, 8);

    let nrm_xyz = decode_normal(ctx, vcd_lo.normal(), vat_a);
    ctx.bd.ins().stack_store(nrm_xyz[0], raw_nrm_slot, 0);
    ctx.bd.ins().stack_store(nrm_xyz[1], raw_nrm_slot, 4);
    ctx.bd.ins().stack_store(nrm_xyz[2], raw_nrm_slot, 8);

    let color0 = decode_color(ctx, vcd_lo.color0(), vat_a.clr0_fmt(), vat_a.clr0_cnt(), 2);
    store_vec4(ctx.bd, ctx.out_ptr, offset::COLOR0, &color0);

    let color1 = decode_color(ctx, vcd_lo.color1(), vat_a.clr1_fmt(), vat_a.clr1_cnt(), 3);
    store_vec4(ctx.bd, ctx.out_ptr, offset::COLOR1, &color1);

    let mut present_mask: u32 = 0;
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

    for i in 0..8 {
        if !matches!(tex_attrs[i], AttributeType::None) {
            let st = decode_texcoord(ctx, tex_attrs[i], tex_fmts[i], tex_shifts[i], tex_cnts[i], 4 + i);
            ctx.bd.ins().stack_store(st[0], raw_tex_slot, (i * 8) as i32);
            ctx.bd.ins().stack_store(st[1], raw_tex_slot, (i * 8 + 4) as i32);
            present_mask |= 1u32 << i;
        }
    }

    let pos_view = xf_transform_3x4(ctx, pos_mtx_idx, &pos_xyz);
    store_vec3(ctx.bd, ctx.out_ptr, offset::POS_VIEW, &pos_view);

    let nrm_view = transform_and_normalize_normal(ctx, pos_mtx_idx, &nrm_xyz);
    store_vec3(ctx.bd, ctx.out_ptr, offset::NORMAL, &nrm_view);

    call_texgen_extern(
        ctx,
        raw_pos_slot,
        raw_nrm_slot,
        raw_tex_slot,
        present_mask,
        tex_mtx_slot,
    );
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
            MEMFLAGS_RO,
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
        MEMFLAGS_RO,
        ctx.xf_mem_ptr,
        xf_byte_off(XF_MATRIX_INDEX_A),
    );
    let cell_b = ctx.bd.ins().load(
        ir::types::I32,
        MEMFLAGS_RO,
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

fn attr_ptr_and_advance(ctx: &mut AttrCtx, attr: AttributeType, array_idx: usize, direct_size: usize) -> ir::Value {
    match attr {
        AttributeType::Direct => {
            let p = ctx.data_ptr;
            ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, direct_size as i64);
            p
        }
        AttributeType::Index8 => {
            let raw = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ctx.data_ptr, 0);
            let idx = ctx.bd.ins().uextend(ir::types::I32, raw);
            ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, 1);
            indexed_addr(ctx, array_idx, idx)
        }
        AttributeType::Index16 => {
            let raw = ctx.bd.ins().load(ir::types::I16, MEMFLAGS_RO, ctx.data_ptr, 0);
            let raw = ctx.bd.ins().bswap(raw);
            let idx = ctx.bd.ins().uextend(ir::types::I32, raw);
            ctx.data_ptr = ctx.bd.ins().iadd_imm(ctx.data_ptr, 2);
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

fn decode_position(ctx: &mut AttrCtx, attr: AttributeType, vat_a: crate::flipper::gx::regs::VatA) -> [ir::Value; 3] {
    if matches!(attr, AttributeType::None) {
        let z = ctx.bd.ins().f32const(0.0);
        return [z, z, z];
    }

    let count = vat_a.pos_cnt().components();
    let fmt = vat_a.pos_fmt();
    let direct = vat_a.pos_data_size();
    let recip = 1.0f32 / ((1u32 << vat_a.pos_shift()) as f32);
    let ptr = attr_ptr_and_advance(ctx, attr, ARRAY_POS, direct);
    let comp_size = fmt.size() as i32;
    let mut out = [ctx.bd.ins().f32const(0.0); 3];

    for i in 0..count {
        out[i] = decode_component_at(ctx, ptr, (i as i32) * comp_size, fmt, recip);
    }

    if matches!(vat_a.pos_cnt(), PosCount::Xy) {
        out[2] = ctx.bd.ins().f32const(0.0);
    }

    out
}

fn decode_normal(ctx: &mut AttrCtx, attr: AttributeType, vat_a: crate::flipper::gx::regs::VatA) -> [ir::Value; 3] {
    if matches!(attr, AttributeType::None) {
        let z = ctx.bd.ins().f32const(0.0);
        let one = ctx.bd.ins().f32const(1.0);
        return [z, z, one];
    }
    let cnt = vat_a.nrm_cnt().components().min(3);
    let fmt = vat_a.nrm_fmt();

    let direct = vat_a.nrm_data_size();
    let recip = match fmt {
        ComponentFormat::U8 | ComponentFormat::S8 => 1.0f32 / 64.0,
        ComponentFormat::U16 | ComponentFormat::S16 => 1.0f32 / 16384.0,
        ComponentFormat::F32 => 1.0f32,
    };

    let ptr = attr_ptr_and_advance(ctx, attr, ARRAY_NRM, direct);
    let comp_size = fmt.size() as i32;
    let mut out = [ctx.bd.ins().f32const(0.0); 3];

    for i in 0..cnt {
        out[i] = decode_component_at(ctx, ptr, (i as i32) * comp_size, fmt, recip);
    }

    out
}

fn decode_color(
    ctx: &mut AttrCtx,
    attr: AttributeType,
    fmt: ColorFormat,
    cnt: ColorCount,
    array_idx: usize,
) -> [ir::Value; 4] {
    let one = ctx.bd.ins().f32const(1.0);
    if matches!(attr, AttributeType::None) {
        return [one, one, one, one];
    }

    let direct = fmt.data_size(cnt);
    let ptr = attr_ptr_and_advance(ctx, attr, array_idx, direct);
    decode_color_bytes(ctx, ptr, fmt, cnt)
}

fn decode_color_bytes(ctx: &mut AttrCtx, ptr: ir::Value, fmt: ColorFormat, cnt: ColorCount) -> [ir::Value; 4] {
    let one = ctx.bd.ins().f32const(1.0);
    let has_alpha = matches!(cnt, ColorCount::Rgba);

    match fmt {
        ColorFormat::Rgb565 => {
            let raw = ctx.bd.ins().load(ir::types::I16, MEMFLAGS_RO, ptr, 0);
            let raw = ctx.bd.ins().bswap(raw);
            let raw = ctx.bd.ins().uextend(ir::types::I32, raw);
            let r = unpack_norm(ctx, raw, 11, 0x1F, 31.0);
            let g = unpack_norm(ctx, raw, 5, 0x3F, 63.0);
            let b = unpack_norm(ctx, raw, 0, 0x1F, 31.0);
            [r, g, b, one]
        }
        ColorFormat::Rgb8 | ColorFormat::Rgbx8 => {
            let r = byte_norm(ctx, ptr, 0, 255.0);
            let g = byte_norm(ctx, ptr, 1, 255.0);
            let b = byte_norm(ctx, ptr, 2, 255.0);
            [r, g, b, one]
        }
        ColorFormat::Rgba4 => {
            let raw = ctx.bd.ins().load(ir::types::I16, MEMFLAGS_RO, ptr, 0);
            let raw = ctx.bd.ins().bswap(raw);
            let raw = ctx.bd.ins().uextend(ir::types::I32, raw);
            let r = unpack_norm(ctx, raw, 12, 0xF, 15.0);
            let g = unpack_norm(ctx, raw, 8, 0xF, 15.0);
            let b = unpack_norm(ctx, raw, 4, 0xF, 15.0);
            let a = if has_alpha {
                unpack_norm(ctx, raw, 0, 0xF, 15.0)
            } else {
                one
            };
            [r, g, b, a]
        }
        ColorFormat::Rgba6 => {
            let b0 = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ptr, 0);
            let b1 = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ptr, 1);
            let b2 = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ptr, 2);
            let b0 = ctx.bd.ins().uextend(ir::types::I32, b0);
            let b1 = ctx.bd.ins().uextend(ir::types::I32, b1);
            let b2 = ctx.bd.ins().uextend(ir::types::I32, b2);
            let s0 = ctx.bd.ins().ishl_imm(b0, 16);
            let s1 = ctx.bd.ins().ishl_imm(b1, 8);
            let raw = ctx.bd.ins().bor(s0, s1);
            let raw = ctx.bd.ins().bor(raw, b2);
            let r = unpack_norm(ctx, raw, 18, 0x3F, 63.0);
            let g = unpack_norm(ctx, raw, 12, 0x3F, 63.0);
            let b = unpack_norm(ctx, raw, 6, 0x3F, 63.0);
            let a = if has_alpha {
                unpack_norm(ctx, raw, 0, 0x3F, 63.0)
            } else {
                one
            };
            [r, g, b, a]
        }
        ColorFormat::Rgba8 => {
            let r = byte_norm(ctx, ptr, 0, 255.0);
            let g = byte_norm(ctx, ptr, 1, 255.0);
            let b = byte_norm(ctx, ptr, 2, 255.0);
            let a = if has_alpha { byte_norm(ctx, ptr, 3, 255.0) } else { one };
            [r, g, b, a]
        }
    }
}

#[inline(always)]
fn byte_norm(ctx: &mut AttrCtx, ptr: ir::Value, off: i32, divisor: f32) -> ir::Value {
    let raw = ctx.bd.ins().load(ir::types::I8, MEMFLAGS_RO, ptr, off);
    let raw = ctx.bd.ins().uextend(ir::types::I32, raw);
    let f = ctx.bd.ins().fcvt_from_uint(ir::types::F32, raw);
    let d = ctx.bd.ins().f32const(divisor);
    ctx.bd.ins().fdiv(f, d)
}

#[inline(always)]
fn unpack_norm(ctx: &mut AttrCtx, raw: ir::Value, shift: i64, mask: i64, divisor: f32) -> ir::Value {
    let shifted = if shift > 0 {
        ctx.bd.ins().ushr_imm(raw, shift)
    } else {
        raw
    };
    let masked = ctx.bd.ins().band_imm(shifted, mask);
    let f = ctx.bd.ins().fcvt_from_uint(ir::types::F32, masked);
    let d = ctx.bd.ins().f32const(divisor);
    ctx.bd.ins().fdiv(f, d)
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
    let comp_size = fmt.size() as i32;
    let direct = count * fmt.size();
    let recip = 1.0f32 / ((1u32 << shift) as f32);
    let ptr = attr_ptr_and_advance(ctx, attr, array_idx, direct);
    let zero = ctx.bd.ins().f32const(0.0);
    let mut out = [zero, zero];
    for i in 0..count {
        out[i] = decode_component_at(ctx, ptr, (i as i32) * comp_size, fmt, recip);
    }
    out
}

fn xf_transform_3x4(ctx: &mut AttrCtx, pos_mtx_idx: ir::Value, pos: &[ir::Value; 3]) -> [ir::Value; 3] {
    // base_byte_off = pos_mtx_idx * XF_POS_MTX_STRIDE * 4
    let base_off = ctx.bd.ins().imul_imm(pos_mtx_idx, (XF_POS_MTX_STRIDE * 4) as i64);
    let base_off = ctx.bd.ins().uextend(ctx.pointer_ty, base_off);
    let base_addr = ctx.bd.ins().iadd(ctx.xf_mem_ptr, base_off);

    let m = std::array::from_fn::<ir::Value, 12, _>(|i| {
        ctx.bd
            .ins()
            .load(ir::types::F32, MEMFLAGS_RO, base_addr, (i * 4) as i32)
    });

    let row = |i: usize, ctx: &mut AttrCtx| -> ir::Value {
        let m0 = m[i * 4 + 0];
        let m1 = m[i * 4 + 1];
        let m2 = m[i * 4 + 2];
        let m3 = m[i * 4 + 3];
        let p0 = ctx.bd.ins().fmul(m0, pos[0]);
        let p1 = ctx.bd.ins().fmul(m1, pos[1]);
        let p2 = ctx.bd.ins().fmul(m2, pos[2]);
        let s = ctx.bd.ins().fadd(p0, p1);
        let s = ctx.bd.ins().fadd(s, p2);
        ctx.bd.ins().fadd(s, m3)
    };

    let r0 = row(0, ctx);
    let r1 = row(1, ctx);
    let r2 = row(2, ctx);
    [r0, r1, r2]
}

fn transform_and_normalize_normal(ctx: &mut AttrCtx, pos_mtx_idx: ir::Value, nrm: &[ir::Value; 3]) -> [ir::Value; 3] {
    // nrm_mtx_base = XF_NRM_MTX_BASE + (pos_mtx_idx & 31) * 3 cells
    let masked = ctx.bd.ins().band_imm(pos_mtx_idx, 31);
    let cell_off = ctx.bd.ins().imul_imm(masked, 3 * 4);
    let cell_off = ctx.bd.ins().uextend(ctx.pointer_ty, cell_off);
    let base = ctx.bd.ins().iadd(ctx.xf_mem_ptr, cell_off);

    let nm = std::array::from_fn::<ir::Value, 9, _>(|i| {
        ctx.bd
            .ins()
            .load(ir::types::F32, MEMFLAGS_RO, base, (XF_NRM_MTX_BASE * 4 + i * 4) as i32)
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

fn call_texgen_extern(
    ctx: &mut AttrCtx,
    raw_pos_slot: ir::StackSlot,
    raw_nrm_slot: ir::StackSlot,
    raw_tex_slot: ir::StackSlot,
    present_mask: u32,
    tex_mtx_slot: ir::StackSlot,
) {
    let pos_addr = ctx.bd.ins().stack_addr(ctx.pointer_ty, raw_pos_slot, 0);
    let nrm_addr = ctx.bd.ins().stack_addr(ctx.pointer_ty, raw_nrm_slot, 0);
    let tex_addr = ctx.bd.ins().stack_addr(ctx.pointer_ty, raw_tex_slot, 0);
    let mtx_addr = ctx.bd.ins().stack_addr(ctx.pointer_ty, tex_mtx_slot, 0);

    let out_tc_addr = ctx.bd.ins().iadd_imm(ctx.out_ptr, offset::TEXCOORDS as i64);

    let mask = ctx.bd.ins().iconst(ir::types::I32, present_mask as i64);

    let mut sig = ir::Signature::new(ctx.isa.default_call_conv());
    sig.params.push(ir::AbiParam::new(ctx.pointer_ty)); // gp
    sig.params.push(ir::AbiParam::new(ctx.pointer_ty)); // position
    sig.params.push(ir::AbiParam::new(ctx.pointer_ty)); // normal
    sig.params.push(ir::AbiParam::new(ctx.pointer_ty)); // raw_tex
    sig.params.push(ir::AbiParam::new(ir::types::I32)); // present_mask
    sig.params.push(ir::AbiParam::new(ctx.pointer_ty)); // tex_mtx
    sig.params.push(ir::AbiParam::new(ctx.pointer_ty)); // out_texcoords

    let sig_ref = ctx.bd.import_signature(sig);

    let target = super::runtime::gecko_gx_jit_apply_texgens as *const () as usize as i64;
    let target = ctx.bd.ins().iconst(ctx.pointer_ty, target);

    ctx.bd.ins().call_indirect(
        sig_ref,
        target,
        &[ctx.gp_ptr, pos_addr, nrm_addr, tex_addr, mask, mtx_addr, out_tc_addr],
    );
}

#[inline(always)]
fn store_vec3(bd: &mut FunctionBuilder, base: ir::Value, off: i32, v: &[ir::Value; 3]) {
    bd.ins().store(MEMFLAGS, v[0], base, off);
    bd.ins().store(MEMFLAGS, v[1], base, off + 4);
    bd.ins().store(MEMFLAGS, v[2], base, off + 8);
}

#[inline(always)]
fn store_vec4(bd: &mut FunctionBuilder, base: ir::Value, off: i32, v: &[ir::Value; 4]) {
    bd.ins().store(MEMFLAGS, v[0], base, off);
    bd.ins().store(MEMFLAGS, v[1], base, off + 4);
    bd.ins().store(MEMFLAGS, v[2], base, off + 8);
    bd.ins().store(MEMFLAGS, v[3], base, off + 12);
}
