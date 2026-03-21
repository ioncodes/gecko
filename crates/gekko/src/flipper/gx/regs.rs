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
pub enum NrmCount {
    Xyz, // 3 components (normal only)
    Nbt, // 9 components (normal + binormal + tangent)
}

impl NrmCount {
    pub fn components(&self) -> usize {
        match self {
            NrmCount::Xyz => 3,
            NrmCount::Nbt => 9,
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

    #[bits(9)]
    pub nrm_cnt: NrmCount,

    #[bits(10..=12)]
    pub nrm_fmt: ComponentFormat,

    #[bits(13)]
    pub clr0_cnt: ColorCount,

    #[bits(14..=16)]
    pub clr0_fmt: ColorFormat,

    #[bits(17)]
    pub clr1_cnt: ColorCount,

    #[bits(18..=20)]
    pub clr1_fmt: ColorFormat,

    #[bits(21)]
    pub tex0_cnt: TexCount,

    #[bits(22..=24)]
    pub tex0_fmt: ComponentFormat,

    #[bits(25..=29)]
    pub tex0_shift: u8,

    #[bits(30)]
    pub byte_dequant: bool,

    #[bits(31)]
    pub nrm_index3: bool,
}

impl VatA {
    pub fn pos_data_size(&self) -> usize {
        self.pos_cnt().components() * self.pos_fmt().size()
    }

    pub fn nrm_data_size(&self) -> usize {
        self.nrm_cnt().components() * self.nrm_fmt().size()
    }

    /// Number of bytes the normal attribute occupies in the FIFO stream.
    pub fn nrm_stream_size(&self, attr: AttributeType) -> usize {
        match attr {
            AttributeType::None => 0,
            AttributeType::Direct => self.nrm_data_size(),
            AttributeType::Index8 => {
                if self.nrm_index3() && self.nrm_cnt() == NrmCount::Nbt {
                    3
                } else {
                    1
                }
            }
            AttributeType::Index16 => {
                if self.nrm_index3() && self.nrm_cnt() == NrmCount::Nbt {
                    6
                } else {
                    2
                }
            }
        }
    }

    pub fn clr0_data_size(&self) -> usize {
        self.clr0_fmt().data_size(self.clr0_cnt())
    }

    pub fn clr1_data_size(&self) -> usize {
        self.clr1_fmt().data_size(self.clr1_cnt())
    }

    pub fn tex0_data_size(&self) -> usize {
        self.tex0_cnt().components() * self.tex0_fmt().size()
    }
}

// CP 0x80-0x87 (one per vertex format)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VatB {
    #[bits(0)]
    pub tex1_cnt: TexCount,

    #[bits(1..=3)]
    pub tex1_fmt: ComponentFormat,

    #[bits(4..=8)]
    pub tex1_shift: u8,

    #[bits(9)]
    pub tex2_cnt: TexCount,

    #[bits(10..=12)]
    pub tex2_fmt: ComponentFormat,

    #[bits(13..=17)]
    pub tex2_shift: u8,

    #[bits(18)]
    pub tex3_cnt: TexCount,

    #[bits(19..=21)]
    pub tex3_fmt: ComponentFormat,

    #[bits(22..=26)]
    pub tex3_shift: u8,

    #[bits(27)]
    pub tex4_cnt: TexCount,

    #[bits(28..=30)]
    pub tex4_fmt: ComponentFormat,
}

impl VatB {
    pub fn tex1_data_size(&self) -> usize {
        self.tex1_cnt().components() * self.tex1_fmt().size()
    }

    pub fn tex2_data_size(&self) -> usize {
        self.tex2_cnt().components() * self.tex2_fmt().size()
    }

    pub fn tex3_data_size(&self) -> usize {
        self.tex3_cnt().components() * self.tex3_fmt().size()
    }

    pub fn tex4_data_size(&self) -> usize {
        self.tex4_cnt().components() * self.tex4_fmt().size()
    }
}

// CP 0x90-0x97 (one per vertex format)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VatC {
    #[bits(0..=4)]
    pub tex4_shift: u8,

    #[bits(5)]
    pub tex5_cnt: TexCount,

    #[bits(6..=8)]
    pub tex5_fmt: ComponentFormat,

    #[bits(9..=13)]
    pub tex5_shift: u8,

    #[bits(14)]
    pub tex6_cnt: TexCount,

    #[bits(15..=17)]
    pub tex6_fmt: ComponentFormat,

    #[bits(18..=22)]
    pub tex6_shift: u8,

    #[bits(23)]
    pub tex7_cnt: TexCount,

    #[bits(24..=26)]
    pub tex7_fmt: ComponentFormat,

    #[bits(27..=31)]
    pub tex7_shift: u8,
}

impl VatC {
    pub fn tex5_data_size(&self) -> usize {
        self.tex5_cnt().components() * self.tex5_fmt().size()
    }

    pub fn tex6_data_size(&self) -> usize {
        self.tex6_cnt().components() * self.tex6_fmt().size()
    }

    pub fn tex7_data_size(&self) -> usize {
        self.tex7_cnt().components() * self.tex7_fmt().size()
    }
}

// CP 0x60-0x67 (one per vertex format)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VcdHi {
    #[bits(0..=1)]
    pub tex0: AttributeType,

    #[bits(2..=3)]
    pub tex1: AttributeType,

    #[bits(4..=5)]
    pub tex2: AttributeType,

    #[bits(6..=7)]
    pub tex3: AttributeType,

    #[bits(8..=9)]
    pub tex4: AttributeType,

    #[bits(10..=11)]
    pub tex5: AttributeType,

    #[bits(12..=13)]
    pub tex6: AttributeType,

    #[bits(14..=15)]
    pub tex7: AttributeType,
}

// CP 0x50-0x57
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VcdLo {
    #[bits(0, alias = "pmidx")]
    pub pos_nrm_mtx_idx: bool,

    #[bits(1, alias = "t0midx")]
    pub tex0_mtx_idx: bool,

    #[bits(2, alias = "t1midx")]
    pub tex1_mtx_idx: bool,

    #[bits(3, alias = "t2midx")]
    pub tex2_mtx_idx: bool,

    #[bits(4, alias = "t3midx")]
    pub tex3_mtx_idx: bool,

    #[bits(5, alias = "t4midx")]
    pub tex4_mtx_idx: bool,

    #[bits(6, alias = "t5midx")]
    pub tex5_mtx_idx: bool,

    #[bits(7, alias = "t6midx")]
    pub tex6_mtx_idx: bool,

    #[bits(8, alias = "t7midx")]
    pub tex7_mtx_idx: bool,

    #[bits(9..=10, alias = "pos")]
    pub position: AttributeType,

    #[bits(11..=12, alias = "nrm")]
    pub normal: AttributeType,

    #[bits(13..=14, alias = "col0")]
    pub color0: AttributeType,

    #[bits(15..=16, alias = "col1")]
    pub color1: AttributeType,
}

impl VcdLo {
    /// Number of matrix index bytes in the vertex stream.
    pub fn mtx_idx_count(&self) -> usize {
        self.pos_nrm_mtx_idx() as usize
            + self.tex0_mtx_idx() as usize
            + self.tex1_mtx_idx() as usize
            + self.tex2_mtx_idx() as usize
            + self.tex3_mtx_idx() as usize
            + self.tex4_mtx_idx() as usize
            + self.tex5_mtx_idx() as usize
            + self.tex6_mtx_idx() as usize
            + self.tex7_mtx_idx() as usize
    }
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

#[derive(Debug, PartialEq, Eq, Hash, BitEnum)]
pub enum WrapMode {
    Clamp = 0,
    Repeat = 1,
    Mirror = 2,
}

#[derive(Debug, PartialEq, Eq, Hash, BitEnum)]
pub enum MagFilter {
    Nearest = 0,
    Linear = 1,
}

#[derive(Debug, PartialEq, Eq, Hash, BitEnum)]
pub enum MinFilter {
    Nearest = 0,
    NearestMipmapNearest = 1,
    NearestMipmapLinear = 2,
    Linear = 4,
    LinearMipmapNearest = 5,
    LinearMipmapLinear = 6,
}

#[derive(Debug, PartialEq, Eq, Hash, BitEnum)]
pub enum RasterChannel {
    Color0 = 0,
    Color1 = 1,
    Alpha0 = 2,
    Alpha1 = 3,
    Color0A0 = 4,
    Color1A1 = 5,
    ColorZero = 6,
    Bump = 7,
}

// BP 0x28-0x2F RAS1_TREF0-7
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TevOrder {
    #[bits(0..=2)]
    pub texmap0: u8,

    #[bits(3..=5)]
    pub texcoord0: u8,

    #[bits(6)]
    pub tex_enable0: bool,

    #[bits(7..=9)]
    pub channel0: RasterChannel,

    #[bits(12..=14)]
    pub texmap1: u8,

    #[bits(15..=17)]
    pub texcoord1: u8,

    #[bits(18)]
    pub tex_enable1: bool,

    #[bits(19..=21)]
    pub channel1: RasterChannel,
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct TxSetMode0 {
    #[bits(0..=1)]
    pub wrap_s: WrapMode,

    #[bits(2..=3)]
    pub wrap_t: WrapMode,

    #[bits(4..=4)]
    pub mag_filter: MagFilter,

    #[bits(5..=7)]
    pub min_filter: MinFilter,
}

// TEV color combiner input select (SELA-SELD)
#[derive(Debug, PartialEq, BitEnum)]
pub enum TevColorIn {
    PrevColor = 0x0,
    PrevAlpha = 0x1,
    Reg0Color = 0x2,
    Reg0Alpha = 0x3,
    Reg1Color = 0x4,
    Reg1Alpha = 0x5,
    Reg2Color = 0x6,
    Reg2Alpha = 0x7,
    TexColor = 0x8,
    TexAlpha = 0x9,
    RasColor = 0xA,
    RasAlpha = 0xB,
    One = 0xC,
    Half = 0xD,
    Konst = 0xE,
    Zero = 0xF,
}

impl std::fmt::Display for TevColorIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::PrevColor => "Previous Color",
            Self::PrevAlpha => "Previous Alpha",
            Self::Reg0Color => "Reg0 Color",
            Self::Reg0Alpha => "Reg0 Alpha",
            Self::Reg1Color => "Reg1 Color",
            Self::Reg1Alpha => "Reg1 Alpha",
            Self::Reg2Color => "Reg2 Color",
            Self::Reg2Alpha => "Reg2 Alpha",
            Self::TexColor => "Texture Color",
            Self::TexAlpha => "Texture Alpha",
            Self::RasColor => "Vertex Color",
            Self::RasAlpha => "Vertex Alpha",
            Self::One => "1",
            Self::Half => "0.5",
            Self::Konst => "Constant",
            Self::Zero => "0",
        })
    }
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

impl std::fmt::Display for TevRegId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TevPrev => "Previous",
            Self::TevReg0 => "Reg0",
            Self::TevReg1 => "Reg1",
            Self::TevReg2 => "Reg2",
        })
    }
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

// TEV alpha combiner input select (SELA-SELD)
#[derive(Debug, PartialEq, BitEnum)]
pub enum TevAlphaIn {
    PrevAlpha = 0,
    Reg0Alpha = 1,
    Reg1Alpha = 2,
    Reg2Alpha = 3,
    TexAlpha = 4,
    RasAlpha = 5,
    Konst = 6,
    Zero = 7,
}

impl std::fmt::Display for TevAlphaIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::PrevAlpha => "Previous Alpha",
            Self::Reg0Alpha => "Reg0 Alpha",
            Self::Reg1Alpha => "Reg1 Alpha",
            Self::Reg2Alpha => "Reg2 Alpha",
            Self::TexAlpha => "Texture Alpha",
            Self::RasAlpha => "Vertex Alpha",
            Self::Konst => "Constant",
            Self::Zero => "0",
        })
    }
}

// BP 0xC1+stage*2 TEV
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TevAlphaEnv {
    #[bits(4..=6)]
    pub d: TevAlphaIn,

    #[bits(7..=9)]
    pub c: TevAlphaIn,

    #[bits(10..=12)]
    pub b: TevAlphaIn,

    #[bits(13..=15)]
    pub a: TevAlphaIn,

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

// XF 0x1018 Matrix Index A (position/tex0-3 matrix indices)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MatrixIndex0 {
    #[bits(0..=5)]
    pub pos_mtx_idx: u8,

    #[bits(6..=11)]
    pub tex0_mtx_idx: u8,

    #[bits(12..=17)]
    pub tex1_mtx_idx: u8,

    #[bits(18..=23)]
    pub tex2_mtx_idx: u8,

    #[bits(24..=29)]
    pub tex3_mtx_idx: u8,
}

impl MatrixIndex0 {
    pub fn tex_mtx_idx(&self, n: usize) -> u8 {
        match n {
            0 => self.tex0_mtx_idx(),
            1 => self.tex1_mtx_idx(),
            2 => self.tex2_mtx_idx(),
            3 => self.tex3_mtx_idx(),
            _ => 0,
        }
    }
}

// XF 0x1019 Matrix Index B (tex4-7 matrix indices)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MatrixIndex1 {
    #[bits(0..=5)]
    pub tex4_mtx_idx: u8,

    #[bits(6..=11)]
    pub tex5_mtx_idx: u8,

    #[bits(12..=17)]
    pub tex6_mtx_idx: u8,

    #[bits(18..=23)]
    pub tex7_mtx_idx: u8,
}

impl MatrixIndex1 {
    pub fn tex_mtx_idx(&self, n: usize) -> u8 {
        match n {
            4 => self.tex4_mtx_idx(),
            5 => self.tex5_mtx_idx(),
            6 => self.tex6_mtx_idx(),
            7 => self.tex7_mtx_idx(),
            _ => 0,
        }
    }
}

// Diffuse lighting function
#[derive(Debug, PartialEq, BitEnum)]
pub enum DiffuseFn {
    None = 0,
    Signed = 1,
    Clamp = 2,
}

// Attenuation function
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum AttnFn {
    None,
    Spot,
    Spec,
}

// XF 0x100E-0x1011 Channel Control
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ChanCtrl {
    #[bits(0)]
    pub mat_src: bool,

    #[bits(1)]
    pub enable: bool,

    #[bits(2..=5)]
    pub lit_mask_lo: u8,

    #[bits(6)]
    pub amb_src: bool,

    #[bits(7..=8)]
    pub diff_fn: DiffuseFn,

    #[bits(9)]
    pub attn_enable: bool,

    #[bits(10)]
    pub attn_select: bool,

    #[bits(11..=14)]
    pub lit_mask_hi: u8,
}

impl ChanCtrl {
    pub fn light_mask(&self) -> u8 {
        self.lit_mask_lo() | (self.lit_mask_hi() << 4)
    }

    pub fn attn_fn(&self) -> AttnFn {
        if !self.attn_enable() {
            AttnFn::None
        } else if self.attn_select() {
            AttnFn::Spot
        } else {
            AttnFn::Spec
        }
    }
}

// XF 0x1040-0x1047: Texture coordinate generation parameters
#[derive(Debug, PartialEq, BitEnum)]
pub enum TexGenProjection {
    St,  // 2x4 matrix (s, t output)
    Stq, // 3x4 matrix (s, t, q output)
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum TexGenInputForm {
    Ab11, // (a, b, 1.0, 1.0)
    Abc1, // (a, b, c, 1.0)
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum TexGenType {
    Regular = 0,
    EmbossMap = 1,
    Color0 = 2,
    Color1 = 3,
}

#[derive(Debug, PartialEq, BitEnum)]
pub enum TexGenSrc {
    Pos = 0,
    Nrm = 1,
    Color = 2,
    BinNrm = 3,
    Tangent = 4,
    Tex0 = 5,
    Tex1 = 6,
    Tex2 = 7,
    Tex3 = 8,
    Tex4 = 9,
    Tex5 = 10,
    Tex6 = 11,
    Tex7 = 12,
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct TexGenReg {
    #[bits(1)]
    pub projection: TexGenProjection,

    #[bits(2, alias = "input_form")]
    pub texgen_input_form: TexGenInputForm,

    #[bits(4..=6)]
    pub texgen_type: TexGenType,

    #[bits(7..=11)]
    pub source_row: TexGenSrc,

    #[bits(12..=14)]
    pub emboss_source: u8,

    #[bits(15..=17)]
    pub emboss_light: u8,
}

// XF 0x1050-0x1057: Dual texture transform (post-transform)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct DualTexGenReg {
    #[bits(0..=5)]
    pub post_mtx_idx: u8,

    #[bits(6)]
    pub normalize: bool,
}
