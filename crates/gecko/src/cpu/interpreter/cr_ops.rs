#[inline(always)]
pub fn cr_ops<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::semantics::Instruction) {
    match OP {
        crate::cpu::lut::OP_MTCRF => {
            let crm = instr.crm();
            let rs = ctx.cpu.read_gpr(instr.rs());
            let mut cr = ctx.cpu.cr.raw();
            for i in 0u8..8 {
                if crm & (1 << (7 - i)) != 0 {
                    let shift = (7 - i) * 4;
                    let mask = 0xFu32 << shift;
                    cr = (cr & !mask) | (rs & mask);
                }
            }
            ctx.cpu.cr = crate::cpu::condition::ConditionRegister::from(cr);
        }
        crate::cpu::lut::OP_MFCR => {
            ctx.cpu.write_gpr(instr.rd(), ctx.cpu.cr.raw());
        }
        crate::cpu::lut::OP_MCRF => {
            let src = ctx.cpu.cr.get_field(instr.crfs());
            ctx.cpu.cr.set_field(instr.crfd(), src);
        }
        // CR bit operations
        _ => {
            let a = ctx.cpu.cr.get_bit(instr.crba());
            let b = ctx.cpu.cr.get_bit(instr.crbb());
            let result = match OP {
                crate::cpu::lut::OP_CRXOR => a ^ b,
                crate::cpu::lut::OP_CROR => a | b,
                crate::cpu::lut::OP_CRAND => a & b,
                crate::cpu::lut::OP_CREQV => a == b,
                crate::cpu::lut::OP_CRNOR => !(a | b),
                crate::cpu::lut::OP_CRNAND => !(a & b),
                crate::cpu::lut::OP_CRANDC => a & !b,
                crate::cpu::lut::OP_CRORC => a | !b,
                _ => todo!("CR instruction with OP = {OP:#x}"),
            };
            ctx.cpu.cr.set_bit(instr.crbd(), result);
        }
    }
}
