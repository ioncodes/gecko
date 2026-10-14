use super::WaitCondition;

/// Shared by the scheduler's wait table and JIT block discovery. IRAM uploads
/// rebuild the table and invalidate compiled blocks together.
pub(super) fn window<const N: usize>(read: impl Fn(u16) -> u16, start: u16) -> [u16; N] {
    std::array::from_fn(|i| read(start.wrapping_add(i as u16)))
}

pub(super) fn interrupt_wait_at(pc: u16, read: impl Fn(u16) -> u16) -> WaitCondition {
    let words = window::<9>(&read, pc);
    if let [0x8100, 0x8900, 0x00DF, rhs, 0x00DE, lhs, 0x8200, 0x0293, target] = words
        && lhs < 0x1000
        && rhs < 0x1000
        && target == pc
    {
        return WaitCondition::InterruptComparison { lhs, rhs };
    }

    // CLR $ac0; LR $ac0.m,@flag; TST $ac0; JNZ self; RET.
    // Match only DRAM: reads of IFX/accelerator registers can have side effects.
    if let [0x8100, 0x00DE, addr, 0xB100, 0x0294, target, 0x02DF, ..] = words
        && addr < 0x1000
        && target == pc
    {
        return WaitCondition::InterruptNonzero { addr };
    }

    let (start, head) = if words[0] == 0x029F {
        (words[1], window::<9>(&read, words[1]))
    } else {
        (pc, words)
    };
    let [
        0x00C0,
        pointer_addr,
        0x0088,
        0x002F,
        0x00DA,
        addr,
        0x8600,
        0x0295,
        target,
    ] = head
    else {
        return WaitCondition::None;
    };
    if pointer_addr >= 0x1000 || addr >= 0x1000 {
        return WaitCondition::None;
    }
    let [0x0088, 0xFFFF, 0x029F, back] = window::<4>(&read, target) else {
        return WaitCondition::None;
    };
    if back != start && window::<2>(&read, back) != [0x029F, start] {
        return WaitCondition::None;
    }

    WaitCondition::InterruptZero { addr }
}
