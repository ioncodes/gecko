use super::*;
use crate::mmio::traits::MmioRegister;

fn vi_with_top(raw: u32) -> Vi {
    let mut vi = Vi::new();
    vi.top_field_base = regs::TopFieldBase::from_raw(raw);
    vi
}

fn vi_with_dcr(raw: u16) -> Vi {
    let mut vi = Vi::new();
    vi.dcr = regs::DisplayConfiguration::from_raw(raw);
    vi
}

const TOP_OFF: u32 = regs::TopFieldBase::ADDR - VI_BASE;
const DCR_OFF: u32 = regs::DisplayConfiguration::ADDR - VI_BASE;

#[test]
fn u16_write_upper_half_of_u32_register() {
    let mut vi = vi_with_top(0xAABBCCDD);
    vi.mmio_write_u16(TOP_OFF, 0x1234);
    assert_eq!(vi.top_field_base.to_raw(), 0x1234CCDD);
}

#[test]
fn u16_write_lower_half_of_u32_register() {
    let mut vi = vi_with_top(0xAABBCCDD);
    vi.mmio_write_u16(TOP_OFF + 2, 0x5678);
    assert_eq!(vi.top_field_base.to_raw(), 0xAABB5678);
}

#[test]
fn u16_read_upper_half_of_u32_register() {
    let vi = vi_with_top(0xAABBCCDD);
    assert_eq!(vi.mmio_read_u16(TOP_OFF), 0xAABB);
}

#[test]
fn u16_read_lower_half_of_u32_register() {
    let vi = vi_with_top(0xAABBCCDD);
    assert_eq!(vi.mmio_read_u16(TOP_OFF + 2), 0xCCDD);
}

#[test]
fn u8_write_each_byte_of_u32_register() {
    let mut vi = vi_with_top(0x00000000);
    vi.mmio_write_u8(TOP_OFF + 0, 0xAA);
    vi.mmio_write_u8(TOP_OFF + 1, 0xBB);
    vi.mmio_write_u8(TOP_OFF + 2, 0xCC);
    vi.mmio_write_u8(TOP_OFF + 3, 0xDD);
    assert_eq!(vi.top_field_base.to_raw(), 0xAABBCCDD);
}

#[test]
fn u8_write_does_not_disturb_other_bytes_in_u32() {
    let mut vi = vi_with_top(0xAABBCCDD);
    vi.mmio_write_u8(TOP_OFF + 1, 0xFF);
    assert_eq!(vi.top_field_base.to_raw(), 0xAAFFCCDD);
}

#[test]
fn u8_write_upper_byte_of_u16_register() {
    let mut vi = vi_with_dcr(0x00FF);
    vi.mmio_write_u8(DCR_OFF, 0xAB);
    assert_eq!(vi.dcr.to_raw(), 0xABFF);
}

#[test]
fn u8_write_lower_byte_of_u16_register() {
    let mut vi = vi_with_dcr(0xFF00);
    vi.mmio_write_u8(DCR_OFF + 1, 0xCD);
    assert_eq!(vi.dcr.to_raw(), 0xFFCD);
}
