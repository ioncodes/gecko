use crate::cpu::condition::ConditionRegister;

pub fn compare<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    let field = match OP {
        crate::cpu::lut::OP_CMP => ConditionRegister::field_from_ord(
            (ctx.cpu.read_gpr(instr.ra()) as i32).cmp(&(ctx.cpu.read_gpr(instr.rb()) as i32)),
        ),
        crate::cpu::lut::OP_CMPI => {
            ConditionRegister::field_from_ord((ctx.cpu.read_gpr(instr.ra()) as i32).cmp(&instr.simm()))
        }
        crate::cpu::lut::OP_CMPL => {
            ConditionRegister::field_from_ord(ctx.cpu.read_gpr(instr.ra()).cmp(&ctx.cpu.read_gpr(instr.rb())))
        }
        crate::cpu::lut::OP_CMPLI => {
            ConditionRegister::field_from_ord(ctx.cpu.read_gpr(instr.ra()).cmp(&(instr.uimm() as u32)))
        }
        _ => todo!("Compare instruction with OP = {OP:#x}"),
    };

    ctx.cpu.cr.set_field(instr.crfd(), field);
}
