use crate::cpu::condition::ConditionField;

#[inline(always)]
pub fn ps_ops<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    match OP {
        crate::cpu::lut::OP_PS_ADD => {
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) + ctx.cpu.read_fpr(instr.rb())) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) + ctx.cpu.read_ps1(instr.rb())) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_SUB => {
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) - ctx.cpu.read_fpr(instr.rb())) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) - ctx.cpu.read_ps1(instr.rb())) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MUL => {
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc())) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) * ctx.cpu.read_ps1(instr.fc())) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_DIV => {
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) / ctx.cpu.read_fpr(instr.rb())) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) / ctx.cpu.read_ps1(instr.rb())) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MADD => {
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) + ctx.cpu.read_fpr(instr.rb()))
                as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) * ctx.cpu.read_ps1(instr.fc()) + ctx.cpu.read_ps1(instr.rb()))
                as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MSUB => {
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) - ctx.cpu.read_fpr(instr.rb()))
                as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) * ctx.cpu.read_ps1(instr.fc()) - ctx.cpu.read_ps1(instr.rb()))
                as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_NMADD => {
            let ps0 = -(ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) + ctx.cpu.read_fpr(instr.rb()))
                as f32 as f64;
            let ps1 = -(ctx.cpu.read_ps1(instr.ra()) * ctx.cpu.read_ps1(instr.fc()) + ctx.cpu.read_ps1(instr.rb()))
                as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_NMSUB => {
            let ps0 = -(ctx.cpu.read_fpr(instr.ra()) * ctx.cpu.read_fpr(instr.fc()) - ctx.cpu.read_fpr(instr.rb()))
                as f32 as f64;
            let ps1 = -(ctx.cpu.read_ps1(instr.ra()) * ctx.cpu.read_ps1(instr.fc()) - ctx.cpu.read_ps1(instr.rb()))
                as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MULS0 => {
            let c0 = ctx.cpu.read_fpr(instr.fc());
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) * c0) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) * c0) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MULS1 => {
            let c1 = ctx.cpu.read_ps1(instr.fc());
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) * c1) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) * c1) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MADDS0 => {
            let c0 = ctx.cpu.read_fpr(instr.fc());
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) * c0 + ctx.cpu.read_fpr(instr.rb())) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) * c0 + ctx.cpu.read_ps1(instr.rb())) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MADDS1 => {
            let c1 = ctx.cpu.read_ps1(instr.fc());
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) * c1 + ctx.cpu.read_fpr(instr.rb())) as f32 as f64;
            let ps1 = (ctx.cpu.read_ps1(instr.ra()) * c1 + ctx.cpu.read_ps1(instr.rb())) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_SUM0 => {
            let ps0 = (ctx.cpu.read_fpr(instr.ra()) + ctx.cpu.read_ps1(instr.rb())) as f32 as f64;
            let ps1 = ctx.cpu.read_ps1(instr.fc());
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_SUM1 => {
            let ps0 = ctx.cpu.read_fpr(instr.fc());
            let ps1 = (ctx.cpu.read_fpr(instr.ra()) + ctx.cpu.read_ps1(instr.rb())) as f32 as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_MR => {
            ps_write(ctx, &instr, ctx.cpu.read_fpr(instr.rb()), ctx.cpu.read_ps1(instr.rb()));
        }
        crate::cpu::lut::OP_PS_NEG => {
            ps_write(
                ctx,
                &instr,
                -ctx.cpu.read_fpr(instr.rb()),
                -ctx.cpu.read_ps1(instr.rb()),
            );
        }
        crate::cpu::lut::OP_PS_ABS => {
            ps_write(
                ctx,
                &instr,
                ctx.cpu.read_fpr(instr.rb()).abs(),
                ctx.cpu.read_ps1(instr.rb()).abs(),
            );
        }
        crate::cpu::lut::OP_PS_NABS => {
            ps_write(
                ctx,
                &instr,
                -ctx.cpu.read_fpr(instr.rb()).abs(),
                -ctx.cpu.read_ps1(instr.rb()).abs(),
            );
        }
        crate::cpu::lut::OP_PS_MERGE00 => {
            ps_write(ctx, &instr, ctx.cpu.read_fpr(instr.ra()), ctx.cpu.read_fpr(instr.rb()));
        }
        crate::cpu::lut::OP_PS_MERGE01 => {
            ps_write(ctx, &instr, ctx.cpu.read_fpr(instr.ra()), ctx.cpu.read_ps1(instr.rb()));
        }
        crate::cpu::lut::OP_PS_MERGE10 => {
            ps_write(ctx, &instr, ctx.cpu.read_ps1(instr.ra()), ctx.cpu.read_fpr(instr.rb()));
        }
        crate::cpu::lut::OP_PS_MERGE11 => {
            ps_write(ctx, &instr, ctx.cpu.read_ps1(instr.ra()), ctx.cpu.read_ps1(instr.rb()));
        }
        crate::cpu::lut::OP_PS_SEL => {
            let ps0 = if ctx.cpu.read_fpr(instr.ra()) >= 0.0 {
                ctx.cpu.read_fpr(instr.fc())
            } else {
                ctx.cpu.read_fpr(instr.rb())
            };
            let ps1 = if ctx.cpu.read_ps1(instr.ra()) >= 0.0 {
                ctx.cpu.read_ps1(instr.fc())
            } else {
                ctx.cpu.read_ps1(instr.rb())
            };
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_CMPU0 | crate::cpu::lut::OP_PS_CMPO0 => {
            let fa = ctx.cpu.read_fpr(instr.ra());
            let fb = ctx.cpu.read_fpr(instr.rb());
            ctx.cpu.cr.set_field(instr.crfd(), fp_compare(fa, fb));
        }
        crate::cpu::lut::OP_PS_CMPU1 | crate::cpu::lut::OP_PS_CMPO1 => {
            let fa = ctx.cpu.read_ps1(instr.ra());
            let fb = ctx.cpu.read_ps1(instr.rb());
            ctx.cpu.cr.set_field(instr.crfd(), fp_compare(fa, fb));
        }
        crate::cpu::lut::OP_PS_RES => {
            let ps0 = (1.0f32 / ctx.cpu.read_fpr(instr.rb()) as f32) as f64;
            let ps1 = (1.0f32 / ctx.cpu.read_ps1(instr.rb()) as f32) as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        crate::cpu::lut::OP_PS_RSQRTE => {
            let ps0 = (1.0f32 / (ctx.cpu.read_fpr(instr.rb()) as f32).sqrt()) as f64;
            let ps1 = (1.0f32 / (ctx.cpu.read_ps1(instr.rb()) as f32).sqrt()) as f64;
            ps_write(ctx, &instr, ps0, ps1);
        }
        _ => todo!("PS instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
fn ps_write(ctx: &mut crate::gamecube::GameCube, instr: &crate::cpu::instruction::Instruction, ps0: f64, ps1: f64) {
    ctx.cpu.write_fpr(instr.rd(), ps0);
    ctx.cpu.write_ps1(instr.rd(), ps1);
    if instr.rc() {
        ctx.cpu.update_cr1();
    }
}

#[inline(always)]
fn fp_compare(a: f64, b: f64) -> ConditionField {
    if a.is_nan() || b.is_nan() {
        ConditionField::new().with_so(true)
    } else if a < b {
        ConditionField::new().with_lt(true)
    } else if a > b {
        ConditionField::new().with_gt(true)
    } else {
        ConditionField::new().with_eq(true)
    }
}
