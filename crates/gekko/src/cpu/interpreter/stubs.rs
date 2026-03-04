macro_rules! stub {
    ($($name:ident),* $(,)?) => {
        $(
            #[rustfmt::skip]
            pub fn $name(
                _ctx: &mut crate::gekko::Gekko,
                _instr: crate::cpu::semantics::Instruction,
            ) {
                todo!(stringify!($name))
            }
        )*
    };
}

stub! {
    twi,
    ps_cmpu0, ps_cmpo0, ps_cmpu1, ps_cmpo1,
    ps_res, ps_rsqrte, ps_neg, ps_mr, ps_nabs, ps_abs,
    ps_merge00, ps_merge01, ps_merge10, ps_merge11,
    psq_lx, psq_stx, psq_lux, psq_stux,
    ps_sum0, ps_sum1, ps_muls0, ps_muls1,
    ps_madds0, ps_madds1,
    ps_div, ps_sub, ps_add, ps_sel, ps_mul,
    ps_msub, ps_madd, ps_nmsub, ps_nmadd,
    sc, tw,
    lwarx, stwcx_dot,
    mtsrin, mcrxr,
    lwbrx, lswx, lswi, mfsrin,
    stswx, stwbrx, stswi,
    lhbrx,
    eciwx, ecowx,
    sthbrx, stfiwx,
    psq_l, psq_lu, psq_st, psq_stu,
    fsqrtsx, fresx, fsqrtx, fselx, frsqrtex,
}
