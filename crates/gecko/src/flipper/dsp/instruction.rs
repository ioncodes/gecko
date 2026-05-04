include!(concat!(env!("OUT_DIR"), "/dsp_instr.rs"));
include!(concat!(env!("OUT_DIR"), "/dsp_ext_instr.rs"));

impl Instruction {
    /// Extract the extension opcode byte from the instruction, if present.
    /// Returns `Some(ext)` for instructions with upper nibble >= 3, `None` otherwise.
    #[inline(always)]
    pub fn ext_opcode(&self) -> Option<u8> {
        let nibble = (self.0 >> 12) & 0xF;
        if nibble >= 3 {
            Some(if nibble == 3 { self.ext_9_15() } else { self.ext_8_15() })
        } else {
            None
        }
    }
}
