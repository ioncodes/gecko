use crate::cpu::sr::Sr;

#[inline(always)]
pub fn msr<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    match OP {
        crate::cpu::lut::OP_MTMSR => {
            ctx.cpu.msr = crate::cpu::msr::Msr::from(ctx.cpu.read_gpr(instr.rs()));
        }
        crate::cpu::lut::OP_MFMSR => {
            ctx.cpu.write_gpr(instr.rd(), ctx.cpu.msr.raw());
        }
        crate::cpu::lut::OP_RFI => {
            const RFI_MSR_MASK: u32 = 0x0000_FF73;
            ctx.cpu.msr =
                crate::cpu::msr::Msr::from((ctx.cpu.msr.raw() & !RFI_MSR_MASK) | (ctx.cpu.spr.srr1 & RFI_MSR_MASK));
            ctx.cpu.nia = ctx.cpu.spr.srr0.value() << 2;
        }
        _ => todo!("MSR instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn spr<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    match OP {
        crate::cpu::lut::OP_MTSPR => {
            let spr_num = instr.spr_swapped();
            let val = ctx.cpu.read_gpr(instr.rs());
            match spr_num {
                284 => ctx.scheduler.set_timebase_lower(val),
                285 => ctx.scheduler.set_timebase_upper(val),
                _ => ctx.cpu.spr.write(spr_num, val),
            }
        }
        crate::cpu::lut::OP_MFSPR => {
            let spr_num = instr.spr_swapped();
            let val = match spr_num {
                268 => ctx.scheduler.timebase_lower(),
                269 => ctx.scheduler.timebase_upper(),
                _ => ctx.cpu.spr.read(spr_num),
            };
            ctx.cpu.write_gpr(instr.rd(), val);
        }
        _ => todo!("SPR instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn segment<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    match OP {
        crate::cpu::lut::OP_MTSR => {
            ctx.cpu.sr[instr.sr() as usize] = Sr::from_raw(ctx.cpu.read_gpr(instr.rs()));
        }
        crate::cpu::lut::OP_MFSR => {
            ctx.cpu.write_gpr(instr.rd(), ctx.cpu.sr[instr.sr() as usize].raw());
        }
        _ => todo!("Segment Register instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn mftb(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    let tbr = instr.spr_swapped();
    let val = match tbr {
        268 => ctx.scheduler.timebase_lower(),
        269 => ctx.scheduler.timebase_upper(),
        _ => panic!("unknown TBR {tbr}"),
    };
    ctx.cpu.write_gpr(instr.rd(), val);
}

#[inline(always)]
pub fn nop<const OP: u32>(_ctx: &mut crate::gamecube::GameCube, _instr: crate::cpu::instruction::Instruction) {}

#[inline(always)]
pub fn sc(ctx: &mut crate::gamecube::GameCube, _instr: crate::cpu::instruction::Instruction) {
    ctx.cause_syscall_interrupt();
}
