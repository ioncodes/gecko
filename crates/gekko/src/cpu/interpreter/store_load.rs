use crate::cpu::condition::ConditionField;

pub fn store_load<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    match OP {
        crate::cpu::lut::OP_STW | crate::cpu::lut::OP_STWU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            ctx.write_u32(addr, ctx.cpu.read_gpr(instr.rs()));
            if OP == crate::cpu::lut::OP_STWU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STH | crate::cpu::lut::OP_STHU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            ctx.write_u16(addr, ctx.cpu.read_gpr(instr.rs()) as u16);
            if OP == crate::cpu::lut::OP_STHU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STB | crate::cpu::lut::OP_STBU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            ctx.write_u8(addr, ctx.cpu.read_gpr(instr.rs()) as u8);
            if OP == crate::cpu::lut::OP_STBU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LWZ | crate::cpu::lut::OP_LWZU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            let val = ctx.read_u32(addr);
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LWZU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LBZ | crate::cpu::lut::OP_LBZU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            let val = ctx.read_u8(addr) as u32;
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LBZU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LHZ | crate::cpu::lut::OP_LHZU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            let val = ctx.read_u16(addr) as u32;
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LHZU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LHA | crate::cpu::lut::OP_LHAU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            let val = ctx.read_u16(addr) as i16 as i32 as u32;
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LHAU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LMW => {
            let mut addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            for r in instr.rd()..32 {
                let val = ctx.read_u32(addr);
                ctx.cpu.write_gpr(r, val);
                addr = addr.wrapping_add(4);
            }
        }
        crate::cpu::lut::OP_STMW => {
            let mut addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            for r in instr.rs()..32 {
                let val = ctx.cpu.read_gpr(r);
                ctx.write_u32(addr, val);
                addr = addr.wrapping_add(4);
            }
        }
        crate::cpu::lut::OP_LWZX | crate::cpu::lut::OP_LWZUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let val = ctx.read_u32(addr);
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LWZUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LBZX | crate::cpu::lut::OP_LBZUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let val = ctx.read_u8(addr) as u32;
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LBZUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LHZX | crate::cpu::lut::OP_LHZUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let val = ctx.read_u16(addr) as u32;
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LHZUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LHAX | crate::cpu::lut::OP_LHAUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let val = ctx.read_u16(addr) as i16 as i32 as u32;
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LHAUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STWX | crate::cpu::lut::OP_STWUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            ctx.write_u32(addr, ctx.cpu.read_gpr(instr.rs()));
            if OP == crate::cpu::lut::OP_STWUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STBX | crate::cpu::lut::OP_STBUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            ctx.write_u8(addr, ctx.cpu.read_gpr(instr.rs()) as u8);
            if OP == crate::cpu::lut::OP_STBUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STHX | crate::cpu::lut::OP_STHUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            ctx.write_u16(addr, ctx.cpu.read_gpr(instr.rs()) as u16);
            if OP == crate::cpu::lut::OP_STHUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        _ => todo!("Store/Load instruction with OP = {OP:#x}"),
    }
}

pub fn lwarx(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    let addr = ctx
        .cpu
        .read_gpr_or_zero(instr.ra())
        .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
    let val = ctx.read_u32(addr);
    ctx.cpu.write_gpr(instr.rd(), val);
    ctx.cpu.reserve_addr = Some(addr);
}

pub fn stwcx_dot(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    let addr = ctx
        .cpu
        .read_gpr_or_zero(instr.ra())
        .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
    let so = ctx.cpu.spr.xer.summary_overflow();
    let store_performed = ctx.cpu.reserve_addr.is_some();
    ctx.cpu.reserve_addr = None;
    if store_performed {
        ctx.write_u32(addr, ctx.cpu.read_gpr(instr.rs()));
        ctx.cpu.cr.set_cr0(ConditionField::new().with_eq(true).with_so(so));
    } else {
        ctx.cpu.cr.set_cr0(ConditionField::new().with_so(so));
    }
}

pub fn store_load_fp<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    match OP {
        crate::cpu::lut::OP_LFD | crate::cpu::lut::OP_LFDU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            let val = ctx.read_f64(addr);
            ctx.cpu.write_fpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LFDU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STFD | crate::cpu::lut::OP_STFDU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            ctx.write_f64(addr, ctx.cpu.read_fpr(instr.rs()));
            if OP == crate::cpu::lut::OP_STFDU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LFS | crate::cpu::lut::OP_LFSU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            let val = ctx.read_f32(addr);
            ctx.cpu.write_fpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LFSU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STFS | crate::cpu::lut::OP_STFSU => {
            let addr = ctx.cpu.read_gpr_or_zero(instr.ra()).wrapping_add_signed(instr.disp());
            ctx.write_f32(addr, ctx.cpu.read_fpr(instr.rs()));
            if OP == crate::cpu::lut::OP_STFSU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LFSX | crate::cpu::lut::OP_LFSUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let val = ctx.read_f32(addr);
            ctx.cpu.write_fpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LFSUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LFDX | crate::cpu::lut::OP_LFDUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            let val = ctx.read_f64(addr);
            ctx.cpu.write_fpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LFDUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STFSX | crate::cpu::lut::OP_STFSUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            ctx.write_f32(addr, ctx.cpu.read_fpr(instr.rs()));
            if OP == crate::cpu::lut::OP_STFSUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STFDX | crate::cpu::lut::OP_STFDUX => {
            let addr = ctx
                .cpu
                .read_gpr_or_zero(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            ctx.write_f64(addr, ctx.cpu.read_fpr(instr.rs()));
            if OP == crate::cpu::lut::OP_STFDUX {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        _ => todo!("FP Store/Load instruction with OP = {OP:#x}"),
    }
}
