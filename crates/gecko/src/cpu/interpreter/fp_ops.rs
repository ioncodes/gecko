use crate::cpu::condition::ConditionField;

#[inline(always)]
pub fn fp_ops<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    match OP {
        crate::cpu::lut::OP_MTFSFX => {
            let fm = instr.fm();
            let fb = ctx.cpu.read_fpr(instr.rb()).to_bits() as u32;
            let mut mask = 0u32;
            for i in 0u8..8 {
                if fm & (1 << (7 - i)) != 0 {
                    mask |= 0xF << ((7 - i) * 4);
                }
            }
            ctx.cpu.fpscr = (ctx.cpu.fpscr & !mask) | (fb & mask);
        }
        crate::cpu::lut::OP_MFFSX => {
            ctx.cpu.write_fpr(instr.rd(), f64::from_bits(ctx.cpu.fpscr as u64));
            if instr.rc() {
                ctx.cpu.update_cr1();
            }
        }
        crate::cpu::lut::OP_MTFSB0X => {
            ctx.cpu.fpscr &= !(1 << (31 - instr.crbd()));
        }
        crate::cpu::lut::OP_MTFSB1X => {
            ctx.cpu.fpscr |= 1 << (31 - instr.crbd());
        }
        crate::cpu::lut::OP_MTFSFIX => {
            let crfd = instr.crfd();
            let imm = (instr.0 >> 12) & 0xF;
            let shift = (7 - crfd) * 4;
            let mask = 0xFu32 << shift;
            ctx.cpu.fpscr = (ctx.cpu.fpscr & !mask) | (imm << shift);
        }
        crate::cpu::lut::OP_MCRFS => {
            let src_field = instr.crfs();
            let shift = (7 - src_field) * 4;
            let fpscr_nibble = (ctx.cpu.fpscr >> shift) & 0xF;
            ctx.cpu
                .cr
                .set_field(instr.crfd(), ConditionField::from(fpscr_nibble as u8));
            if src_field != 0 && src_field != 1 {
                ctx.cpu.fpscr &= !(0xF << shift);
            }
        }
        crate::cpu::lut::OP_FMRX => {
            ctx.cpu.write_fpr(instr.rd(), ctx.cpu.read_fpr(instr.rb()));
            if instr.rc() {
                ctx.cpu.update_cr1();
            }
        }
        crate::cpu::lut::OP_FNEGX => {
            ctx.cpu.write_fpr(instr.rd(), -ctx.cpu.read_fpr(instr.rb()));
            if instr.rc() {
                ctx.cpu.update_cr1();
            }
        }
        crate::cpu::lut::OP_FABSX => {
            ctx.cpu.write_fpr(instr.rd(), ctx.cpu.read_fpr(instr.rb()).abs());
            if instr.rc() {
                ctx.cpu.update_cr1();
            }
        }
        crate::cpu::lut::OP_FNABSX => {
            ctx.cpu.write_fpr(instr.rd(), -ctx.cpu.read_fpr(instr.rb()).abs());
            if instr.rc() {
                ctx.cpu.update_cr1();
            }
        }
        crate::cpu::lut::OP_FRSPX => {
            let val = ctx.cpu.read_fpr(instr.rb()) as f32 as f64;
            fp_write_single(ctx, &instr, val);
        }
        crate::cpu::lut::OP_FCTIWX => {
            let res = ctx.cpu.read_fpr(instr.rb()).round() as i32;
            ctx.cpu.write_fpr(instr.rd(), f64::from_bits(res as u32 as u64));
            if instr.rc() {
                ctx.cpu.update_cr1();
            }
        }
        crate::cpu::lut::OP_FCTIWZX => {
            let res = ctx.cpu.read_fpr(instr.rb()) as i32;
            ctx.cpu.write_fpr(instr.rd(), f64::from_bits(res as u32 as u64));
            if instr.rc() {
                ctx.cpu.update_cr1();
            }
        }
        crate::cpu::lut::OP_FCMPU | crate::cpu::lut::OP_FCMPO => {
            let fa = ctx.cpu.read_fpr(instr.ra());
            let fb = ctx.cpu.read_fpr(instr.rb());
            let cf = if fa.is_nan() || fb.is_nan() {
                ConditionField::new().with_so(true)
            } else if fa < fb {
                ConditionField::new().with_lt(true)
            } else if fa > fb {
                ConditionField::new().with_gt(true)
            } else {
                ConditionField::new().with_eq(true)
            };
            ctx.cpu.cr.set_field(instr.crfd(), cf);
        }
        crate::cpu::lut::OP_FADDX => fp_write(ctx, &instr, ctx.cpu.read_fpr(instr.ra()) + ctx.cpu.read_fpr(instr.rb())),
        crate::cpu::lut::OP_FSUBX => fp_write(ctx, &instr, ctx.cpu.read_fpr(instr.ra()) - ctx.cpu.read_fpr(instr.rb())),
        crate::cpu::lut::OP_FMULX => fp_write(ctx, &instr, ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc())),
        crate::cpu::lut::OP_FDIVX => fp_write(ctx, &instr, ctx.cpu.read_fpr(instr.ra()) / ctx.cpu.read_fpr(instr.rb())),
        crate::cpu::lut::OP_FMADDX => fp_write(
            ctx,
            &instr,
            ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) + ctx.cpu.read_fpr(instr.rb()),
        ),
        crate::cpu::lut::OP_FMSUBX => fp_write(
            ctx,
            &instr,
            ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) - ctx.cpu.read_fpr(instr.rb()),
        ),
        crate::cpu::lut::OP_FNMADDX => fp_write(
            ctx,
            &instr,
            -(ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) + ctx.cpu.read_fpr(instr.rb())),
        ),
        crate::cpu::lut::OP_FNMSUBX => fp_write(
            ctx,
            &instr,
            -(ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) - ctx.cpu.read_fpr(instr.rb())),
        ),
        crate::cpu::lut::OP_FADDSX => fp_write_single(
            ctx,
            &instr,
            (ctx.cpu.read_fpr(instr.ra()) + ctx.cpu.read_fpr(instr.rb())) as f32 as f64,
        ),
        crate::cpu::lut::OP_FSUBSX => fp_write_single(
            ctx,
            &instr,
            (ctx.cpu.read_fpr(instr.ra()) - ctx.cpu.read_fpr(instr.rb())) as f32 as f64,
        ),
        crate::cpu::lut::OP_FMULSX => fp_write_single(
            ctx,
            &instr,
            (ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc())) as f32 as f64,
        ),
        crate::cpu::lut::OP_FDIVSX => fp_write_single(
            ctx,
            &instr,
            (ctx.cpu.read_fpr(instr.ra()) / ctx.cpu.read_fpr(instr.rb())) as f32 as f64,
        ),
        crate::cpu::lut::OP_FMADDSX => fp_write_single(
            ctx,
            &instr,
            (ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) + ctx.cpu.read_fpr(instr.rb())) as f32 as f64,
        ),
        crate::cpu::lut::OP_FMSUBSX => fp_write_single(
            ctx,
            &instr,
            (ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) - ctx.cpu.read_fpr(instr.rb())) as f32 as f64,
        ),
        crate::cpu::lut::OP_FNMADDSX => fp_write_single(
            ctx,
            &instr,
            (-(ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) + ctx.cpu.read_fpr(instr.rb()))) as f32
                as f64,
        ),
        crate::cpu::lut::OP_FNMSUBSX => fp_write_single(
            ctx,
            &instr,
            (-(ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) - ctx.cpu.read_fpr(instr.rb()))) as f32
                as f64,
        ),
        crate::cpu::lut::OP_FSQRTX => fp_write(ctx, &instr, ctx.cpu.read_fpr(instr.rb()).sqrt()),
        crate::cpu::lut::OP_FSQRTSX => {
            fp_write_single(ctx, &instr, (ctx.cpu.read_fpr(instr.rb()).sqrt()) as f32 as f64)
        }
        crate::cpu::lut::OP_FRESX => {
            fp_write_single(ctx, &instr, (1.0f32 / ctx.cpu.read_fpr(instr.rb()) as f32) as f64)
        }
        crate::cpu::lut::OP_FRSQRTEX => fp_write(ctx, &instr, 1.0 / ctx.cpu.read_fpr(instr.rb()).sqrt()),
        crate::cpu::lut::OP_FSELX => {
            let fa = ctx.cpu.read_fpr(instr.ra());
            let fb = ctx.cpu.read_fpr(instr.rb());
            let fc = ctx.cpu.read_fpr(instr.fc());
            fp_write(ctx, &instr, if fa >= 0.0 { fc } else { fb });
        }

        _ => todo!("FP instruction with OP = {OP:#x}"),
    }
}

/// Write FP result to fD and optionally update CR1
#[inline(always)]
fn fp_write(ctx: &mut crate::gamecube::GameCube, instr: &crate::cpu::instruction::Instruction, val: f64) {
    ctx.cpu.write_fpr(instr.rd(), val);
    if instr.rc() {
        ctx.cpu.update_cr1();
    }
}

/// Write single-precision FP result to fD.
/// On Gekko, single-precision instructions duplicate the result into both ps0 and ps1.
#[inline(always)]
fn fp_write_single(ctx: &mut crate::gamecube::GameCube, instr: &crate::cpu::instruction::Instruction, val: f64) {
    ctx.cpu.write_fpr(instr.rd(), val);
    ctx.cpu.write_ps1(instr.rd(), val);
    if instr.rc() {
        ctx.cpu.update_cr1();
    }
}
