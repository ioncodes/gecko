use cranelift_codegen::Context;
use cranelift_codegen::ir::{self, InstBuilder};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::Module;
use std::mem::offset_of;

use super::attr::{self, AttrCtx};
use super::{ResolvedArray, VtxKey};
use crate::host::DrawVertex;

pub(crate) const MEMFLAGS: ir::MemFlagsData = ir::MemFlagsData::new().with_notrap();
pub(crate) const MEMFLAGS_RO: ir::MemFlagsData = ir::MemFlagsData::new().with_notrap().with_readonly();

pub fn build_parser(
    ctx: &mut Context,
    fn_ctx: &mut FunctionBuilderContext,
    module: &mut JITModule,
    pointer_ty: ir::Type,
    key: VtxKey,
) -> bool {
    if !self::is_supported() {
        return false;
    }

    let isa = module.isa();
    let mut bd = FunctionBuilder::new(&mut ctx.func, fn_ctx);

    let entry = bd.create_block();
    let header = bd.create_block();
    let body = bd.create_block();
    let exit = bd.create_block();

    bd.append_block_params_for_function_params(entry);
    bd.append_block_param(header, pointer_ty);
    bd.append_block_param(header, pointer_ty);
    bd.append_block_param(header, ir::types::I32);

    bd.switch_to_block(entry);
    bd.seal_block(entry);

    let params = bd.block_params(entry);
    let gp_ptr = params[0];
    let xf_mem_ptr = params[1];
    let arrays_ptr = params[2];
    let init_data_ptr = params[3];
    let init_out_ptr = params[4];
    let count = params[5];

    let zero = bd.ins().iconst(ir::types::I32, 0);
    bd.ins().jump(
        header,
        &[
            ir::BlockArg::Value(init_data_ptr),
            ir::BlockArg::Value(init_out_ptr),
            ir::BlockArg::Value(zero),
        ],
    );

    bd.switch_to_block(header);
    let h = bd.block_params(header);
    let data_ptr = h[0];
    let out_ptr = h[1];
    let iter = h[2];

    let cond = bd.ins().icmp(ir::condcodes::IntCC::UnsignedLessThan, iter, count);
    bd.ins().brif(cond, body, &[], exit, &[]);

    bd.switch_to_block(body);
    bd.seal_block(body);

    let mut actx = AttrCtx {
        bd: &mut bd,
        isa,
        gp_ptr,
        xf_mem_ptr,
        arrays_ptr,
        data_ptr,
        out_ptr,
        pointer_ty,
        key,
    };

    attr::emit_vertex(&mut actx);

    let new_data_ptr = actx.data_ptr;
    let new_out_ptr = actx
        .bd
        .ins()
        .iadd_imm(actx.out_ptr, std::mem::size_of::<DrawVertex>() as i64);
    let new_iter = actx.bd.ins().iadd_imm(iter, 1);

    actx.bd.ins().jump(
        header,
        &[
            ir::BlockArg::Value(new_data_ptr),
            ir::BlockArg::Value(new_out_ptr),
            ir::BlockArg::Value(new_iter),
        ],
    );

    bd.seal_block(header);

    bd.switch_to_block(exit);
    bd.seal_block(exit);
    bd.ins().return_(&[]);

    bd.finalize();
    true
}

fn is_supported() -> bool {
    std::env::var("GECKO_VTX_JIT_OFF").is_err()
}

pub mod offset {
    use super::*;
    pub const POSITION: i32 = offset_of!(DrawVertex, position) as i32;
    pub const NORMAL: i32 = offset_of!(DrawVertex, normal) as i32;
    pub const COLOR0: i32 = offset_of!(DrawVertex, color0) as i32;
    pub const COLOR1: i32 = offset_of!(DrawVertex, color1) as i32;
    pub const POS_VIEW: i32 = offset_of!(DrawVertex, pos_view) as i32;
    pub const TEXCOORDS: i32 = offset_of!(DrawVertex, texcoords) as i32;
}

pub mod array_offset {
    use super::*;
    pub const HOST_BASE: i32 = offset_of!(ResolvedArray, host_base) as i32;
    pub const STRIDE: i32 = offset_of!(ResolvedArray, stride) as i32;
    pub const SIZE: i32 = std::mem::size_of::<ResolvedArray>() as i32;
}

#[inline(always)]
pub fn xf_byte_off(cell: usize) -> i32 {
    (cell * 4) as i32
}
