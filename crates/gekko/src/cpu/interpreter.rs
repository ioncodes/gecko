pub fn branch<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    if instr.lk() {
        ctx.cpu.write_gpr(0, ctx.cpu.current_pc.wrapping_add(4));
    }

    let target = if instr.aa() {
        instr.li() as u32
    } else {
        ctx.cpu.current_pc.wrapping_add_signed(instr.li())
    };
    ctx.cpu.next_pc = target;
}

pub fn alu<const OP: u32>(ctx: &mut crate::gekko::Gekko, instr: crate::cpu::semantics::Instruction) {
    match OP {
        crate::cpu::lut::OP_ADDI => {
            let ra = if instr.ra() == 0 { 0 } else { ctx.cpu.gprs[instr.ra()] };
            ctx.cpu.gprs[instr.rd()] = ra.wrapping_add_signed(instr.simm());
        }
        crate::cpu::lut::OP_ADDIS => {
            let ra = if instr.ra() == 0 { 0 } else { ctx.cpu.gprs[instr.ra()] };
            ctx.cpu.gprs[instr.rd()] = ra.wrapping_add_signed(instr.simm() << 16);
        }
        crate::cpu::lut::OP_ORI => {
            ctx.cpu.gprs[instr.ra()] = ctx.cpu.gprs[instr.rs()] | instr.uimm();
        }
        crate::cpu::lut::OP_ORIS => {
            ctx.cpu.gprs[instr.ra()] = ctx.cpu.gprs[instr.rs()] | (instr.uimm() << 16);
        }
        _ => todo!("ALU instruction with OP = {OP:#x}")
    }
}

pub fn twi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("twi") }
pub fn ps_cmpu0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpu0") }
pub fn ps_cmpo0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpo0") }
pub fn ps_cmpu1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpu1") }
pub fn ps_cmpo1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpo1") }
pub fn ps_res(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_res") }
pub fn ps_rsqrte(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_rsqrte") }
pub fn ps_neg(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_neg") }
pub fn ps_mr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_mr") }
pub fn ps_nabs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_nabs") }
pub fn ps_abs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_abs") }
pub fn ps_merge00(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge00") }
pub fn ps_merge01(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge01") }
pub fn ps_merge10(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge10") }
pub fn ps_merge11(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge11") }
pub fn psq_lx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_lx") }
pub fn psq_stx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_stx") }
pub fn psq_lux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_lux") }
pub fn psq_stux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_stux") }
pub fn ps_sum0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sum0") }
pub fn ps_sum1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sum1") }
pub fn ps_muls0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_muls0") }
pub fn ps_muls1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_muls1") }
pub fn ps_madds0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_madds0") }
pub fn ps_madds1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_madds1") }
pub fn ps_div(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_div") }
pub fn ps_sub(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sub") }
pub fn ps_add(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_add") }
pub fn ps_sel(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sel") }
pub fn ps_mul(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_mul") }
pub fn ps_msub(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_msub") }
pub fn ps_madd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_madd") }
pub fn ps_nmsub(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_nmsub") }
pub fn ps_nmadd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_nmadd") }
pub fn dcbz_l(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbz_l") }
pub fn mulli(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mulli") }
pub fn subfic(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfic") }
pub fn cmpli(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cmpli") }
pub fn cmpi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cmpi") }
pub fn addic(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addic") }
pub fn addic_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addic_dot") }
pub fn bcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("bcx") }
pub fn sc(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sc") }
pub fn mcrf(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mcrf") }
pub fn bclrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("bclrx") }
pub fn crnor(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crnor") }
pub fn rfi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("rfi") }
pub fn crandc(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crandc") }
pub fn isync(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("isync") }
pub fn crxor(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crxor") }
pub fn crnand(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crnand") }
pub fn crand(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crand") }
pub fn creqv(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("creqv") }
pub fn crorc(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crorc") }
pub fn cror(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cror") }
pub fn bcctrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("bcctrx") }
pub fn rlwimix(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("rlwimix") }
pub fn rlwinmx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("rlwinmx") }
pub fn rlwnmx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("rlwnmx") }
pub fn ori(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ori") }
pub fn oris(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("oris") }
pub fn xori(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("xori") }
pub fn xoris(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("xoris") }
pub fn andi_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andi_dot") }
pub fn andis_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andis_dot") }
pub fn cmp(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cmp") }
pub fn tw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tw") }
pub fn subfcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfcx") }
pub fn mulhwux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mulhwux") }
pub fn addcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addcx") }
pub fn lwarx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwarx") }
pub fn lwzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwzx") }
pub fn slwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("slwx") }
pub fn cntlzwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cntlzwx") }
pub fn andx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andx") }
pub fn cmpl(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cmpl") }
pub fn subfx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfx") }
pub fn dcbst(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbst") }
pub fn lwzux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwzux") }
pub fn andcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andcx") }
pub fn mulhwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mulhwx") }
pub fn mfcr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfcr") }
pub fn mfmsr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfmsr") }
pub fn dcbf(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbf") }
pub fn lbzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbzx") }
pub fn negx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("negx") }
pub fn lbzux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbzux") }
pub fn norx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("norx") }
pub fn subfex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfex") }
pub fn addex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addex") }
pub fn mtcrf(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtcrf") }
pub fn mtmsr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtmsr") }
pub fn stwcx_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwcx_dot") }
pub fn stwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwx") }
pub fn stwux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwux") }
pub fn subfzex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfzex") }
pub fn addzex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addzex") }
pub fn mtsr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtsr") }
pub fn stbx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stbx") }
pub fn subfmex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfmex") }
pub fn addmex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addmex") }
pub fn mullwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mullwx") }
pub fn mtsrin(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtsrin") }
pub fn dcbtst(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbtst") }
pub fn stbux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stbux") }
pub fn addx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addx") }
pub fn dcbt(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbt") }
pub fn lhzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhzx") }
pub fn eqvx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("eqvx") }
pub fn tlbie(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbie") }
pub fn eciwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("eciwx") }
pub fn lhzux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhzux") }
pub fn xorx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("xorx") }
pub fn mfspr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfspr") }
pub fn lhax(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhax") }
pub fn tlbia(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbia") }
pub fn mftb(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mftb") }
pub fn lhaux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhaux") }
pub fn sthx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sthx") }
pub fn orcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("orcx") }
pub fn ecowx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ecowx") }
pub fn sthux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sthux") }
pub fn orx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("orx") }
pub fn divwux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("divwux") }
pub fn mtspr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtspr") }
pub fn dcbi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbi") }
pub fn nandx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("nandx") }
pub fn divwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("divwx") }
pub fn mcrxr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mcrxr") }
pub fn lwbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwbrx") }
pub fn lfsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfsx") }
pub fn srwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("srwx") }
pub fn lswx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lswx") }
pub fn tlbsync(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbsync") }
pub fn lfsux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfsux") }
pub fn mfsr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfsr") }
pub fn lswi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lswi") }
pub fn sync(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sync") }
pub fn lfdx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfdx") }
pub fn lfdux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfdux") }
pub fn mfsrin(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfsrin") }
pub fn stswx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stswx") }
pub fn stwbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwbrx") }
pub fn stfsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfsx") }
pub fn stfsux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfsux") }
pub fn stswi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stswi") }
pub fn stfdx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfdx") }
pub fn stfdux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfdux") }
pub fn dcba(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcba") }
pub fn lhbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhbrx") }
pub fn srawx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("srawx") }
pub fn srawix(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("srawix") }
pub fn eieio(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("eieio") }
pub fn sthbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sthbrx") }
pub fn extshx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("extshx") }
pub fn extsbx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("extsbx") }
pub fn icbi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("icbi") }
pub fn stfiwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfiwx") }
pub fn tlbld(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbld") }
pub fn tlbli(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbli") }
pub fn dcbz(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbz") }
pub fn lwz(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwz") }
pub fn lwzu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwzu") }
pub fn lbz(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbz") }
pub fn lbzu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbzu") }
pub fn stw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stw") }
pub fn stwu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwu") }
pub fn stb(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stb") }
pub fn stbu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stbu") }
pub fn lhz(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhz") }
pub fn lhzu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhzu") }
pub fn lha(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lha") }
pub fn lhau(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhau") }
pub fn sth(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sth") }
pub fn sthu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sthu") }
pub fn lmw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lmw") }
pub fn stmw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stmw") }
pub fn lfs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfs") }
pub fn lfsu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfsu") }
pub fn lfd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfd") }
pub fn lfdu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfdu") }
pub fn stfs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfs") }
pub fn stfsu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfsu") }
pub fn stfd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfd") }
pub fn stfdu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfdu") }
pub fn psq_l(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_l") }
pub fn psq_lu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_lu") }
pub fn fdivsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fdivsx") }
pub fn fsubsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsubsx") }
pub fn faddsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("faddsx") }
pub fn fsqrtsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsqrtsx") }
pub fn fresx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fresx") }
pub fn fmulsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmulsx") }
pub fn fmsubsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmsubsx") }
pub fn fmaddsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmaddsx") }
pub fn fnmsubsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmsubsx") }
pub fn fnmaddsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmaddsx") }
pub fn psq_st(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_st") }
pub fn psq_stu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_stu") }
pub fn fcmpu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fcmpu") }
pub fn frspx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("frspx") }
pub fn fctiwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fctiwx") }
pub fn fctiwzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fctiwzx") }
pub fn fcmpo(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fcmpo") }
pub fn mtfsb1x(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsb1x") }
pub fn fnegx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnegx") }
pub fn mcrfs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mcrfs") }
pub fn mtfsb0x(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsb0x") }
pub fn fmrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmrx") }
pub fn mtfsfix(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsfix") }
pub fn fnabsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnabsx") }
pub fn fabsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fabsx") }
pub fn mffsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mffsx") }
pub fn mtfsfx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsfx") }
pub fn fdivx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fdivx") }
pub fn fsubx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsubx") }
pub fn faddx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("faddx") }
pub fn fsqrtx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsqrtx") }
pub fn fselx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fselx") }
pub fn fmulx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmulx") }
pub fn frsqrtex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("frsqrtex") }
pub fn fmsubx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmsubx") }
pub fn fmaddx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmaddx") }
pub fn fnmsubx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmsubx") }
pub fn fnmaddx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmaddx") }
