use crate::cpu::{irq, spr::Srr0, sr::Sr};

pub fn msr<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
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

pub fn spr<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    match OP {
        crate::cpu::lut::OP_MTSPR => {
            ctx.cpu.spr.write(instr.spr_swapped(), ctx.cpu.read_gpr(instr.rs()));
        }
        crate::cpu::lut::OP_MFSPR => {
            ctx.cpu.write_gpr(instr.rd(), ctx.cpu.spr.read(instr.spr_swapped()));
        }
        _ => todo!("SPR instruction with OP = {OP:#x}"),
    }
}

pub fn segment<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
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

pub fn mftb(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    let tbr = instr.spr_swapped();
    let val = match tbr {
        268 => ctx.scheduler.cycles as u32,         // TBL
        269 => (ctx.scheduler.cycles >> 32) as u32, // TBU
        _ => panic!("unknown TBR {tbr}"),
    };
    ctx.cpu.write_gpr(instr.rd(), val);
}

pub fn nop<const OP: u32>(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) {}

pub fn sc(ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) {
    let base: u32 = if ctx.cpu.msr.exception_prefix() { 0xFFF0_0000 } else { 0 };

    ctx.cpu.spr.srr0 = Srr0::from(ctx.cpu.cia.wrapping_add(4));
    ctx.cpu.spr.srr1 = chapa::extract_bits!(ctx.cpu.msr; 0, 5..=9, 16..=31).raw();

    ctx.cpu.msr = ctx
        .cpu
        .msr
        .with_pow(false)
        .with_fp(false)
        .with_be(false)
        .with_dr(false)
        .with_fe1(false)
        .with_pm(false)
        .with_ee(false)
        .with_fe0(false)
        .with_ri(false)
        .with_pr(false)
        .with_se(false)
        .with_ir(false)
        .with_le(ctx.cpu.msr.ile());

    ctx.cpu.nia = base | irq::IRQ_SYSTEM_CALL;
}
