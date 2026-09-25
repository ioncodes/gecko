use super::constants::*;

pub(super) fn unimplemented(bank: &str, reg: usize, value: u32) {
    tracing::warn!(
        target: "gecko::gx::unimplemented",
        %bank, reg = format_args!("{reg:#06X}"), value = format_args!("{value:#010X}"),
        "unimplemented GX register write"
    );
}

pub(super) fn bp_implemented(reg: usize) -> bool {
    matches!(
        reg,
        BP_GEN_MODE
            | BP_SU_SCIS_TL
            | BP_SU_SCIS_BR
            | BP_SU_SCIS_OFFSET
            | BP_RAS1_SS0
            | BP_RAS1_SS1
            | BP_RAS1_IREF
            | BP_PE_ZMODE
            | BP_PE_CMODE0
            | BP_PE_CMODE1
            | BP_PE_ZCOMPARE
            | BP_PE_DONE
            | BP_PE_TOKEN
            | BP_PE_TOKEN_INT
            | BP_PE_COPY_SRC
            | BP_PE_COPY_DIMS
            | BP_PE_COPY_DST
            | BP_PE_COPY_DST_STRIDE
            | BP_PE_COPY_YSCALE
            | BP_PE_COPY_CLEAR_AR
            | BP_PE_COPY_CLEAR_GB
            | BP_PE_COPY_CLEAR_Z
            | BP_PE_COPY_CMD
            | BP_PRELOAD_ADDR
            | BP_PRELOAD_TMEM_EVEN
            | BP_PRELOAD_TMEM_ODD
            | BP_PRELOAD_MODE
            | BP_LOAD_TLUT0
            | BP_LOAD_TLUT1
            | BP_PE_ALPHA_COMPARE
            | BP_TEV_ZTEX1
            | BP_TEV_ZTEX2
            | BP_BP_MASK
    ) || (BP_IND_MTX_A0..=BP_IND_MTX_C2).contains(&reg)
        || (BP_IND_CMD_0..BP_IND_CMD_0 + BP_IND_CMD_COUNT).contains(&reg)
        || (BP_RAS1_TREF0..BP_RAS1_TREF0 + BP_RAS1_TREF_COUNT).contains(&reg)
        || (BP_SU_SSIZE0..BP_SU_SIZE_END).contains(&reg)
        || (BP_TX_SETMODE0_I0..BP_TX_SETTLUT_I0 + 4).contains(&reg)
        || (BP_TX_SETMODE0_I4..BP_TX_SETTLUT_I4 + 4).contains(&reg)
        || (BP_TEV_COLOR_ENV_0..BP_TEV_COLOR_ENV_0 + 32).contains(&reg)
        || (BP_TEV_REGISTERL_0..BP_TEV_REGISTERL_0 + 8).contains(&reg)
        || (BP_TEV_KSEL_0..BP_TEV_KSEL_0 + 8).contains(&reg)
}

pub(super) fn cp_implemented(reg: usize) -> bool {
    matches!(reg, VCD_LO_REG | VCD_HI_REG)
        || (VATA_REG..VATA_REG + 8).contains(&reg)
        || (VATB_REG..VATB_REG + 8).contains(&reg)
        || (VATC_REG..VATC_REG + 8).contains(&reg)
        || (ARRAY_BASE_REG..ARRAY_BASE_REG + 16).contains(&reg)
        || (ARRAY_STRIDE_REG..ARRAY_STRIDE_REG + 16).contains(&reg)
}

pub(super) fn xf_implemented(reg: usize) -> bool {
    reg < dff::XF_MEM_SIZE
        || matches!(
            reg,
            XF_MATRIX_INDEX_A | XF_MATRIX_INDEX_B | XF_DUAL_TEX_ENABLE | XF_NUM_TEXGENS
        )
        || (XF_AMBIENT_COLOR0..=XF_ALPHA_CTRL1).contains(&reg)
        || (XF_VIEWPORT_BASE..=XF_VIEWPORT_END).contains(&reg)
        || (XF_PROJECTION_BASE..=XF_PROJECTION_END).contains(&reg)
        || (XF_TEXGEN_BASE..XF_TEXGEN_BASE + 8).contains(&reg)
        || (XF_DUALTEX_BASE..XF_DUALTEX_BASE + 8).contains(&reg)
}
