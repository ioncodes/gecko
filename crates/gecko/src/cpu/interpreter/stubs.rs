macro_rules! stub {
    ($($name:ident),* $(,)?) => {
        $(
            #[rustfmt::skip]
            #[inline(always)]
            pub fn $name(
                _ctx: &mut crate::gamecube::GameCube,
                _instr: crate::cpu::semantics::Instruction,
            ) {
                todo!(stringify!($name))
            }
        )*
    };
}

stub! {
    twi,
    tw,
    mtsrin, mcrxr,
    lwbrx, lswx, lswi, mfsrin,
    stswx, stwbrx, stswi,
    lhbrx,
    eciwx, ecowx,
    sthbrx,
}
