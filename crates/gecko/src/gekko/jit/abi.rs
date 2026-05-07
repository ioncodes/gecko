use core::mem::offset_of;

use crate::gekko::Gekko;
use crate::gekko::condition::ConditionRegister;
use crate::gekko::msr::Msr;
use crate::gekko::spr::{Spr, Xer};
use crate::mmio::Mmio;
use crate::scheduler::Scheduler;
use crate::system::{System, SystemId};

const _: () = {
    if core::mem::size_of::<ConditionRegister>() != 4 {
        panic!("ConditionRegister is no longer 4 bytes; JIT CR access needs to be revisited");
    }
    if core::mem::align_of::<ConditionRegister>() != 4 {
        panic!("ConditionRegister alignment changed; JIT CR access needs to be revisited");
    }
    if core::mem::size_of::<Xer>() != 4 {
        panic!("Xer is no longer 4 bytes; JIT XER access needs to be revisited");
    }
    if core::mem::size_of::<Msr>() != 4 {
        panic!("Msr is no longer 4 bytes; JIT MSR access needs to be revisited");
    }
};

#[inline(always)]
pub const fn gekko_offset<const SYSTEM: SystemId>() -> usize {
    offset_of!(System<SYSTEM>, gekko)
}

#[inline(always)]
pub const fn scheduler_offset<const SYSTEM: SystemId>() -> usize {
    offset_of!(System<SYSTEM>, scheduler)
}

#[inline(always)]
pub const fn pc_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, pc)
}

#[inline(always)]
pub const fn cia_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, cia)
}

#[inline(always)]
pub const fn nia_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, nia)
}

#[inline(always)]
pub const fn gpr_base_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, gprs)
}

#[inline(always)]
pub const fn fpr_base_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, fprs)
}

#[inline(always)]
pub const fn ps1_base_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, ps1s)
}

#[inline(always)]
pub const fn cycles_offset<const SYSTEM: SystemId>() -> usize {
    scheduler_offset::<SYSTEM>() + offset_of!(Scheduler<SYSTEM>, cycles)
}

#[inline(always)]
pub const fn next_deadline_offset<const SYSTEM: SystemId>() -> usize {
    scheduler_offset::<SYSTEM>() + offset_of!(Scheduler<SYSTEM>, next_deadline)
}

#[inline(always)]
pub const fn cr_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, cr)
}

#[inline(always)]
pub const fn xer_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, spr) + offset_of!(Spr, xer)
}

#[inline(always)]
pub const fn fpscr_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, fpscr)
}

#[inline(always)]
pub const fn ram_ptr_offset<const SYSTEM: SystemId>() -> usize {
    offset_of!(System<SYSTEM>, mmio) + offset_of!(Mmio<SYSTEM>, ram_ptr)
}

#[inline(always)]
pub const fn fastmem_lut_ptr_offset<const SYSTEM: SystemId>() -> usize {
    offset_of!(System<SYSTEM>, mmio) + offset_of!(Mmio<SYSTEM>, fastmem_lut_ptr)
}

#[inline(always)]
pub const fn gqr_offset<const SYSTEM: SystemId>(i: u8) -> usize {
    let spr_off = gekko_offset::<SYSTEM>() + offset_of!(Gekko, spr);
    spr_off
        + match i {
            0 => offset_of!(Spr, gqr0),
            1 => offset_of!(Spr, gqr1),
            2 => offset_of!(Spr, gqr2),
            3 => offset_of!(Spr, gqr3),
            4 => offset_of!(Spr, gqr4),
            5 => offset_of!(Spr, gqr5),
            6 => offset_of!(Spr, gqr6),
            7 => offset_of!(Spr, gqr7),
            _ => panic!("GQR index out of range"),
        }
}

#[inline(always)]
pub const fn lr_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, spr) + offset_of!(Spr, lr)
}

#[inline(always)]
pub const fn ctr_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, spr) + offset_of!(Spr, ctr)
}

#[inline(always)]
pub const fn msr_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, msr)
}

#[inline(always)]
pub const fn timebase_offset_offset<const SYSTEM: SystemId>() -> usize {
    scheduler_offset::<SYSTEM>() + offset_of!(Scheduler<SYSTEM>, timebase_offset)
}

#[inline(always)]
pub const fn reserve_addr_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, reserve_addr)
}

#[inline(always)]
pub const fn sr_base_offset<const SYSTEM: SystemId>() -> usize {
    gekko_offset::<SYSTEM>() + offset_of!(Gekko, sr)
}

#[inline(always)]
pub fn spr_field_offset<const SYSTEM: SystemId>(spr_num: u16) -> Option<usize> {
    let spr_off = gekko_offset::<SYSTEM>() + offset_of!(Gekko, spr);
    let inner = match spr_num {
        1 => offset_of!(Spr, xer),
        8 => offset_of!(Spr, lr),
        9 => offset_of!(Spr, ctr),
        18 => offset_of!(Spr, dsisr),
        19 => offset_of!(Spr, dar),
        25 => offset_of!(Spr, sdr1),
        26 => offset_of!(Spr, srr0),
        27 => offset_of!(Spr, srr1),
        272 => offset_of!(Spr, sprg0),
        273 => offset_of!(Spr, sprg1),
        274 => offset_of!(Spr, sprg2),
        275 => offset_of!(Spr, sprg3),
        282 => offset_of!(Spr, ear),
        287 => offset_of!(Spr, pvr),
        528 => offset_of!(Spr, ibat0u),
        529 => offset_of!(Spr, ibat0l),
        530 => offset_of!(Spr, ibat1u),
        531 => offset_of!(Spr, ibat1l),
        532 => offset_of!(Spr, ibat2u),
        533 => offset_of!(Spr, ibat2l),
        534 => offset_of!(Spr, ibat3u),
        535 => offset_of!(Spr, ibat3l),
        536 => offset_of!(Spr, dbat0u),
        537 => offset_of!(Spr, dbat0l),
        538 => offset_of!(Spr, dbat1u),
        539 => offset_of!(Spr, dbat1l),
        540 => offset_of!(Spr, dbat2u),
        541 => offset_of!(Spr, dbat2l),
        542 => offset_of!(Spr, dbat3u),
        543 => offset_of!(Spr, dbat3l),

        560 => offset_of!(Spr, ibat4u),
        561 => offset_of!(Spr, ibat4l),
        562 => offset_of!(Spr, ibat5u),
        563 => offset_of!(Spr, ibat5l),
        564 => offset_of!(Spr, ibat6u),
        565 => offset_of!(Spr, ibat6l),
        566 => offset_of!(Spr, ibat7u),
        567 => offset_of!(Spr, ibat7l),
        568 => offset_of!(Spr, dbat4u),
        569 => offset_of!(Spr, dbat4l),
        570 => offset_of!(Spr, dbat5u),
        571 => offset_of!(Spr, dbat5l),
        572 => offset_of!(Spr, dbat6u),
        573 => offset_of!(Spr, dbat6l),
        574 => offset_of!(Spr, dbat7u),
        575 => offset_of!(Spr, dbat7l),
        912 => offset_of!(Spr, gqr0),
        913 => offset_of!(Spr, gqr1),
        914 => offset_of!(Spr, gqr2),
        915 => offset_of!(Spr, gqr3),
        916 => offset_of!(Spr, gqr4),
        917 => offset_of!(Spr, gqr5),
        918 => offset_of!(Spr, gqr6),
        919 => offset_of!(Spr, gqr7),
        920 => offset_of!(Spr, hid2),
        921 => offset_of!(Spr, wpar),
        936 => offset_of!(Spr, ummcr0),
        937 => offset_of!(Spr, upmc1),
        938 => offset_of!(Spr, upmc2),
        939 => offset_of!(Spr, usia),
        940 => offset_of!(Spr, ummcr1),
        941 => offset_of!(Spr, upmc3),
        942 => offset_of!(Spr, upmc4),
        943 => offset_of!(Spr, usda),
        952 => offset_of!(Spr, mmcr0),
        953 => offset_of!(Spr, pmc1),
        954 => offset_of!(Spr, pmc2),
        955 => offset_of!(Spr, sia),
        956 => offset_of!(Spr, mmcr1),
        957 => offset_of!(Spr, pmc3),
        958 => offset_of!(Spr, pmc4),
        959 => offset_of!(Spr, sda),
        1008 => offset_of!(Spr, hid0),
        1009 => offset_of!(Spr, hid1),
        1010 => offset_of!(Spr, iabr),
        1011 => offset_of!(Spr, hid4),
        1013 => offset_of!(Spr, dabr),
        1017 => offset_of!(Spr, l2cr),
        1019 => offset_of!(Spr, ictc),
        1020 => offset_of!(Spr, thrm1),
        1021 => offset_of!(Spr, thrm2),
        1022 => offset_of!(Spr, thrm3),
        _ => return None,
    };
    Some(spr_off + inner)
}
