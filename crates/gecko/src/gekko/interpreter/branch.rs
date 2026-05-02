use crate::gekko::condition::BranchControl;
use crate::gekko::instruction::Instruction;
use crate::gekko::lut::*;
use crate::system::{System, SystemId};

#[inline(always)]
pub fn branch<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    match OP {
        OP_BX => {
            ctx.gekko.nia = if instr.aa() {
                instr.li() as u32
            } else {
                ctx.gekko.cia.wrapping_add_signed(instr.li())
            };
            if instr.lk() {
                ctx.gekko.spr.lr = ctx.gekko.cia.wrapping_add(4);
            }
        }
        OP_BCX => {
            let ctrl = BranchControl::from_bo(instr.bo());
            tracing::trace!("Branch control: {ctrl:?}");

            if ctrl.should_decrement_ctr() {
                ctx.gekko.spr.ctr = ctx.gekko.spr.ctr.wrapping_sub(1);
            }

            let condition = ctx.gekko.cr.get_bit(instr.bi());
            if !ctrl.should_branch(ctx.gekko.spr.ctr, condition) {
                return;
            }

            ctx.gekko.nia = if instr.aa() {
                instr.bd() as u32
            } else {
                ctx.gekko.cia.wrapping_add_signed(instr.bd())
            };
            if instr.lk() {
                ctx.gekko.spr.lr = ctx.gekko.cia.wrapping_add(4);
            }
        }
        OP_BCLRX => {
            let ctrl = BranchControl::from_bo(instr.bo());
            tracing::trace!("Branch control: {ctrl:?}");

            if ctrl.should_decrement_ctr() {
                ctx.gekko.spr.ctr = ctx.gekko.spr.ctr.wrapping_sub(1);
            }

            let condition = ctx.gekko.cr.get_bit(instr.bi());
            if !ctrl.should_branch(ctx.gekko.spr.ctr, condition) {
                return;
            }

            ctx.gekko.nia = ctx.gekko.spr.lr & !3;
            if instr.lk() {
                ctx.gekko.spr.lr = ctx.gekko.cia.wrapping_add(4);
            }
        }
        OP_BCCTRX => {
            let bo = instr.bo();
            let condition = (bo & 0x10) != 0 || (ctx.gekko.cr.get_bit(instr.bi()) == ((bo & 0x08) != 0));
            if !condition {
                return;
            }

            ctx.gekko.nia = ctx.gekko.spr.ctr & !3;
            if instr.lk() {
                ctx.gekko.spr.lr = ctx.gekko.cia.wrapping_add(4);
            }
        }
        _ => todo!("branch instruction with OP = {OP:#x}"),
    };
}
