use chapa::BitEnum;

use crate::flipper::gx::draw::TextureFormat;

// GX compare function (shared by Z-mode and alpha compare)
#[derive(Debug, PartialEq, BitEnum, Hash, Eq)]
pub enum CompareFunc {
    Never = 0,
    Less = 1,
    Equal = 2,
    LessEqual = 3,
    Greater = 4,
    NotEqual = 5,
    GreaterEqual = 6,
    Always = 7,
}

// GX blend factor
#[derive(Debug, PartialEq, BitEnum, Hash, Eq)]
pub enum BlendFactor {
    Zero = 0,
    One = 1,
    SrcClr = 2,
    InvSrcClr = 3,
    SrcAlpha = 4,
    InvSrcAlpha = 5,
    DstAlpha = 6,
    InvDstAlpha = 7,
}

// GX alpha combine op
#[derive(Debug, PartialEq, BitEnum, Hash, Eq)]
pub enum AlphaOp {
    And = 0,
    Or = 1,
    Xor = 2,
    Xnor = 3,
}

// BP 0x40 Z-mode
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default)]
pub struct ZMode {
    #[bits(0)]
    pub enable: bool,

    #[bits(1..=3)]
    pub func: CompareFunc,

    #[bits(4)]
    pub update_enable: bool,
}

#[derive(Debug, BitEnum)]
pub enum LogicOp {
    Clear = 0,
    And = 1,
    ReverseAnd = 2,
    Copy = 3,
    InvertedAnd = 4,
    Noop = 5,
    Xor = 6,
    Or = 7,
    Nor = 8,
    Equivalent = 9,
    Invert = 10,
    ReverseOr = 11,
    InvertedCopy = 12,
    InvertedOr = 13,
    Nand = 14,
    Set = 15,
}

// BP 0x41 Blend mode (PE_CMODE0)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default)]
pub struct BlendMode {
    #[bits(0)]
    pub blend_enable: bool,

    #[bits(1)]
    pub logic_op_enable: bool,

    #[bits(2)]
    pub dither_enable: bool,

    #[bits(3)]
    pub color_update: bool,

    #[bits(4)]
    pub alpha_update: bool,

    #[bits(5..=7)]
    pub dst_factor: BlendFactor,

    #[bits(8..=10)]
    pub src_factor: BlendFactor,

    #[bits(11)]
    pub subtract: bool,

    #[bits(12..=15)] // TODO: double check
    pub logic_op: LogicOp,
}

// BP 0xF3 Alpha compare
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AlphaCompare {
    #[bits(0..=7)]
    pub ref0: u8,

    #[bits(8..=15)]
    pub ref1: u8,

    #[bits(16..=18)]
    pub comp0: CompareFunc,

    #[bits(19..=21)]
    pub comp1: CompareFunc,

    #[bits(22..=23)]
    pub op: AlphaOp,
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum TexCount {
    S,  // 1D coordinate
    St, // 2D coordinate
}

impl TexCount {
    pub fn components(&self) -> usize {
        match self {
            TexCount::S => 1,
            TexCount::St => 2,
        }
    }
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum PosCount {
    Xy,
    Xyz,
}

impl PosCount {
    pub fn components(&self) -> usize {
        match self {
            PosCount::Xy => 2,
            PosCount::Xyz => 3,
        }
    }
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum ComponentFormat {
    U8,
    S8,
    U16,
    S16,
    F32,
}

impl ComponentFormat {
    pub fn size(&self) -> usize {
        match self {
            ComponentFormat::U8 | ComponentFormat::S8 => 1,
            ComponentFormat::U16 | ComponentFormat::S16 => 2,
            ComponentFormat::F32 => 4,
        }
    }
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum ColorCount {
    Rgb,
    Rgba,
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum ColorFormat {
    Rgb565,
    Rgb8,
    Rgbx8,
    Rgba4,
    Rgba6,
    Rgba8,
}

impl ColorFormat {
    pub fn data_size(&self, count: ColorCount) -> usize {
        match (self, count) {
            (ColorFormat::Rgb565, _) => 2,
            (ColorFormat::Rgb8, _) => 3,
            (ColorFormat::Rgbx8, _) => 4,
            (ColorFormat::Rgba4, _) => 2,
            (ColorFormat::Rgba6, _) => 3,
            (ColorFormat::Rgba8, ColorCount::Rgb) => 3,
            (ColorFormat::Rgba8, ColorCount::Rgba) => 4,
        }
    }
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum AttributeType {
    None,
    Direct,
    Index8,
    Index16,
}

impl AttributeType {
    pub fn size(&self) -> usize {
        match self {
            AttributeType::None => 0,
            AttributeType::Index8 => 1,
            AttributeType::Index16 => 2,
            AttributeType::Direct => unimplemented!("illegal?"),
        }
    }
}

// CP 0x70-0x77 (one per vertex format)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VatA {
    #[bits(0)]
    pub pos_cnt: PosCount,

    #[bits(1..=3)]
    pub pos_fmt: ComponentFormat,

    #[bits(4..=8)]
    pub pos_shift: u8,

    #[bits(13)]
    pub clr0_cnt: ColorCount,

    #[bits(14..=16)]
    pub clr0_fmt: ColorFormat,

    #[bits(21)]
    pub tex0_cnt: TexCount,

    #[bits(22..=24)]
    pub tex0_fmt: ComponentFormat,

    #[bits(25..=29)]
    pub tex0_shift: u8,
}

impl VatA {
    pub fn pos_data_size(&self) -> usize {
        self.pos_cnt().components() * self.pos_fmt().size()
    }

    pub fn clr0_data_size(&self) -> usize {
        self.clr0_fmt().data_size(self.clr0_cnt())
    }

    pub fn tex0_data_size(&self) -> usize {
        self.tex0_cnt().components() * self.tex0_fmt().size()
    }
}

// CP 0x60-0x67 (one per vertex format)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VcdHi {
    #[bits(0..=1)]
    pub tex0: AttributeType,
}

// CP 0x50
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VcdLo {
    #[bits(9..=10)]
    pub position: AttributeType,

    #[bits(13..=14)]
    pub color0: AttributeType,
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct TxSetImage0 {
    #[bits(0..=9)]
    pub width: u16, // width - 1

    #[bits(10..=19)]
    pub height: u16, // height - 1

    #[bits(20..=23)]
    pub format: TextureFormat,
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct TxSetImage3 {
    #[bits(0..=23)]
    pub image_base: u32,
}

impl TxSetImage3 {
    /// Physical RAM address (image_base << 5)
    pub fn ram_addr(&self) -> usize {
        self.image_base() as usize * 32
    }
}


// TEV color combiner input select (SELA-SELD)
#[derive(Debug, PartialEq, BitEnum)]
pub enum TevColorIn {
    CPrev = 0x0,
    APrev = 0x1,
    C0 = 0x2,
    A0 = 0x3,
    C1 = 0x4,
    A1 = 0x5,
    C2 = 0x6,
    A2 = 0x7,
    TexC = 0x8,
    TexA = 0x9,
    RasC = 0xA,
    RasA = 0xB,
    One = 0xC,
    Half = 0xD,
    Konst = 0xE,
    Zero = 0xF,
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum TevBias {
    Zero = 0,
    AddHalf = 1,
    SubHalf = 2,
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum TevScale {
    Scale1 = 0,
    Scale2 = 1,
    Scale4 = 2,
    Divide2 = 3,
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum TevRegId {
    TevPrev = 0,
    TevReg0 = 1,
    TevReg1 = 2,
    TevReg2 = 3,
}

// BP 0xC0+stage*2 TEV

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TevColorEnv {
    #[bits(0..=3)]
    pub d: TevColorIn,

    #[bits(4..=7)]
    pub c: TevColorIn,

    #[bits(8..=11)]
    pub b: TevColorIn,

    #[bits(12..=15)]
    pub a: TevColorIn,

    #[bits(16..=17)]
    pub bias: TevBias,

    #[bits(18)]
    pub sub: bool,

    #[bits(19)]
    pub clamp: bool,

    #[bits(20..=21)]
    pub scale: TevScale,

    #[bits(22..=23)]
    pub dest: TevRegId,
}

// TEV color register type
#[derive(Debug, PartialEq, BitEnum)]
pub enum TevRegType {
    Color = 0,
    Constant = 1,
}

// BP 0xE0/0xE2/0xE4/0xE6 TEV_REGISTERL
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TevRegisterL {
    #[bits(0..=10)]
    pub r: u16,

    #[bits(12..=22)]
    pub a: u16,

    #[bits(23)]
    pub reg_type: TevRegType,
}

// BP 0xE1/0xE3/0xE5/0xE7 TEV_REGISTERH
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TevRegisterH {
    #[bits(0..=10)]
    pub b: u16,

    #[bits(12..=22)]
    pub g: u16,

    #[bits(23)]
    pub reg_type: TevRegType,
}

// BP 0x00 GEN_MODE
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GenMode {
    #[bits(10..=13)]
    pub num_tev_stages: u8, // num stages - 1
}