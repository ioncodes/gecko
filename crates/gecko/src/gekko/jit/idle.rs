use crate::gekko::jit::block::{BlockSpec, TermKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleClass {
    None,
    BranchToSelf,
    PollingLoop,
    PointerIterLoop { gpr: u8, stride: i32 },
}

pub fn classify<const SYSTEM: crate::system::SystemId>(spec: &BlockSpec, gprs: &[u32; 32]) -> IdleClass {
    let _ = gprs;

    if let Some(class) = classify_branch_to_self(spec) {
        return class;
    }

    if let Some(p) = self::classify_pointer_iter_loop(spec) {
        return p;
    }

    if classify_polling_loop(spec) {
        return IdleClass::PollingLoop;
    }

    IdleClass::None
}

fn classify_pointer_iter_loop(spec: &BlockSpec) -> Option<IdleClass> {
    if spec.terminator != TermKind::BranchCond || spec.instrs.len() != 3 {
        return None;
    }

    let i0 = crate::gekko::instruction::Instruction(spec.instrs[0]);
    let i1 = crate::gekko::instruction::Instruction(spec.instrs[1]);
    let i2 = crate::gekko::instruction::Instruction(spec.instrs[2]);

    if !self::is_cache_op_no_side_effect(i0) {
        return None;
    }

    if i1.primary_opcode() != 14 {
        return None;
    }
    let addi_ra = i1.ra();
    let addi_rd = i1.rd();
    if addi_ra == 0 || addi_ra != addi_rd {
        return None;
    }
    let stride = i1.simm();

    if i2.primary_opcode() != 16 {
        return None;
    }
    if i2.lk() {
        return None;
    }
    let bo = i2.bo();
    if bo & 0b10000 == 0 {
        return None;
    }
    if bo & 0b00100 != 0 {
        return None;
    }
    if bo & 0b00010 != 0 {
        return None;
    }
    let branch_pc = spec.start_pc.wrapping_add(8);
    let target = if i2.aa() {
        i2.bd() as u32
    } else {
        branch_pc.wrapping_add_signed(i2.bd())
    };
    if target != spec.start_pc {
        return None;
    }

    Some(IdleClass::PointerIterLoop {
        gpr: addi_ra,
        stride: stride as i32,
    })
}

fn is_cache_op_no_side_effect(instr: crate::gekko::instruction::Instruction) -> bool {
    if instr.primary_opcode() != 31 {
        return false;
    }
    matches!(instr.xo10(), 86 | 470 | 54 | 278 | 246 | 1014 | 982 | 758)
}

fn classify_branch_to_self(spec: &BlockSpec) -> Option<IdleClass> {
    if spec.terminator != TermKind::Branch || spec.instrs.len() != 1 {
        return None;
    }

    let instr = crate::gekko::instruction::Instruction(spec.instrs[0]);
    if instr.lk() {
        return None;
    }

    let li = instr.li();
    let target = if instr.aa() {
        li as u32
    } else {
        spec.start_pc.wrapping_add_signed(li)
    };

    if target == spec.start_pc {
        Some(IdleClass::BranchToSelf)
    } else {
        None
    }
}

fn classify_polling_loop(spec: &BlockSpec) -> bool {
    const MAX_IDLE_BODY: usize = 6;

    if spec.terminator != TermKind::BranchCond {
        return false;
    }

    let last_idx = match spec.instrs.len().checked_sub(1) {
        Some(i) if i > 0 => i,
        _ => return false,
    };

    if last_idx > MAX_IDLE_BODY {
        return false;
    }

    let term = crate::gekko::instruction::Instruction(spec.instrs[last_idx]);
    let term_pc = spec.start_pc.wrapping_add((last_idx as u32) * 4);
    if !is_idle_loop_terminator(term, term_pc, spec.start_pc) {
        return false;
    }

    validate_idle_loop(&spec.instrs[..last_idx])
}

fn is_idle_loop_terminator(instr: crate::gekko::instruction::Instruction, branch_pc: u32, block_start_pc: u32) -> bool {
    if instr.primary_opcode() != 16 {
        return false;
    }

    if instr.lk() {
        return false;
    }

    if instr.bo() & 0b00100 == 0 {
        return false;
    }

    let target = if instr.aa() {
        instr.bd() as u32
    } else {
        branch_pc.wrapping_add_signed(instr.bd())
    };

    target == block_start_pc
}

fn validate_idle_loop(body: &[u32]) -> bool {
    let mut write_disallowed: u32 = 0;
    let mut written: u32 = 0;

    for &raw in body {
        let (reads, writes) = match gpr_dataflow(crate::gekko::instruction::Instruction(raw)) {
            Some(p) => p,
            None => return false,
        };

        let externals = reads & !written;
        write_disallowed |= externals;
        if writes & write_disallowed != 0 {
            return false;
        }

        written |= writes;
    }

    true
}

fn gpr_dataflow(instr: crate::gekko::instruction::Instruction) -> Option<(u32, u32)> {
    let rd_or_s = instr.rd() as u32;
    let ra = instr.ra() as u32;
    let rb = instr.rb() as u32;
    let bit = |r: u32| 1u32 << r;
    let read_a_or_zero = if ra == 0 { 0 } else { bit(ra) };

    Some(match instr.primary_opcode() {
        14 | 15 => (read_a_or_zero, bit(rd_or_s)),
        7 | 8 | 12 | 13 => (bit(ra), bit(rd_or_s)),
        10 | 11 => (bit(ra), 0),
        24 | 25 | 26 | 27 | 28 | 29 => (bit(rd_or_s), bit(ra)),
        20 => (bit(rd_or_s) | bit(ra), bit(ra)),
        21 => (bit(rd_or_s), bit(ra)),
        23 => (bit(rd_or_s) | bit(rb), bit(ra)),
        32 | 34 | 40 | 42 => (read_a_or_zero, bit(rd_or_s)),
        33 | 35 | 41 | 43 => (bit(ra), bit(rd_or_s) | bit(ra)),
        31 => return xform_dataflow(instr),
        _ => return None,
    })
}

fn xform_dataflow(instr: crate::gekko::instruction::Instruction) -> Option<(u32, u32)> {
    let rd_or_s = instr.rd() as u32;
    let ra = instr.ra() as u32;
    let rb = instr.rb() as u32;
    let bit = |r: u32| 1u32 << r;
    let read_a_or_zero = if ra == 0 { 0 } else { bit(ra) };

    Some(match instr.xo10() {
        266 | 40 | 10 | 138 | 202 | 234 | 8 | 136 | 200 | 232 | 104 | 235 | 75 | 11 | 491 | 459 | 778 | 552 | 522
        | 650 | 714 | 746 | 520 | 648 | 712 | 744 | 616 | 747 | 1003 | 971 => (bit(ra) | bit(rb), bit(rd_or_s)),
        28 | 60 | 124 | 284 | 316 | 412 | 444 | 476 => (bit(rd_or_s) | bit(rb), bit(ra)),
        26 | 922 | 954 => (bit(rd_or_s), bit(ra)),
        24 | 536 | 792 => (bit(rd_or_s) | bit(rb), bit(ra)),
        824 => (bit(rd_or_s), bit(ra)),
        0 | 32 => (bit(ra) | bit(rb), 0),
        23 | 87 | 279 | 343 | 534 | 790 => (read_a_or_zero | bit(rb), bit(rd_or_s)),
        55 | 119 | 311 | 375 => (bit(ra) | bit(rb), bit(rd_or_s) | bit(ra)),
        _ => return None,
    })
}
