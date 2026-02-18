
pub fn branch<const LINK: bool, const ABSOLUTE: bool>(target: i32, ctx: &mut crate::gekko::Gekko) {
    if LINK {
        ctx.cpu.write_gpr(31, ctx.cpu.pc + 4);
    }

    if ABSOLUTE {
        ctx.cpu.pc = target as u32;
    } else {
        ctx.cpu.pc = ctx.cpu.pc.wrapping_add_signed(target);
    }
}