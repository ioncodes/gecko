use super::Registers;
use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};

pub(super) const MAX_INSTRUCTION_BYTES: usize = 15;

fn register_bytes(register: Register) -> std::ops::Range<usize> {
    let start = match register {
        Register::AH | Register::BH | Register::CH | Register::DH => 1,
        _ => 0,
    };

    start..start + register.size()
}

fn read_register(registers: &Registers, register: Register) -> Option<u64> {
    if !register.is_gpr() {
        return None;
    }

    let full = registers.gpr[register.full_register().number()].to_le_bytes();
    let mut value = [0u8; 8];
    value[..register.size()].copy_from_slice(&full[register_bytes(register)]);

    Some(u64::from_le_bytes(value))
}

fn write_register(registers: &mut Registers, register: Register, value: u64) {
    let slot = &mut registers.gpr[register.full_register().number()];

    *slot = match register.size() {
        1 | 2 => {
            let mut full = slot.to_le_bytes();
            full[register_bytes(register)].copy_from_slice(&value.to_le_bytes()[..register.size()]);
            u64::from_le_bytes(full)
        }
        4 => value as u32 as u64,
        8 => value,
        _ => unreachable!(),
    };
}

fn sign_extend(value: u64, bytes: usize) -> u64 {
    match bytes {
        1 => value as i8 as u64,
        2 => value as i16 as u64,
        4 => value as i32 as u64,
        _ => value,
    }
}

pub(super) fn emulate(
    code: &[u8],
    registers: &mut Registers,
    memory: &mut impl FnMut(u64, &mut [u8], bool) -> bool,
) -> Result<bool, &'static str> {
    let mut decoder = Decoder::with_ip(64, code, registers.pc, DecoderOptions::NONE);

    let instruction = decoder.decode();
    let mnemonic = instruction.mnemonic();
    if instruction.is_invalid()
        || instruction.has_lock_prefix()
        || instruction.op_count() != 2
        || !matches!(
            mnemonic,
            Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movsx | Mnemonic::Movsxd | Mnemonic::Movbe
        )
    {
        return Err("expected scalar MOV/MOVZX/MOVSX/MOVSXD/MOVBE");
    }

    let write = instruction.op0_kind() == OpKind::Memory;
    let memory_operand = if write { 0 } else { 1 };
    if instruction.op_kind(memory_operand) != OpKind::Memory {
        return Err("missing memory operand");
    }

    let width = instruction.memory_size().size();
    if !matches!(width, 1 | 2 | 4 | 8) {
        return Err("unsupported memory width");
    }

    let address = instruction
        .virtual_address(memory_operand, 0, |register, _, _| match register {
            Register::DS | Register::ES | Register::CS | Register::SS => Some(0),
            _ => read_register(registers, register),
        })
        .ok_or("unsupported address register or segment")?;

    let mut bytes = [0u8; 8];
    if write {
        if !matches!(mnemonic, Mnemonic::Mov | Mnemonic::Movbe) {
            return Err("invalid store operation");
        }

        let value = match instruction.op1_kind() {
            OpKind::Register => {
                read_register(registers, instruction.op1_register()).ok_or("unsupported store register")?
            }
            OpKind::Immediate8 => instruction.immediate8() as u64,
            OpKind::Immediate16 => instruction.immediate16() as u64,
            OpKind::Immediate32 => instruction.immediate32() as u64,
            OpKind::Immediate32to64 => instruction.immediate32to64() as u64,
            _ => return Err("unsupported store source"),
        };
        bytes.copy_from_slice(&value.to_le_bytes());

        if mnemonic == Mnemonic::Movbe {
            bytes[..width].reverse();
        }

        if !memory(address, &mut bytes[..width], true) {
            return Ok(false);
        }
    } else {
        let target = instruction.op0_register();
        if instruction.op0_kind() != OpKind::Register || !target.is_gpr() {
            return Err("unsupported load destination");
        }

        if !memory(address, &mut bytes[..width], false) {
            return Ok(false);
        }

        if mnemonic == Mnemonic::Movbe {
            bytes[..width].reverse();
        }

        let mut value = u64::from_le_bytes(bytes);
        if matches!(mnemonic, Mnemonic::Movsx | Mnemonic::Movsxd) {
            value = sign_extend(value, width);
        }

        write_register(registers, target, value);
    }

    registers.pc = instruction.next_ip();

    Ok(true)
}
