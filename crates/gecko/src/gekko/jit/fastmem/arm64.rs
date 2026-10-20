use super::Registers;
use yaxpeax_arch::{Decoder, U8Reader};
use yaxpeax_arm::armv8::a64::{InstDecoder, Opcode, Operand, ShiftStyle, SizeCode};

const ZERO_REGISTER: u16 = 31;
const INSTRUCTION_BYTES: u64 = 4;

struct Access {
    bytes: usize,
    store: bool,
    sign_extend: bool,
}

pub(super) fn emulate(
    instruction: u32,
    registers: &mut Registers,
    memory: &mut impl FnMut(u64, &mut [u8], bool) -> bool,
) -> Result<bool, &'static str> {
    let encoded = instruction.to_le_bytes();
    let instruction = InstDecoder::default()
        .decode(&mut U8Reader::new(&encoded))
        .map_err(|_| "undecodable ARM instruction")?;

    let Operand::Register(register_size, rt) = instruction.operands[0] else {
        return Err("expected an integer register load or store");
    };
    let access = access(instruction.opcode, register_size).ok_or("unsupported ARM load/store operation")?;
    let address = address(registers, instruction.operands[1])?;

    let mut bytes = if access.store {
        read_register(registers, rt).to_le_bytes()
    } else {
        [0; 8]
    };
    if !memory(address, &mut bytes[..access.bytes], access.store) {
        return Ok(false);
    }

    if !access.store {
        let mut value = u64::from_le_bytes(bytes);
        if access.sign_extend {
            value = sign_extend(value, access.bytes);
        }
        if register_size == SizeCode::W {
            value = value as u32 as u64;
        }
        write_register(registers, rt, value);
    }

    registers.pc += INSTRUCTION_BYTES;
    Ok(true)
}

fn access(opcode: Opcode, register_size: SizeCode) -> Option<Access> {
    let register_bytes = match register_size {
        SizeCode::W => 4,
        SizeCode::X => 8,
    };

    let (bytes, store, sign_extend) = match opcode {
        Opcode::STRB | Opcode::STURB => (1, true, false),
        Opcode::STRH | Opcode::STURH => (2, true, false),
        Opcode::STR | Opcode::STUR => (register_bytes, true, false),

        Opcode::LDRB | Opcode::LDURB => (1, false, false),
        Opcode::LDRH | Opcode::LDURH => (2, false, false),
        Opcode::LDR | Opcode::LDUR => (register_bytes, false, false),

        Opcode::LDRSB | Opcode::LDURSB => (1, false, true),
        Opcode::LDRSH | Opcode::LDURSH => (2, false, true),
        Opcode::LDRSW | Opcode::LDURSW => (4, false, true),

        _ => return None,
    };

    Some(Access {
        bytes,
        store,
        sign_extend,
    })
}

fn address(registers: &Registers, operand: Operand) -> Result<u64, &'static str> {
    match operand {
        Operand::RegPreIndex(base, offset, false) => {
            Ok(registers.gpr[base as usize].wrapping_add(offset as i64 as u64))
        }
        Operand::RegRegOffset(base, index, index_size, extend, shift) => {
            let index = read_register(registers, index);
            let index = match (extend, index_size) {
                (ShiftStyle::UXTW, SizeCode::W) => index as u32 as u64,
                (ShiftStyle::SXTW, SizeCode::W) => index as u32 as i32 as i64 as u64,
                (ShiftStyle::LSL | ShiftStyle::SXTX, SizeCode::X) => index,
                _ => return Err("unsupported ARM address extension"),
            };

            Ok(registers.gpr[base as usize].wrapping_add(index << shift))
        }
        _ => Err("unsupported ARM memory addressing mode"),
    }
}

fn read_register(registers: &Registers, register: u16) -> u64 {
    if register == ZERO_REGISTER {
        0
    } else {
        registers.gpr[register as usize]
    }
}

fn write_register(registers: &mut Registers, register: u16, value: u64) {
    if register != ZERO_REGISTER {
        registers.gpr[register as usize] = value;
    }
}

fn sign_extend(value: u64, bytes: usize) -> u64 {
    match bytes {
        1 => value as i8 as u64,
        2 => value as i16 as u64,
        4 => value as i32 as u64,
        _ => value,
    }
}
