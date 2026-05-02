use crate::gekko::condition::ConditionRegister;
use crate::gekko::instruction::Instruction;
use crate::gekko::lut::*;
use crate::system::{System, SystemId};

#[inline(always)]
pub fn compare<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    let mut field = match OP {
        OP_CMP => ConditionRegister::field_from_ord(
            (ctx.gekko.read_gpr(instr.ra()) as i32).cmp(&(ctx.gekko.read_gpr(instr.rb()) as i32)),
        ),
        OP_CMPI => ConditionRegister::field_from_ord((ctx.gekko.read_gpr(instr.ra()) as i32).cmp(&instr.simm())),
        OP_CMPL => {
            ConditionRegister::field_from_ord(ctx.gekko.read_gpr(instr.ra()).cmp(&ctx.gekko.read_gpr(instr.rb())))
        }
        OP_CMPLI => ConditionRegister::field_from_ord(ctx.gekko.read_gpr(instr.ra()).cmp(&(instr.uimm() as u32))),
        _ => todo!("Compare instruction with OP = {OP:#x}"),
    };

    field = field.with_so(ctx.gekko.spr.xer.summary_overflow());
    ctx.gekko.cr.set_field(instr.crfd(), field);
}
