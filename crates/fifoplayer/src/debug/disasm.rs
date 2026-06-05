use super::slice::{CmdKind, Command};
use gecko::flipper::gx::constants::*;
use gecko::flipper::gx::draw::Primitive;
use gecko::flipper::gx::regs::*;

pub fn array_name(i: usize) -> String {
    match i {
        ARRAY_POS => "POS".into(),
        ARRAY_NRM => "NRM".into(),
        ARRAY_CLR0 => "CLR0".into(),
        ARRAY_CLR1 => "CLR1".into(),
        4..=11 => format!("TEX{}", i - 4),
        ARRAY_POS_NRM_MTX => "POS_NRM_MTX".into(),
        ARRAY_NRM_MTX => "NRM_MTX".into(),
        ARRAY_POST_MTX => "POST_MTX".into(),
        ARRAY_LIGHT => "LIGHT".into(),
        _ => format!("ARRAY{i}"),
    }
}

pub fn cp_reg_name(reg: u8) -> String {
    let r = reg as usize;
    match r {
        0x30 => "MATINDEX_A".into(),
        0x40 => "MATINDEX_B".into(),
        VCD_LO_REG => "VCD_LO".into(),
        VCD_HI_REG => "VCD_HI".into(),
        _ if (VATA_REG..VATA_REG + 8).contains(&r) => format!("VAT_A{}", r - VATA_REG),
        _ if (VATB_REG..VATB_REG + 8).contains(&r) => format!("VAT_B{}", r - VATB_REG),
        _ if (VATC_REG..VATC_REG + 8).contains(&r) => format!("VAT_C{}", r - VATC_REG),
        _ if (ARRAY_BASE_REG..ARRAY_BASE_REG + 16).contains(&r) => {
            format!("ARRAY_BASE_{}", self::array_name(r - ARRAY_BASE_REG))
        }
        _ if (ARRAY_STRIDE_REG..ARRAY_STRIDE_REG + 16).contains(&r) => {
            format!("ARRAY_STRIDE_{}", self::array_name(r - ARRAY_STRIDE_REG))
        }
        _ => format!("CP_{r:02X}"),
    }
}

pub fn bp_reg_name(reg: u8) -> String {
    let r = reg as usize;
    match r {
        BP_GEN_MODE => "GEN_MODE".into(),
        BP_IND_MTX_A0..=BP_IND_MTX_C2 => {
            let i = r - BP_IND_MTX_A0;
            format!("IND_MTX_{}{}", ['A', 'B', 'C'][i % 3], i / 3)
        }
        BP_BUMP_IMASK => "BUMP_IMASK".into(),
        _ if (BP_IND_CMD_0..BP_IND_CMD_0 + BP_IND_CMD_COUNT).contains(&r) => {
            format!("IND_CMD{}", r - BP_IND_CMD_0)
        }
        BP_SU_SCIS_TL => "SU_SCIS_TL".into(),
        BP_SU_SCIS_BR => "SU_SCIS_BR".into(),
        0x22 => "SU_LPSIZE".into(),
        BP_RAS1_SS0 => "RAS1_SS0".into(),
        BP_RAS1_SS1 => "RAS1_SS1".into(),
        BP_RAS1_IREF => "RAS1_IREF".into(),
        _ if (BP_RAS1_TREF0..BP_RAS1_TREF0 + BP_RAS1_TREF_COUNT).contains(&r) => {
            format!("RAS1_TREF{}", r - BP_RAS1_TREF0)
        }
        0x30..=0x3F => {
            let i = (r - 0x30) / 2;
            if r % 2 == 0 {
                format!("SU_SSIZE{i}")
            } else {
                format!("SU_TSIZE{i}")
            }
        }
        BP_PE_ZMODE => "PE_ZMODE".into(),
        BP_PE_CMODE0 => "PE_CMODE0(blend)".into(),
        0x42 => "PE_CMODE1".into(),
        BP_PE_ZCOMPARE => "PE_CONTROL".into(),
        0x44 => "FIELD_MASK".into(),
        BP_PE_DONE => "PE_DONE".into(),
        BP_PE_TOKEN => "PE_TOKEN".into(),
        BP_PE_TOKEN_INT => "PE_TOKEN_INT".into(),
        BP_PE_COPY_SRC => "EFB_COPY_SRC".into(),
        BP_PE_COPY_DIMS => "EFB_COPY_DIMS".into(),
        BP_PE_COPY_DST => "EFB_COPY_DST".into(),
        BP_PE_COPY_DST_STRIDE => "EFB_COPY_STRIDE".into(),
        BP_PE_COPY_YSCALE => "COPY_YSCALE".into(),
        BP_PE_COPY_CLEAR_AR => "PE_CLEAR_AR".into(),
        BP_PE_COPY_CLEAR_GB => "PE_CLEAR_GB".into(),
        BP_PE_COPY_CLEAR_Z => "PE_CLEAR_Z".into(),
        BP_PE_COPY_CMD => "PE_COPY_EXECUTE".into(),
        0x53 => "COPY_FILTER0".into(),
        0x54 => "COPY_FILTER1".into(),
        BP_SU_SCIS_OFFSET => "SU_SCIS_OFFSET".into(),
        BP_LOAD_TLUT0 => "TX_LOADTLUT0".into(),
        BP_LOAD_TLUT1 => "TX_LOADTLUT1".into(),
        0x80..=0xBF => {
            let map = (r - 0x80) % 4 + if r >= 0xA0 { 4 } else { 0 };
            let base = if r >= 0xA0 { r - 0x20 } else { r };
            match base & !0x03 {
                BP_TX_SETMODE0_I0 => format!("TX_SETMODE0[{map}]"),
                BP_TX_SETMODE1_I0 => format!("TX_SETMODE1[{map}]"),
                BP_TX_SETIMAGE0_I0 => format!("TX_SETIMAGE0[{map}]"),
                BP_TX_SETIMAGE1_I0 => format!("TX_SETIMAGE1[{map}]"),
                BP_TX_SETIMAGE2_I0 => format!("TX_SETIMAGE2[{map}]"),
                BP_TX_SETIMAGE3_I0 => format!("TX_SETIMAGE3[{map}]"),
                BP_TX_SETTLUT_I0 => format!("TX_SETTLUT[{map}]"),
                _ => format!("BP_{r:02X}"),
            }
        }
        0xC0..=0xDF => {
            let stage = (r - 0xC0) / 2;
            if r % 2 == 0 {
                format!("TEV_COLOR_ENV{stage}")
            } else {
                format!("TEV_ALPHA_ENV{stage}")
            }
        }
        0xE0..=0xE7 => {
            let i = (r - 0xE0) / 2;
            if r % 2 == 0 {
                format!("TEV_REGISTERL{i}")
            } else {
                format!("TEV_REGISTERH{i}")
            }
        }
        0xE8..=0xEE => format!("TEV_FOG_PARAM{}", r - 0xE8),
        0xF2 => "TEV_FOG_COLOR".into(),
        BP_PE_ALPHA_COMPARE => "TEV_ALPHAFUNC".into(),
        BP_TEV_ZTEX1 => "TEV_ZTEX1".into(),
        BP_TEV_ZTEX2 => "TEV_ZTEX2".into(),
        _ if (BP_TEV_KSEL_0..BP_TEV_KSEL_0 + 8).contains(&r) => format!("TEV_KSEL{}", r - BP_TEV_KSEL_0),
        BP_BP_MASK => "BP_MASK".into(),
        _ => format!("BP_{r:02X}"),
    }
}

pub fn xf_addr_name(addr: u16) -> String {
    let a = addr as usize;
    match a {
        0x0000..=0x00FF => format!("POS_TEX_MTX[{a:02X}]"),
        0x0400..=0x045F => format!("NRM_MTX[{:02X}]", a - 0x0400),
        0x0500..=0x05FF => format!("POST_MTX[{:02X}]", a - 0x0500),
        0x0600..=0x07FF => {
            let light = (a - XF_LIGHT_BASE) / XF_LIGHT_STRIDE;
            format!("LIGHT{}[{}]", light, (a - XF_LIGHT_BASE) % XF_LIGHT_STRIDE)
        }
        0x1000 => "XF_ERROR".into(),
        0x1005 => "CLIP_DISABLE".into(),
        0x1008 => "INVTXSPEC".into(),
        0x1009 => "NUMCHAN".into(),
        XF_AMBIENT_COLOR0 => "AMBIENT0".into(),
        XF_AMBIENT_COLOR1 => "AMBIENT1".into(),
        XF_MATERIAL_COLOR0 => "MATERIAL0".into(),
        XF_MATERIAL_COLOR1 => "MATERIAL1".into(),
        XF_COLOR_CTRL0 => "CHAN_COLOR0_CTRL".into(),
        XF_COLOR_CTRL1 => "CHAN_COLOR1_CTRL".into(),
        XF_ALPHA_CTRL0 => "CHAN_ALPHA0_CTRL".into(),
        XF_ALPHA_CTRL1 => "CHAN_ALPHA1_CTRL".into(),
        XF_DUAL_TEX_ENABLE => "DUALTEX_ENABLE".into(),
        XF_MATRIX_INDEX_A => "MATINDEX_A".into(),
        XF_MATRIX_INDEX_B => "MATINDEX_B".into(),
        XF_VIEWPORT_SCALE_X => "VIEWPORT_SCALE_X".into(),
        XF_VIEWPORT_SCALE_Y => "VIEWPORT_SCALE_Y".into(),
        XF_VIEWPORT_SCALE_Z => "VIEWPORT_SCALE_Z".into(),
        XF_VIEWPORT_OFFSET_X => "VIEWPORT_OFFSET_X".into(),
        XF_VIEWPORT_OFFSET_Y => "VIEWPORT_OFFSET_Y".into(),
        XF_VIEWPORT_OFFSET_Z => "VIEWPORT_OFFSET_Z".into(),
        XF_PROJECTION_BASE..=XF_PROJECTION_END => format!("PROJECTION[{}]", a - XF_PROJECTION_BASE),
        XF_NUM_TEXGENS => "NUMTEXGENS".into(),
        0x1040..=0x1047 => format!("TEXGEN{}", a - 0x1040),
        0x1050..=0x1057 => format!("DUALTEXGEN{}", a - 0x1050),
        _ => format!("XF_{a:04X}"),
    }
}

fn prim_name(cmd: u8) -> String {
    match Primitive::from_cmd(cmd) {
        Some(p) => format!("{p:?}"),
        None => format!("PRIM_{cmd:02X}"),
    }
}

fn load_indx_letter(cmd: u8) -> char {
    match cmd {
        LOAD_INDX_A_CMD => 'A',
        LOAD_INDX_B_CMD => 'B',
        LOAD_INDX_C_CMD => 'C',
        _ => 'D',
    }
}

pub fn summary(cmd: &Command) -> String {
    match &cmd.kind {
        CmdKind::Nop => "NOP".into(),
        CmdKind::InvVtxCache => "INV_VTX_CACHE".into(),
        CmdKind::Cp { reg, value } => {
            format!("CP  {} = {value:08X}", self::cp_reg_name(*reg))
        }
        CmdKind::Xf { addr, values } => {
            format!("XF  {} ({} words)", self::xf_addr_name(*addr), values.len())
        }
        CmdKind::Bp { reg, value } => {
            format!("BP  {} = {value:06X}", self::bp_reg_name(*reg))
        }
        CmdKind::LoadIndx {
            cmd,
            index,
            xf_addr,
            xf_count,
        } => format!(
            "LOAD_INDX_{} idx {index} -> xf[{xf_addr:03X}] x{xf_count}",
            self::load_indx_letter(*cmd)
        ),
        CmdKind::CallDl {
            phys_addr,
            nbytes,
            children,
            missing,
        } => {
            if *missing {
                format!("CALL_DL {phys_addr:08X} ({nbytes} bytes, unmapped)")
            } else {
                format!("CALL_DL {phys_addr:08X} ({nbytes} bytes, {} cmds)", children.len())
            }
        }
        CmdKind::Draw {
            cmd,
            vat,
            count,
            stride,
        } => format!(
            "DRAW {} vat{vat} {count} verts (stride {stride})",
            self::prim_name(*cmd)
        ),
        CmdKind::Unknown { opcode } => format!("UNKNOWN {opcode:02X}"),
        CmdKind::Truncated { opcode } => format!("TRUNCATED {opcode:02X} ({} bytes)", cmd.len),
    }
}

fn bp_detail(reg: u8, value: u32) -> String {
    let r = reg as usize;
    match r {
        BP_GEN_MODE => format!("{:#?}", GenMode::from_raw(value)),
        _ if (BP_IND_CMD_0..BP_IND_CMD_0 + BP_IND_CMD_COUNT).contains(&r) => {
            format!("{:#?}", TevIndirect::from_raw(value))
        }
        BP_SU_SCIS_TL | BP_SU_SCIS_BR => format!("{:#?}", SuScisRect::from_raw(value)),
        BP_RAS1_SS0 | BP_RAS1_SS1 => format!("{:#?}", Ras1Ss::from_raw(value)),
        BP_RAS1_IREF => format!("{:#?}", Ras1IRef::from_raw(value)),
        _ if (BP_RAS1_TREF0..BP_RAS1_TREF0 + BP_RAS1_TREF_COUNT).contains(&r) => {
            format!("{:#?}", TevOrder::from_raw(value))
        }
        BP_PE_ZMODE => format!("{:#?}", ZMode::from_raw(value)),
        BP_PE_CMODE0 => format!("{:#?}", BlendMode::from_raw(value)),
        BP_PE_ZCOMPARE => format!("{:#?}", PeControl::from_raw(value)),
        BP_PE_COPY_SRC => format!("{:#?}", EfbCopySrc::from_raw(value)),
        BP_PE_COPY_DIMS => format!("{:#?}", EfbCopyDims::from_raw(value)),
        BP_PE_COPY_DST => format!("{:#?}", EfbCopyDst::from_raw(value)),
        BP_PE_COPY_DST_STRIDE => format!("{:#?}", EfbCopyDstStride::from_raw(value)),
        BP_PE_COPY_YSCALE => format!("{:#?}", DispCopyYScale::from_raw(value)),
        BP_PE_COPY_CLEAR_AR => format!("{:#?}", PeClearAr::from_raw(value)),
        BP_PE_COPY_CLEAR_GB => format!("{:#?}", PeClearGb::from_raw(value)),
        BP_PE_COPY_CLEAR_Z => format!("{:#?}", PeClearZ::from_raw(value)),
        BP_PE_COPY_CMD => format!("{:#?}", PeCopyCmd::from_raw(value)),
        BP_SU_SCIS_OFFSET => format!("{:#?}", SuScisOffset::from_raw(value)),
        0x80..=0x83 | 0xA0..=0xA3 => format!("{:#?}", TxSetMode0::from_raw(value)),
        0x88..=0x8B | 0xA8..=0xAB => format!("{:#?}", TxSetImage0::from_raw(value)),
        0x94..=0x97 | 0xB4..=0xB7 => {
            let img = TxSetImage3::from_raw(value);
            format!("{img:#?}\nram addr: {:#010X}", img.image_base() << 5)
        }
        0xC0..=0xDF if r % 2 == 0 => format!("{:#?}", TevColorEnv::from_raw(value)),
        0xC0..=0xDF => format!("{:#?}", TevAlphaEnv::from_raw(value)),
        0xE0..=0xE7 if r % 2 == 0 => format!("{:#?}", TevRegisterL::from_raw(value)),
        0xE0..=0xE7 => format!("{:#?}", TevRegisterH::from_raw(value)),
        BP_PE_ALPHA_COMPARE => format!("{:#?}", AlphaCompare::from_raw(value)),
        BP_TEV_ZTEX1 => format!("{:#?}", TevZtex1::from_raw(value)),
        BP_TEV_ZTEX2 => format!("{:#?}", TevZtex2::from_raw(value)),
        _ => String::new(),
    }
}

fn cp_detail(reg: u8, value: u32) -> String {
    let r = reg as usize;
    match r {
        0x30 => format!("{:#?}", MatrixIndex0::from_raw(value)),
        0x40 => format!("{:#?}", MatrixIndex1::from_raw(value)),
        VCD_LO_REG => format!("{:#?}", VcdLo::from_raw(value)),
        VCD_HI_REG => format!("{:#?}", VcdHi::from_raw(value)),
        _ if (VATA_REG..VATA_REG + 8).contains(&r) => format!("{:#?}", VatA::from_raw(value)),
        _ if (VATB_REG..VATB_REG + 8).contains(&r) => format!("{:#?}", VatB::from_raw(value)),
        _ if (VATC_REG..VATC_REG + 8).contains(&r) => format!("{:#?}", VatC::from_raw(value)),
        _ if (ARRAY_BASE_REG..ARRAY_BASE_REG + 16).contains(&r) => {
            format!("base: {:#010X}", value & 0x03FF_FFFF)
        }
        _ if (ARRAY_STRIDE_REG..ARRAY_STRIDE_REG + 16).contains(&r) => {
            format!("stride: {}", value & 0xFF)
        }
        _ => String::new(),
    }
}

fn xf_detail(addr: u16, values: &[u32]) -> String {
    let mut out = String::new();
    let a = addr as usize;
    let float_region = a < 0x0800
        || (XF_VIEWPORT_BASE..=XF_VIEWPORT_END).contains(&a)
        || (XF_PROJECTION_BASE..=XF_PROJECTION_END).contains(&a);
    for (i, v) in values.iter().enumerate() {
        let reg = a + i;
        let name = self::xf_addr_name(reg as u16);
        if float_region {
            out.push_str(&format!("{name}: {} ({v:08X})\n", f32::from_bits(*v)));
        } else {
            out.push_str(&format!("{name}: {v:08X}\n"));
        }
    }

    match a {
        XF_COLOR_CTRL0 | XF_COLOR_CTRL1 | XF_ALPHA_CTRL0 | XF_ALPHA_CTRL1 => {
            out.push_str(&format!("{:#?}\n", ChanCtrl::from_raw(values[0])));
        }
        0x1040..=0x1047 => out.push_str(&format!("{:#?}\n", TexGenReg::from_raw(values[0]))),
        0x1050..=0x1057 => out.push_str(&format!("{:#?}\n", DualTexGenReg::from_raw(values[0]))),
        XF_MATRIX_INDEX_A => out.push_str(&format!("{:#?}\n", MatrixIndex0::from_raw(values[0]))),
        XF_MATRIX_INDEX_B => out.push_str(&format!("{:#?}\n", MatrixIndex1::from_raw(values[0]))),
        _ => {}
    }

    out
}

pub fn detail(cmd: &Command) -> String {
    match &cmd.kind {
        CmdKind::Cp { reg, value } => {
            let decoded = self::cp_detail(*reg, *value);
            format!("CP[{reg:02X}] {} = {value:08X}\n{decoded}", self::cp_reg_name(*reg))
        }
        CmdKind::Bp { reg, value } => {
            let decoded = self::bp_detail(*reg, *value);
            format!("BP[{reg:02X}] {} = {value:06X}\n{decoded}", self::bp_reg_name(*reg))
        }
        CmdKind::Xf { addr, values } => {
            format!(
                "XF[{addr:04X}..{:04X}]\n{}",
                *addr as usize + values.len(),
                self::xf_detail(*addr, values)
            )
        }
        CmdKind::LoadIndx {
            cmd,
            index,
            xf_addr,
            xf_count,
        } => format!(
            "LOAD_INDX_{}\narray index: {index}\nxf dest: {xf_addr:03X}\ncount: {xf_count}",
            self::load_indx_letter(*cmd)
        ),
        CmdKind::CallDl {
            phys_addr,
            nbytes,
            children,
            missing,
        } => format!(
            "CALL_DL\naddr: {phys_addr:#010X}\nsize: {nbytes} bytes\ncommands: {}{}",
            children.len(),
            if *missing { "\n(source unmapped)" } else { "" }
        ),
        CmdKind::Draw {
            cmd,
            vat,
            count,
            stride,
        } => format!(
            "DRAW {}\nvat: {vat}\nvertices: {count}\nstride: {stride} bytes\ndata: {} bytes",
            self::prim_name(*cmd),
            *count as usize * stride
        ),
        other => self::summary(&Command {
            offset: cmd.offset,
            len: cmd.len,
            kind: other.clone(),
        }),
    }
}
