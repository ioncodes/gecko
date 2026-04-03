use chapa::BitEnum;

#[derive(BitEnum, PartialEq, PartialOrd)]
pub enum SignExtensionMode {
    Bits16,
    Bits40,
}

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct StatusRegister {
    #[bits(0, alias = "c")]
    pub carry: bool,
    #[bits(1, alias = "o")]
    pub overflow: bool,
    #[bits(2, alias = "z")]
    pub arithmetic_zero: bool,
    #[bits(3, alias = "s")]
    pub sign: bool,
    #[bits(4, alias = "as32")]
    pub above_s32: bool,
    #[bits(5, alias = "tb")]
    pub top_two_bits_equal: bool,
    #[bits(6, alias = "lz")]
    pub logical_zero: bool,
    #[bits(7, alias = "os")]
    pub overflow_sticky: bool,
    #[bits(9, alias = "ie")]
    pub interrupt_enable: bool,
    #[bits(11, alias = "eie")]
    pub external_interrupt_enable: bool,
    #[bits(13, alias = "am")]
    pub product_multiply_result_by_2: bool, // when AM = 0
    #[bits(14, alias = "sxm")]
    pub sign_extension_mode: SignExtensionMode,
    #[bits(15, alias = "su")]
    pub multiplication_operands_are_signed: bool,
}

/// DSP DMA transfer direction
#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum DspDmaDirection {
    MainToDsp = 0,
    DspToMain = 1,
}

/// DSP memory type involved in DMA
#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum DspMemoryType {
    Data = 0,
    Instruction = 1,
}

/// Accelerator ARAM access direction
#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum AcceleratorDirection {
    Read = 0,
    Write = 1,
}

/// ARAM DMA request mask state.
#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum AramDmaMask {
    Unmasked = 0,
    Masked = 1,
}

/// 0xFFC9 - DSCR - DSP DMA Control Register
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct DspDmaControl {
    #[bits(0)]
    pub direction: DspDmaDirection,

    #[bits(1)]
    pub memory_type: DspMemoryType,

    #[bits(2, readonly)]
    pub busy: bool,
}

/// 0xFFCB - DSBL - DSP DMA Block Length
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct DspDmaBlockLength {
    #[bits(0..=1, readonly)]
    pub lsb: u8,

    #[bits(2..=15)]
    pub length: u16,
}

/// 0xFFCD - DSPA - DSP DMA DSP Memory Address
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct DspDmaDspAddr {
    #[bits(0, readonly)]
    pub lsb: bool,

    #[bits(1..=15)]
    pub address: u16,
}

/// 0xFFCE - DSMAH - DSP DMA Main Memory Address High
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct DspDmaMainAddrHigh {
    #[bits(0..=9)]
    pub address: u16,

    #[bits(10..=15, readonly)]
    pub _reserved: u8,
}

/// 0xFFCF - DSMAL - DSP DMA Main Memory Address Low
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct DspDmaMainAddrLow {
    #[bits(0..=1, readonly)]
    pub lsb: u8,

    #[bits(2..=15)]
    pub address: u16,
}

/// 0xFFD4 - ACSAH - Accelerator ARAM Starting Address High
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct AccStartAddrHigh {
    #[bits(0..=10)]
    pub address: u16,
}

/// 0xFFD6 - ACEAH - Accelerator ARAM Ending Address High
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct AccEndAddrHigh {
    #[bits(0..=10)]
    pub address: u16,
}

/// 0xFFD8 - ACCAH - Accelerator ARAM Current Address High
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct AccCurrentAddrHigh {
    #[bits(0..=10)]
    pub address: u16,

    #[bits(15)]
    pub direction: AcceleratorDirection,
}

/// 0xFFEF - AMDM - ARAM DMA Request Mask
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Clone, Copy, Default)]
pub struct AramDmaRequestMask {
    #[bits(0)]
    pub mask: AramDmaMask,
}
