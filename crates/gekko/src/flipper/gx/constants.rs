pub const CP_CMD: u8 = 0x08;
pub const XF_CMD: u8 = 0x10;
pub const BP_CMD: u8 = 0x61;

pub const DRAW_COMMANDS_START: u8 = 0x80;
pub const DRAW_COMMANDS_END: u8 = 0xBF; // TODO: Double check this

pub const DRAW_QUADS_CMD: u8 = 0x80;
pub const DRAW_TRIANGLES_CMD: u8 = 0x90;
pub const DRAW_TRIANGLE_STRIP_CMD: u8 = 0x98;
pub const DRAW_TRIANGLE_FAN_CMD: u8 = 0xA0;
pub const DRAW_LINES_CMD: u8 = 0xA8;
pub const DRAW_LINE_STRIP_CMD: u8 = 0xB0;
pub const DRAW_POINTS_CMD: u8 = 0xB8;

pub const BP_REG_SIZE: usize = 0x100;
pub const CP_REG_SIZE: usize = 0xc0;
pub const XF_MEM_SIZE: usize = 0x1058;

pub const VCD_LO_REG: usize = 0x50;
pub const VCD_HI_REG: usize = 0x60;
pub const VATA_REG: usize = 0x70;
pub const ARRAY_BASE_REG: usize = 0xA0;
pub const ARRAY_STRIDE_REG: usize = 0xB0;

pub const ARRAY_POS: usize = 0;
pub const ARRAY_NRM: usize = 1;
pub const ARRAY_CLR0: usize = 2;
pub const ARRAY_CLR1: usize = 3;

// XF memory addresses
pub const XF_MODELVIEW_BASE: usize = 0x0000;
pub const XF_MODELVIEW_END: usize = 0x000B;
pub const XF_PROJECTION_BASE: usize = 0x1020;
pub const XF_PROJECTION_END: usize = 0x1026;

// BP texture register base addresses (maps 0-3: base, maps 4-7: base + 0x20)
pub const BP_TX_SETMODE0_I0: usize = 0x80; // TX_SETMODE0 maps 0-3
pub const BP_TX_SETMODE1_I0: usize = 0x84; // TX_SETMODE1 maps 0-3
pub const BP_TX_SETIMAGE0_I0: usize = 0x88; // TX_SETIMAGE0 maps 0-3 (width/height/format)
pub const BP_TX_SETIMAGE1_I0: usize = 0x8C; // TX_SETIMAGE1 maps 0-3
pub const BP_TX_SETIMAGE2_I0: usize = 0x90; // TX_SETIMAGE2 maps 0-3
pub const BP_TX_SETIMAGE3_I0: usize = 0x94; // TX_SETIMAGE3 maps 0-3 (IMAGE_BASE = addr >> 5)
pub const BP_TX_SETMODE0_I4: usize = 0xA0; // TX_SETMODE0 maps 4-7
pub const BP_TX_SETMODE1_I4: usize = 0xA4; // TX_SETMODE1 maps 4-7
pub const BP_TX_SETIMAGE0_I4: usize = 0xA8; // TX_SETIMAGE0 maps 4-7
pub const BP_TX_SETIMAGE1_I4: usize = 0xAC; // TX_SETIMAGE1 maps 4-7
pub const BP_TX_SETIMAGE2_I4: usize = 0xB0; // TX_SETIMAGE2 maps 4-7
pub const BP_TX_SETIMAGE3_I4: usize = 0xB4; // TX_SETIMAGE3 maps 4-7 (IMAGE_BASE = addr >> 5)

// BP PE (Pixel Engine) registers
pub const BP_PE_ZMODE: usize = 0x40;
pub const BP_PE_CMODE0: usize = 0x41; // blend mode
pub const BP_PE_ALPHA_COMPARE: usize = 0xF3;
pub const BP_PE_DONE: usize = 0x45;
pub const BP_PE_DONE_FINISH_BIT: u32 = 0x02;

// BP TEV (Texture Environment) registers
pub const BP_GEN_MODE: usize = 0x00;
pub const BP_RAS1_TREF0: usize = 0x28; // ..0x2F, order registers
pub const BP_RAS1_TREF_COUNT: usize = 8;
pub const BP_TEV_COLOR_ENV_0: usize = 0xC0; // stage N color = 0xC0 + N*2
pub const BP_TEV_ALPHA_ENV_0: usize = 0xC1; // stage N alpha = 0xC1 + N*2
// lo = 0xE0 + N*2 (R,A), hi = 0xE1 + N*2 (G,B)
pub const BP_TEV_REGISTERL_0: usize = 0xE0;
pub const BP_TEV_REGISTERH_0: usize = 0xE1;
