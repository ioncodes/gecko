pub const CP_CMD: u8 = 0x08;
pub const XF_CMD: u8 = 0x10;
pub const BP_CMD: u8 = 0x61;

pub const DRAW_TRIANGLES_CMD: u8 = 0x90;

pub const BP_REG_SIZE: usize = 0x100;
pub const CP_REG_SIZE: usize = 0xc0;
pub const XF_MEM_SIZE: usize = 0x1058;

pub const VCD_LO_REG: usize = 0x50;
pub const VATA_REG: usize = 0x70;
pub const ARRAY_BASE_REG: usize = 0xA0;
pub const ARRAY_STRIDE_REG: usize = 0xB0;

pub const ARRAY_POS: usize = 0; 
pub const ARRAY_NRM: usize = 1; 
pub const ARRAY_CLR0: usize = 2;
pub const ARRAY_CLR1: usize = 3;