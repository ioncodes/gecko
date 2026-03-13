use chapa::BitEnum;

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
            AttributeType::Direct => unimplemented!("from VAT?"),
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
}

impl VatA {
    pub fn pos_data_size(&self) -> usize {
        self.pos_cnt().components() * self.pos_fmt().size()
    }

    pub fn clr0_data_size(&self) -> usize {
        self.clr0_fmt().data_size(self.clr0_cnt())
    }
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