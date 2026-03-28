use crate::cpu::condition::BranchControl;

#[inline(always)]
pub fn branch<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::semantics::Instruction) {
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

            let (crf, bit) = (instr.bi() >> 2, instr.bi() & 0b11);
            let condition = match bit {
                0 => ctx.cpu.cr.get_field(crf).lt(),
                1 => ctx.cpu.cr.get_field(crf).gt(),
                2 => ctx.cpu.cr.get_field(crf).eq(),
                3 => ctx.cpu.cr.get_field(crf).so(),
                _ => panic!("Invalid CR bit index: {}", bit),
            };
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
