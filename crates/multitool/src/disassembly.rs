use disasm::dsp::GcDspInstruction;
use disasm::gekko::GekkoInstruction;

pub fn disassemble_ppc(data: &[u8], start: usize) {
    let mut offset = start;
    while offset + 4 <= data.len() {
        let word = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
        let addr = offset as u32;

        match GekkoInstruction::decode(&data[offset..]) {
            Some((instr, _)) => println!("{:08X}  {:08X}  {}", addr, word, instr),
            None => println!("{:08X}  {:08X}  .long {:#010x}", addr, word, word),
        }

        offset += 4;
    }
}

pub fn disassemble_dsp(data: &[u8], start: usize) {
    // start is a word address; convert to byte offset
    let mut offset = start * 2;
    while offset + 2 <= data.len() {
        let word = u16::from_be_bytes(data[offset..offset + 2].try_into().unwrap());
        let addr = (offset / 2) as u32;

        match GcDspInstruction::decode(&data[offset..]) {
            Some((instr, bytes_consumed)) => {
                let hex_parts: Vec<_> = data[offset..offset + bytes_consumed]
                    .chunks_exact(2)
                    .map(|c| format!("{:04x}", u16::from_be_bytes(c.try_into().unwrap())))
                    .collect();
                println!("{:04x}  {:9}  {}", addr, hex_parts.join(" "), instr);
                offset += bytes_consumed;
            }
            None => {
                println!("{:04x}  {:04x}      .word  {:#06x}", addr, word, word);
                offset += 2;
            }
        }
    }
}
