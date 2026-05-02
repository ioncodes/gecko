use crate::gekko::instruction::Instruction;
use crate::gekko::lut::*;
use crate::gekko::sr::Sr;
use crate::system::{System, SystemId};

#[inline(always)]
pub fn msr<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    match OP {
        OP_MTMSR => {
            ctx.gekko.msr = crate::gekko::msr::Msr::from(ctx.gekko.read_gpr(instr.rs()));
        }
        OP_MFMSR => {
            ctx.gekko.write_gpr(instr.rd(), ctx.gekko.msr.raw());
        }
        OP_RFI => {
            const RFI_MSR_MASK: u32 = 0x87C0_FFFF;
            let msr = (ctx.gekko.msr.raw() & !RFI_MSR_MASK) | (ctx.gekko.spr.srr1 & RFI_MSR_MASK);
            ctx.gekko.msr = crate::gekko::msr::Msr::from(msr & !0x0004_0000);
            ctx.gekko.nia = ctx.gekko.spr.srr0.value() << 2;
        }
        _ => todo!("MSR instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn spr<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    match OP {
        OP_MTSPR => {
            let spr_num = instr.spr_swapped();
            let val = ctx.gekko.read_gpr(instr.rs());
            match spr_num {
                22 => {
                    ctx.scheduler.cancel(crate::gekko::dec::underflow_handler::<SYSTEM>);
                    ctx.gekko.dec.write(ctx.scheduler.cycles, val);
                    ctx.gekko.spr.dec = val;
                    ctx.scheduler.schedule_in(
                        crate::gekko::dec::cycles_until_underflow(val),
                        crate::gekko::dec::underflow_handler::<SYSTEM>,
                    );
                    tracing::debug!(cycles = ctx.scheduler.cycles, value = val, "decrementer set");
                }
                284 => ctx.scheduler.set_timebase_lower(val),
                285 => ctx.scheduler.set_timebase_upper(val),
                923 => {
                    ctx.gekko.spr.dmal = crate::gekko::spr::DmaLower::from_raw(val);
                    if ctx.gekko.spr.dmal.trigger() {
                        ctx.mmio
                            .process_locked_cache_dma(&ctx.gekko.spr.dmau, &ctx.gekko.spr.dmal);
                    }
                }
                _ => ctx.gekko.spr.write(spr_num, val),
            }
        }
        OP_MFSPR => {
            let spr_num = instr.spr_swapped();
            let val = match spr_num {
                22 => {
                    ctx.gekko.spr.dec = ctx.gekko.dec.read(ctx.scheduler.cycles);
                    ctx.gekko.spr.dec
                }
                268 => ctx.scheduler.timebase_lower(),
                269 => ctx.scheduler.timebase_upper(),
                _ => ctx.gekko.spr.read(spr_num),
            };
            ctx.gekko.write_gpr(instr.rd(), val);
        }
        _ => todo!("SPR instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn segment<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    match OP {
        OP_MTSR => {
            ctx.gekko.sr[instr.sr() as usize] = Sr::from_raw(ctx.gekko.read_gpr(instr.rs()));
        }
        OP_MFSR => {
            ctx.gekko.write_gpr(instr.rd(), ctx.gekko.sr[instr.sr() as usize].raw());
        }
        _ => todo!("Segment Register instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn mtsrin<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    let sr_idx = (ctx.gekko.read_gpr(instr.rb()) >> 28) as usize;
    ctx.gekko.sr[sr_idx] = Sr::from_raw(ctx.gekko.read_gpr(instr.rs()));
}

#[inline(always)]
pub fn mfsrin<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    let sr_idx = (ctx.gekko.read_gpr(instr.rb()) >> 28) as usize;
    ctx.gekko.write_gpr(instr.rd(), ctx.gekko.sr[sr_idx].raw());
}

#[inline(always)]
pub fn mftb<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    let tbr = instr.spr_swapped();
    let val = match tbr {
        268 => ctx.scheduler.timebase_lower(),
        269 => ctx.scheduler.timebase_upper(),
        _ => panic!("unknown TBR {tbr}"),
    };
    ctx.gekko.write_gpr(instr.rd(), val);
}

#[inline(always)]
pub fn twi<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    let a = ctx.gekko.read_gpr(instr.ra()) as i32;
    let simm = instr.simm();
    let to = instr.to();

    let trap = (to & 0x10 != 0 && a < simm)
        || (to & 0x08 != 0 && a > simm)
        || (to & 0x04 != 0 && a == simm)
        || (to & 0x02 != 0 && (a as u32) < (simm as u32))
        || (to & 0x01 != 0 && (a as u32) > (simm as u32));

    if trap {
        ctx.cause_trap_exception();
    }
}

#[inline(always)]
pub fn tw<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    let a = ctx.gekko.read_gpr(instr.ra()) as i32;
    let b = ctx.gekko.read_gpr(instr.rb()) as i32;
    let to = instr.to();

    let trap = (to & 0x10 != 0 && a < b)
        || (to & 0x08 != 0 && a > b)
        || (to & 0x04 != 0 && a == b)
        || (to & 0x02 != 0 && (a as u32) < (b as u32))
        || (to & 0x01 != 0 && (a as u32) > (b as u32));

    if trap {
        ctx.cause_trap_exception();
    }
}

#[inline(always)]
pub fn nop<const OP: u32, const SYSTEM: SystemId>(_ctx: &mut System<SYSTEM>, _instr: Instruction) {}

#[inline(always)]
pub fn sc<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, _instr: Instruction) {
    ctx.cause_syscall_interrupt();
}
