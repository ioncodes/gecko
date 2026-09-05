pub const MAX_BLOCK_INSTRS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Halt,
    Jump,
    Call,
    Ret,
    IfCc,
    LoopSetup,
    LengthLimit,
}

#[derive(Debug, Clone)]
pub struct InstrEntry {
    pub pc: u16,
    pub raw: u32,
    pub size: u8,
}

#[derive(Debug, Clone)]
pub struct BlockSpec {
    pub start_pc: u16,
    pub instrs: Vec<InstrEntry>,
    pub terminator: TermKind,
    pub fallthrough_pc: u16,
    pub unrolled_loop_start: Option<usize>,
}

fn read_imem_word(iram: &[u8], irom: &[u8], addr: u16) -> Option<u16> {
    let off = (addr & 0x0FFF) as usize * 2;
    match addr & 0xF000 {
        0x0000 => Some(u16::from_be_bytes([iram[off], iram[off + 1]])),
        0x8000 => Some(u16::from_be_bytes([irom[off], irom[off + 1]])),
        _ => None,
    }
}

pub fn is_terminator_word(primary: u16) -> bool {
    self::classify(primary).is_some()
}

fn classify(primary: u16) -> Option<TermKind> {
    match primary {
        0x0021 => Some(TermKind::Halt),

        0x0040..=0x007F | 0x1000..=0x11FF => Some(TermKind::LoopSetup),

        0x0270..=0x027F => Some(TermKind::IfCc),
        0x0290..=0x029F => Some(TermKind::Jump),
        0x02B0..=0x02BF => Some(TermKind::Call),
        0x02D0..=0x02DF | 0x02F0..=0x02FF => Some(TermKind::Ret),
        _ if (primary & 0xFF10) == 0x1700 => Some(TermKind::Jump),
        _ if (primary & 0xFF10) == 0x1710 => Some(TermKind::Call),

        0x0000 | 0x0004..=0x001F => None,
        0x0080..=0x009F | 0x00C0..=0x00FF => None,
        0x02CA | 0x02CB => None,
        0x0400..=0x0FFF => None,
        0x1200..=0x1207 | 0x1300..=0x1307 => None,
        0x1400..=0x16FF | 0x1800..=0x1FFF => None,
        _ if (primary & 0xFEFF) == 0x0200 => None,
        _ if (primary & 0xFEFF) == 0x0220 => None,
        _ if (primary & 0xFEFF) == 0x0240 => None,
        _ if (primary & 0xFEFF) == 0x0260 => None,
        _ if (primary & 0xFEFF) == 0x0280 => None,
        _ if (primary & 0xFEFF) == 0x02A0 => None,
        _ if (primary & 0xFEFF) == 0x02C0 => None,
        _ if (primary & 0xFEF0) == 0x0210 => None,

        _ if (primary >> 12) <= 0x1 => Some(TermKind::Jump),

        _ => None,
    }
}

fn approx_size(primary: u16) -> u8 {
    use crate::flipper::dsp::instruction::Instruction;
    use crate::flipper::dsp::lut;
    lut::instr_size(Instruction(primary as u32)) as u8
}

pub fn discover(iram: &[u8], irom: &[u8], start_pc: u16) -> BlockSpec {
    let mut instrs = Vec::with_capacity(8);
    let mut pc = start_pc;
    let mut term = TermKind::LengthLimit;

    while instrs.len() < MAX_BLOCK_INSTRS {
        let Some(w0) = read_imem_word(iram, irom, pc) else {
            term = TermKind::LengthLimit;
            break;
        };

        let size = approx_size(w0);
        let raw = if size == 2 {
            let w1 = read_imem_word(iram, irom, pc.wrapping_add(1)).unwrap_or(0);
            (w0 as u32) | ((w1 as u32) << 16)
        } else {
            w0 as u32
        };
        instrs.push(InstrEntry { pc, raw, size });

        if let Some(kind) = classify(w0) {
            term = kind;
            pc = pc.wrapping_add(size as u16);
            break;
        }

        pc = pc.wrapping_add(size as u16);
    }

    let mut spec = BlockSpec {
        start_pc,
        instrs,
        terminator: term,
        fallthrough_pc: pc,
        unrolled_loop_start: None,
    };
    try_unroll_immediate_loop(iram, irom, &mut spec);

    spec
}

fn can_unroll_instruction(raw: u32) -> bool {
    use disasm::dsp::GcDspInstruction as I;

    if classify(raw as u16).is_some() {
        return false;
    }

    let bytes = [(raw >> 8) as u8, raw as u8, (raw >> 24) as u8, (raw >> 16) as u8];
    let Some((insn, _)) = I::decode(&bytes) else {
        return false;
    };
    let is_stack_register = |reg: u8| (12..=15).contains(&reg);

    match insn {
        I::Lri { rd, .. } | I::Lr { rd, .. } => !is_stack_register(rd),
        I::Sr { rs, .. } => !is_stack_register(rs),
        I::Lrr { d, .. } | I::Lrrd { d, .. } | I::Lrri { d, .. } | I::Lrrn { d, .. } => !is_stack_register(d),
        I::Srr { s, .. } | I::Srrd { s, .. } | I::Srri { s, .. } | I::Srrn { s, .. } => !is_stack_register(s),
        I::Mrr { dst, src } => !is_stack_register(dst) && !is_stack_register(src),
        _ => true,
    }
}

fn try_unroll_immediate_loop(iram: &[u8], irom: &[u8], spec: &mut BlockSpec) {
    const MAX_EXPANDED_INSTRS: usize = 256;

    if spec.terminator != TermKind::LoopSetup {
        return;
    }

    let Some(setup) = spec.instrs.last() else {
        return;
    };

    let primary = setup.raw as u16;
    if primary & 0xFF00 != 0x1100 {
        return;
    }

    let count = (primary & 0x00FF) as usize;

    if count == 0 {
        return;
    }

    let end_pc = (setup.raw >> 16) as u16;
    let mut body = Vec::new();
    let mut pc = spec.fallthrough_pc;

    loop {
        let Some(w0) = read_imem_word(iram, irom, pc) else {
            return;
        };

        let size = approx_size(w0);
        let raw = if size == 2 {
            let Some(w1) = read_imem_word(iram, irom, pc.wrapping_add(1)) else {
                return;
            };
            (w0 as u32) | ((w1 as u32) << 16)
        } else {
            w0 as u32
        };

        if !can_unroll_instruction(raw) {
            return;
        }

        body.push(InstrEntry { pc, raw, size });

        let last_word = pc.wrapping_add(size as u16 - 1);

        if last_word == end_pc {
            break;
        }

        let next = pc.wrapping_add(size as u16);

        if next <= pc || next > end_pc || body.len() > MAX_EXPANDED_INSTRS {
            return;
        }
        pc = next;
    }

    let prefix_len = spec.instrs.len() - 1;
    let Some(expanded_len) = body.len().checked_mul(count).and_then(|n| n.checked_add(prefix_len)) else {
        return;
    };

    if expanded_len > MAX_EXPANDED_INSTRS {
        return;
    }

    spec.instrs.pop();
    spec.instrs.reserve(body.len() * count);

    for _ in 0..count {
        spec.instrs.extend(body.iter().cloned());
    }

    spec.terminator = TermKind::LengthLimit;
    spec.fallthrough_pc = end_pc.wrapping_add(1);
    spec.unrolled_loop_start = Some(prefix_len);
}
