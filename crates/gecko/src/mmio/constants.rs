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
pub const CP_BASE: u32 = 0x0C00_0000;
pub const CP_END: u32 = 0x0C00_0FFF;

pub const PE_BASE: u32 = 0x0C00_1000;
pub const PE_END: u32 = 0x0C00_10FF;

pub const VI_BASE: u32 = 0x0C00_2000;
pub const VI_END: u32 = 0x0C00_27FF;

pub const PI_BASE: u32 = 0x0C00_3000;
pub const PI_END: u32 = 0x0C00_3FFF;

pub const MI_BASE: u32 = 0x0C00_4000;
pub const MI_END: u32 = 0x0C00_4FFF;

pub const DSP_BASE: u32 = 0x0C00_5000;
pub const DSP_END: u32 = 0x0C00_5FFF;

pub const DI_BASE: u32 = 0x0C00_6000;
pub const DI_END: u32 = 0x0C00_63FF;

pub const SI_BASE: u32 = 0x0C00_6400;
pub const SI_END: u32 = 0x0C00_67FF;

pub const EXI_BASE: u32 = 0x0C00_6800;
pub const EXI_END: u32 = 0x0C00_6BFF;

pub const AI_BASE: u32 = 0x0C00_6C00;
pub const AI_END: u32 = 0x0C00_6FFF;

// GX Write Gather Pipe
pub const GX_FIFO_BASE: u32 = 0x0C00_8000;
pub const GX_FIFO_END: u32 = 0x0C00_801F;

// IPL / Bootrom
pub const IPL_BASE: u32 = 0x3FF0_0000;
pub const IPL_END: u32 = 0x3FFF_FFFF;

// Locked Cache 16KB
pub const LCACHE_BASE: u32 = 0xE000_0000;
pub const LCACHE_END: u32 = 0xE000_3FFF;
pub const LCACHE_SIZE: usize = 0x4000;

// [Wii] MEM2 64MB at virt 0x9000_0000 / phys 0x1000_0000
pub const MEM2_BASE: u32 = 0x1000_0000;
pub const MEM2_END: u32 = 0x13FF_FFFF;
pub const MEM2_SIZE: usize = 0x0400_0000;

// [Wii] Hollywood register space
pub const HW_HOLLYWOOD_BASE: u32 = 0x0D00_0000;
pub const HW_HOLLYWOOD_END: u32 = 0x0D80_FFFF;

// [Wii] IPC registers (HW_IPC_PPCMSG / PPCCTRL / ARMMSG)
pub const IPC_BASE: u32 = 0x0D00_0000;
pub const IPC_END: u32 = 0x0D00_000F;

// [Wii] Hollywood interrupt controller
pub const HOLLYWOOD_IRQ_BASE: u32 = 0x0D00_0030;
pub const HOLLYWOOD_IRQ_END: u32 = 0x0D00_003F;

// [Wii] PPC-side mirror of the Hollywood GPIO controller (8 x 32-bit regs)
pub const HOLLYWOOD_GPIO_PPC_BASE: u32 = 0x0D80_00C0;
pub const HOLLYWOOD_GPIO_PPC_END: u32 = 0x0D80_00DF;
