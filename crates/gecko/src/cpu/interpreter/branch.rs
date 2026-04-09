use crate::cpu::condition::BranchControl;

#[inline(always)]
pub fn branch<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    // Read LR before potentially overwriting LR with CIA+4 (matters for blrl/bctrl)
    let old_lr = ctx.cpu.spr.lr;

    if instr.lk() {
        ctx.cpu.spr.lr = ctx.cpu.cia.wrapping_add(4);
    }

    match OP {
        crate::cpu::lut::OP_BX => {
            ctx.cpu.nia = if instr.aa() {
                instr.li() as u32
            } else {
                ctx.cpu.cia.wrapping_add_signed(instr.li())
            }
        }
        crate::cpu::lut::OP_BCLRX | crate::cpu::lut::OP_BCX | crate::cpu::lut::OP_BCCTRX => {
            let ctrl = BranchControl::from_bo(instr.bo());
            tracing::trace!("Branch control: {ctrl:?}");

            if ctrl.should_decrement_ctr() {
                ctx.cpu.spr.ctr = ctx.cpu.spr.ctr.wrapping_sub(1);
            }

            let condition = ctx.cpu.cr.get_bit(instr.bi());
            if !ctrl.should_branch(ctx.cpu.spr.ctr, condition) {
                return;
            }

            match OP {
                crate::cpu::lut::OP_BCLRX => ctx.cpu.nia = old_lr,
                crate::cpu::lut::OP_BCX => {
                    ctx.cpu.nia = if instr.aa() {
                        instr.bd() as u32
                    } else {
                        ctx.cpu.cia.wrapping_add_signed(instr.bd())
                    }
                }
                crate::cpu::lut::OP_BCCTRX => ctx.cpu.nia = ctx.cpu.spr.ctr,
                _ => tracing::error!("missing OP = {OP:#x}"),
            }
        }
        _ => todo!("branch instruction with OP = {OP:#x}"),
    };
}
