mod alu;
mod branch;
mod compare;
mod cr_ops;
mod fp_ops;
mod ps_ops;
mod psq;
mod rotate;
mod store_load;
mod system;

pub use alu::{alu, logical};
pub use branch::branch;
pub use compare::compare;
pub use cr_ops::{cr_ops, mcrxr};
pub use fp_ops::fp_ops;
pub use ps_ops::ps_ops;
pub use psq::store_load_psq;
pub use rotate::rotate;
pub use store_load::{eciwx, ecowx, lswi, lswx, lwarx, store_load, store_load_fp, stswi, stswx, stwcx_dot};
pub use system::{mfsrin, mftb, msr, mtsrin, nop, sc, segment, spr, tw, twi};

#[cold]
#[inline(never)]
pub fn invalid(_ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::instruction::Instruction) {
    panic!("unimplemented Gekko opcode {:#010x}", instr.0);
}
