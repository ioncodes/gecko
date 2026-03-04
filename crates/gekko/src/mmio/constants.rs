pub const RAM_BASE: u32 = 0x0000_0000;
pub const RAM_END: u32 = 0x017F_FFFF;
pub const RAM_SIZE: usize = 0x0180_0000; // 24 MB

pub const EFB_BASE: u32 = 0x0800_0000;
pub const EFB_END: u32 = 0x081F_FFFF;
pub const EFB_SIZE: usize = 0x0020_0000; // 2 MB

pub const HW_REG_BASE: u32 = 0x0C00_0000;
pub const HW_REG_END: u32 = 0x0C7F_FFFF;
pub const HW_REG_SIZE: usize = 0x0080_0000; // 8 MB

// Hardware block address ranges (physical)
pub const VI_BASE: u32 = 0x0C00_2000;
pub const VI_END: u32 = 0x0C00_27FF;
