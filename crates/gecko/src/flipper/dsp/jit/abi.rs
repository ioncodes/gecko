use core::mem::offset_of;

use crate::flipper::dsp::Dsp;
use crate::flipper::dsp::core::Registers;
use crate::flipper::dsp::regs::ControlStatus;
use crate::system::{GC, System, SystemId, WII};

macro_rules! make_dsp_offset {
    ($name:ident, $reg_field:ident, $($rest:tt)*) => {
        #[inline(always)]
        pub const fn $name() -> usize {
            const fn off<const SYSTEM: SystemId>() -> usize {
                offset_of!(System<SYSTEM>, dsp)
                    + offset_of!(Dsp, registers)
                    + offset_of!(Registers, $reg_field)
                    $($rest)*
            }
            const G: usize = off::<GC>();
            const W: usize = off::<WII>();
            const _: () = {
                if G != W {
                    panic!(concat!(
                        stringify!($name),
                        " offset differs between GC and Wii layouts"
                    ));
                }
            };
            G
        }
    };
}

macro_rules! make_dsp_field_offset {
    ($name:ident, $field:ident) => {
        #[inline(always)]
        pub const fn $name() -> usize {
            const fn off<const SYSTEM: SystemId>() -> usize {
                offset_of!(System<SYSTEM>, dsp) + offset_of!(Dsp, $field)
            }
            const G: usize = off::<GC>();
            const W: usize = off::<WII>();
            const _: () = {
                if G != W {
                    panic!(concat!(
                        stringify!($name),
                        " offset differs between GC and Wii layouts"
                    ));
                }
            };
            G
        }
    };
}

make_dsp_offset!(dsp_pc_offset_max, pc,);
make_dsp_offset!(dsp_cia_offset_max, cia,);
make_dsp_offset!(dsp_nia_offset_max, nia,);
make_dsp_offset!(dsp_ac0_high_offset, ac0_high,);
make_dsp_offset!(dsp_ac1_high_offset, ac1_high,);
make_dsp_offset!(dsp_ac0_mid_offset, ac0_mid,);
make_dsp_offset!(dsp_ac1_mid_offset, ac1_mid,);
make_dsp_offset!(dsp_ac0_low_offset, ac0_low,);
make_dsp_offset!(dsp_ac1_low_offset, ac1_low,);
make_dsp_offset!(dsp_config_offset, config,);
make_dsp_offset!(dsp_product_low_offset, product_low,);
make_dsp_offset!(dsp_product_mid1_offset, product_mid1,);
make_dsp_offset!(dsp_product_high_offset, product_high,);
make_dsp_offset!(dsp_product_mid2_offset, product_mid2,);
make_dsp_offset!(dsp_ar_base_offset, ar,);
make_dsp_offset!(dsp_ix_base_offset, ix,);
make_dsp_offset!(dsp_wr_base_offset, wr,);
make_dsp_offset!(dsp_ax_base_offset, ax,);
make_dsp_offset!(dsp_axh_base_offset, axh,);
make_dsp_offset!(dsp_status_offset, status,);
make_dsp_offset!(dsp_ext_ac_cache_base_offset, ext_ac_cache,);
make_dsp_offset!(dsp_call_stack_offset, call_stack,);
make_dsp_offset!(dsp_data_stack_offset, data_stack,);
make_dsp_offset!(dsp_loop_addr_offset, loop_addr,);
make_dsp_offset!(dsp_loop_counter_offset, loop_counter,);

#[inline(always)]
pub const fn dsp_loop_addr_ptr_offset_max() -> usize {
    const fn off<const SYSTEM: SystemId>() -> usize {
        offset_of!(System<SYSTEM>, dsp)
            + offset_of!(Dsp, registers)
            + offset_of!(Registers, loop_addr)
            + core::mem::size_of::<[u16; 32]>()
    }
    const G: usize = off::<GC>();
    const W: usize = off::<WII>();
    const _: () = {
        if G != W {
            panic!("dsp_loop_addr_ptr_offset differs between GC and Wii");
        }
    };
    G
}

make_dsp_field_offset!(dsp_csr_offset, csr);
make_dsp_field_offset!(dsp_chain_budget_offset, chain_budget);
make_dsp_field_offset!(dsp_instr_count_offset, instr_count);
make_dsp_field_offset!(dsp_dram_ptr_offset, dram);
make_dsp_field_offset!(dsp_coef_ptr_offset, coef);
make_dsp_field_offset!(dsp_iram_ptr_offset, iram);
make_dsp_field_offset!(dsp_irom_ptr_offset, irom);

const _: () = {
    let _ = dsp_pc_offset_max();
    let _ = dsp_cia_offset_max();
    let _ = dsp_nia_offset_max();
    let _ = dsp_ac0_high_offset();
    let _ = dsp_ac1_high_offset();
    let _ = dsp_ac0_mid_offset();
    let _ = dsp_ac1_mid_offset();
    let _ = dsp_ac0_low_offset();
    let _ = dsp_ac1_low_offset();
    let _ = dsp_config_offset();
    let _ = dsp_product_low_offset();
    let _ = dsp_product_mid1_offset();
    let _ = dsp_product_high_offset();
    let _ = dsp_product_mid2_offset();
    let _ = dsp_ar_base_offset();
    let _ = dsp_ix_base_offset();
    let _ = dsp_wr_base_offset();
    let _ = dsp_ax_base_offset();
    let _ = dsp_axh_base_offset();
    let _ = dsp_status_offset();
    let _ = dsp_ext_ac_cache_base_offset();
    let _ = dsp_csr_offset();
    let _ = dsp_loop_addr_ptr_offset_max();
    let _ = dsp_chain_budget_offset();
    let _ = dsp_instr_count_offset();
    let _ = dsp_dram_ptr_offset();
    let _ = dsp_coef_ptr_offset();
    let _ = dsp_call_stack_offset();
    let _ = dsp_data_stack_offset();
    let _ = dsp_loop_addr_offset();
    let _ = dsp_loop_counter_offset();
    let _ = dsp_iram_ptr_offset();
    let _ = dsp_irom_ptr_offset();
};

const _: () = {
    if core::mem::size_of::<ControlStatus>() != 2 {
        panic!("ControlStatus is no longer 2 bytes");
    }
    if core::mem::align_of::<ControlStatus>() != 2 {
        panic!("ControlStatus alignment changed");
    }
};
