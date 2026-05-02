use crate::gekko::condition::ConditionField;
use crate::gekko::fpscr::Fpscr;
use crate::gekko::instruction::Instruction;
use crate::gekko::lut::*;
use crate::system::{System, SystemId};

const FPSCR_FX: u32 = 1 << (31 - 0);
const FPSCR_OX: u32 = 1 << (31 - 3);
const FPSCR_UX: u32 = 1 << (31 - 4);
const FPSCR_ZX: u32 = 1 << (31 - 5);
const FPSCR_XX: u32 = 1 << (31 - 6);
const FPSCR_VXSNAN: u32 = 1 << (31 - 7);
const FPSCR_VXISI: u32 = 1 << (31 - 8);
const FPSCR_VXIDI: u32 = 1 << (31 - 9);
const FPSCR_VXZDZ: u32 = 1 << (31 - 10);
const FPSCR_VXIMZ: u32 = 1 << (31 - 11);
const FPSCR_VXVC: u32 = 1 << (31 - 12);
const FPSCR_VXSOFT: u32 = 1 << (31 - 21);
const FPSCR_VXSQRT: u32 = 1 << (31 - 22);
const FPSCR_VXCVI: u32 = 1 << (31 - 23);
const FPSCR_ANY_X: u32 = FPSCR_OX
    | FPSCR_UX
    | FPSCR_ZX
    | FPSCR_XX
    | FPSCR_VXSNAN
    | FPSCR_VXISI
    | FPSCR_VXIDI
    | FPSCR_VXZDZ
    | FPSCR_VXIMZ
    | FPSCR_VXVC
    | FPSCR_VXSOFT
    | FPSCR_VXSQRT
    | FPSCR_VXCVI;

#[inline(always)]
pub fn fp_ops<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    if !ctx.check_fp_available() {
        return;
    }

    match OP {
        OP_MTFSFX => {
            let fm = instr.fm();
            let fb = ctx.gekko.read_fpr(instr.rb()).to_bits() as u32;
            let mut mask = 0u32;
            for i in 0u8..8 {
                if fm & (1 << (7 - i)) != 0 {
                    mask |= 0xF << ((7 - i) * 4);
                }
            }
            ctx.gekko.fpscr = Fpscr::from((ctx.gekko.fpscr.raw() & !mask) | (fb & mask));
            ctx.gekko.recompute_fpscr_summary();
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_MFFSX => {
            ctx.gekko.write_fpr(
                instr.rd(),
                f64::from_bits(0xFFF8_0000_0000_0000 | ctx.gekko.fpscr.raw() as u64),
            );
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_MTFSB0X => {
            ctx.gekko.fpscr = Fpscr::from(ctx.gekko.fpscr.raw() & !(1 << (31 - instr.crbd())));
            ctx.gekko.recompute_fpscr_summary();
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_MTFSB1X => {
            let bit = 1 << (31 - instr.crbd());
            let mut fpscr = ctx.gekko.fpscr.raw();
            if bit & FPSCR_ANY_X != 0 && fpscr & bit == 0 {
                fpscr |= FPSCR_FX;
            }
            ctx.gekko.fpscr = Fpscr::from(fpscr | bit);
            ctx.gekko.recompute_fpscr_summary();
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_MTFSFIX => {
            let crfd = instr.crfd();
            let imm = (instr.0 >> 12) & 0xF;
            let shift = (7 - crfd) * 4;
            let mask = 0xFu32 << shift;
            ctx.gekko.fpscr = Fpscr::from((ctx.gekko.fpscr.raw() & !mask) | (imm << shift));
            ctx.gekko.recompute_fpscr_summary();
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_MCRFS => {
            let src_field = instr.crfs();
            let shift = (7 - src_field) * 4;
            let fpscr_nibble = (ctx.gekko.fpscr.raw() >> shift) & 0xF;
            ctx.gekko
                .cr
                .set_field(instr.crfd(), ConditionField::from(fpscr_nibble as u8));
            let clear_mask = (0xF << shift) & (FPSCR_FX | FPSCR_ANY_X);
            ctx.gekko.fpscr = Fpscr::from(ctx.gekko.fpscr.raw() & !clear_mask);
            ctx.gekko.recompute_fpscr_summary();
        }
        OP_FMRX => {
            ctx.gekko.write_fpr(instr.rd(), ctx.gekko.read_fpr(instr.rb()));
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_FNEGX => {
            ctx.gekko.write_fpr(instr.rd(), -ctx.gekko.read_fpr(instr.rb()));
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_FABSX => {
            ctx.gekko.write_fpr(instr.rd(), ctx.gekko.read_fpr(instr.rb()).abs());
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_FNABSX => {
            ctx.gekko.write_fpr(instr.rd(), -ctx.gekko.read_fpr(instr.rb()).abs());
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_FRSPX => {
            let val = ctx.gekko.read_fpr(instr.rb()) as f32 as f64;
            fp_write_single(ctx, &instr, val);
        }
        OP_FCTIWX => {
            let res = ctx.gekko.read_fpr(instr.rb()).round() as i32;
            ctx.gekko.write_fpr(instr.rd(), f64::from_bits(res as u32 as u64));
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_FCTIWZX => {
            let res = ctx.gekko.read_fpr(instr.rb()) as i32;
            ctx.gekko.write_fpr(instr.rd(), f64::from_bits(res as u32 as u64));
            if instr.rc() {
                ctx.gekko.update_cr1();
            }
        }
        OP_FCMPU | OP_FCMPO => {
            let fa = ctx.gekko.read_fpr(instr.ra());
            let fb = ctx.gekko.read_fpr(instr.rb());
            let cf = if fa.is_nan() || fb.is_nan() {
                ConditionField::new().with_so(true)
            } else if fa < fb {
                ConditionField::new().with_lt(true)
            } else if fa > fb {
                ConditionField::new().with_gt(true)
            } else {
                ConditionField::new().with_eq(true)
            };
            ctx.gekko.cr.set_field(instr.crfd(), cf);
        }
        OP_FADDX => fp_write(
            ctx,
            &instr,
            ctx.gekko.read_fpr(instr.ra()) + ctx.gekko.read_fpr(instr.rb()),
        ),
        OP_FSUBX => fp_write(
            ctx,
            &instr,
            ctx.gekko.read_fpr(instr.ra()) - ctx.gekko.read_fpr(instr.rb()),
        ),
        OP_FMULX => fp_write(
            ctx,
            &instr,
            ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()),
        ),
        OP_FDIVX => fp_write(
            ctx,
            &instr,
            ctx.gekko.read_fpr(instr.ra()) / ctx.gekko.read_fpr(instr.rb()),
        ),
        OP_FMADDX => fp_write(
            ctx,
            &instr,
            ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) + ctx.gekko.read_fpr(instr.rb()),
        ),
        OP_FMSUBX => fp_write(
            ctx,
            &instr,
            ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) - ctx.gekko.read_fpr(instr.rb()),
        ),
        OP_FNMADDX => fp_write(
            ctx,
            &instr,
            -(ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) + ctx.gekko.read_fpr(instr.rb())),
        ),
        OP_FNMSUBX => fp_write(
            ctx,
            &instr,
            -(ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) - ctx.gekko.read_fpr(instr.rb())),
        ),
        OP_FADDSX => fp_write_single(
            ctx,
            &instr,
            (ctx.gekko.read_fpr(instr.ra()) + ctx.gekko.read_fpr(instr.rb())) as f32 as f64,
        ),
        OP_FSUBSX => fp_write_single(
            ctx,
            &instr,
            (ctx.gekko.read_fpr(instr.ra()) - ctx.gekko.read_fpr(instr.rb())) as f32 as f64,
        ),
        OP_FMULSX => fp_write_single(
            ctx,
            &instr,
            (ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc())) as f32 as f64,
        ),
        OP_FDIVSX => fp_write_single(
            ctx,
            &instr,
            (ctx.gekko.read_fpr(instr.ra()) / ctx.gekko.read_fpr(instr.rb())) as f32 as f64,
        ),
        OP_FMADDSX => fp_write_single(
            ctx,
            &instr,
            (ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) + ctx.gekko.read_fpr(instr.rb())) as f32
                as f64,
        ),
        OP_FMSUBSX => fp_write_single(
            ctx,
            &instr,
            (ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) - ctx.gekko.read_fpr(instr.rb())) as f32
                as f64,
        ),
        OP_FNMADDSX => fp_write_single(
            ctx,
            &instr,
            (-(ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) + ctx.gekko.read_fpr(instr.rb()))) as f32
                as f64,
        ),
        OP_FNMSUBSX => fp_write_single(
            ctx,
            &instr,
            (-(ctx.gekko.read_fpr(instr.ra()) * ctx.gekko.read_fpr(instr.fc()) - ctx.gekko.read_fpr(instr.rb()))) as f32
                as f64,
        ),
        OP_FSQRTX => fp_write(ctx, &instr, ctx.gekko.read_fpr(instr.rb()).sqrt()),
        OP_FSQRTSX => fp_write_single(ctx, &instr, (ctx.gekko.read_fpr(instr.rb()).sqrt()) as f32 as f64),
        OP_FRESX => fp_write_single(ctx, &instr, (1.0f32 / ctx.gekko.read_fpr(instr.rb()) as f32) as f64),
        OP_FRSQRTEX => fp_write(ctx, &instr, 1.0 / ctx.gekko.read_fpr(instr.rb()).sqrt()),
        OP_FSELX => {
            let fa = ctx.gekko.read_fpr(instr.ra());
            let fb = ctx.gekko.read_fpr(instr.rb());
            let fc = ctx.gekko.read_fpr(instr.fc());
            fp_write(ctx, &instr, if fa >= 0.0 { fc } else { fb });
        }

        _ => todo!("FP instruction with OP = {OP:#x}"),
    }

    ctx.check_fp_program_exception();
}

/// Write FP result to fD and optionally update CR1
#[inline(always)]
fn fp_write<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: &Instruction, val: f64) {
    ctx.gekko.write_fpr(instr.rd(), val);
    if instr.rc() {
        ctx.gekko.update_cr1();
    }
}

/// Write single-precision FP result to fD.
/// On Gekko, single-precision instructions duplicate the result into both ps0 and ps1.
#[inline(always)]
fn fp_write_single<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: &Instruction, val: f64) {
    ctx.gekko.write_fpr(instr.rd(), val);
    ctx.gekko.write_ps1(instr.rd(), val);
    if instr.rc() {
        ctx.gekko.update_cr1();
    }
}
