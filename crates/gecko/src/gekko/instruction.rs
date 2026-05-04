include!(concat!(env!("OUT_DIR"), "/gekko_instr.rs"));

impl Instruction {
    #[inline(always)]
    pub fn disp(&self) -> i32 {
        self.d_16_31()
    }

    #[inline(always)]
    pub fn disp_psq(&self) -> i32 {
        self.d_20_31()
    }

    // D-form PSQ fields (psq_l, psq_lu, psq_st, psq_stu)
    #[inline(always)]
    pub fn psq_w(&self) -> bool {
        self.w_16_16()
    }

    #[inline(always)]
    pub fn psq_i(&self) -> u8 {
        self.i_17_19()
    }

    // X-form PSQ fields (psq_lx, psq_lux, psq_stx, psq_stux)
    #[inline(always)]
    pub fn psq_wx(&self) -> bool {
        self.w_21_21()
    }

    #[inline(always)]
    pub fn psq_ix(&self) -> u8 {
        self.i_22_24()
    }

    // The SPR field in PowerPC instructions has a special encoding where
    // the two 5-bit halves are swapped. This method returns the decoded value
    #[inline(always)]
    pub fn spr_swapped(&self) -> u32 {
        let raw = self.spr() as u32;
        (raw >> 5) | ((raw & 0x1f) << 5)
    }
}
