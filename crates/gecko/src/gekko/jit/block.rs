use crate::gekko::instruction::Instruction;
use crate::system::{System, SystemId};

pub const MAX_BLOCK_INSTRS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Branch,
    BranchCond,
    BranchToReg,
    SystemCall,
    Rfi,
    Mtmsr,
    Mtspr,
    Isync,
    LengthCap,
}

#[derive(Debug, Clone)]
pub struct BlockSpec {
    pub start_pc: u32,
    pub instrs: Vec<u32>,
    pub pcs: Vec<u32>,
    pub terminator: TermKind,
}

impl BlockSpec {
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.instrs.len()
    }

    #[inline(always)]
    pub fn end_pc(&self) -> u32 {
        self.pcs
            .last()
            .copied()
            .map(|p| p.wrapping_add(4))
            .unwrap_or(self.start_pc)
    }

    #[inline(always)]
    pub fn pc_of(&self, i: usize) -> u32 {
        self.pcs[i]
    }
}

pub fn discover<const SYSTEM: SystemId>(sys: &System<SYSTEM>, start_pc: u32) -> BlockSpec {
    const EXTENSION_MAX_FORWARD_BYTES: u32 = 1024;

    let mut instrs: Vec<u32> = Vec::with_capacity(8);
    let mut pcs: Vec<u32> = Vec::with_capacity(8);
    let mut terminator = TermKind::LengthCap;
    let mut pc = start_pc;

    while instrs.len() < MAX_BLOCK_INSTRS {
        let instr = Instruction(sys.mmio.fetch_instruction(pc));
        let cur_pc = pc;

        if let Some(target) = extension_target(instr, cur_pc) {
            if target > cur_pc && target.wrapping_sub(cur_pc) <= EXTENSION_MAX_FORWARD_BYTES && !pcs.contains(&target) {
                pc = target;
                continue;
            }
        }

        instrs.push(instr.0);
        pcs.push(cur_pc);

        if let Some(t) = classify_terminator(instr) {
            terminator = t;
            break;
        }
        pc = pc.wrapping_add(4);
    }

    BlockSpec {
        start_pc,
        instrs,
        pcs,
        terminator,
    }
}

#[inline]
fn extension_target(instr: Instruction, pc: u32) -> Option<u32> {
    if instr.primary_opcode() != 18 {
        return None;
    }
    if instr.lk() {
        return None;
    }
    let target = if instr.aa() {
        instr.li() as u32
    } else {
        pc.wrapping_add_signed(instr.li())
    };
    Some(target)
}

#[inline]
fn mtspr_is_block_safe(spr: u16) -> bool {
    matches!(
        spr,
        1
        | 8
        | 9
        | 22
        | 26 | 27
        | 272..=275
        | 912..=919
        | 920
        | 1008 | 1009
    )
}

#[inline]
pub fn classify_terminator(instr: Instruction) -> Option<TermKind> {
    match instr.primary_opcode() {
        16 => Some(TermKind::BranchCond),
        17 => Some(TermKind::SystemCall),
        18 => Some(TermKind::Branch),
        19 => match instr.xo10() {
            16 | 528 => Some(TermKind::BranchToReg),
            50 => Some(TermKind::Rfi),
            _ => None,
        },
        31 => match instr.xo10() {
            467 => {
                let spr_num = instr.spr_swapped() as u16;
                if mtspr_is_block_safe(spr_num) {
                    None
                } else {
                    Some(TermKind::Mtspr)
                }
            }
            _ => None,
        },
        _ => None,
    }
}
