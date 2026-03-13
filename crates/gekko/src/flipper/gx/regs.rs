// CP 0x70
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VatA {
    #[bits(0)]
    pub pos_cnt: u8,

    #[bits(1..=3)]
    pub pos_fmt: u8,

    #[bits(4..=8)]
    pub pos_shift: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum PosCount {
    Xy,
    Xyz,
}

#[derive(Debug, Clone, Copy)]
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

impl VatA {
    pub fn pos_count(&self) -> PosCount {
        match self.pos_cnt() {
            0 => PosCount::Xy,
            1 => PosCount::Xyz,
            _ => unreachable!(),
        }
    }

    pub fn pos_format(&self) -> ComponentFormat {
        match self.pos_fmt() {
            0 => ComponentFormat::U8,
            1 => ComponentFormat::S8,
            2 => ComponentFormat::U16,
            3 => ComponentFormat::S16,
            4 => ComponentFormat::F32,
            _ => unreachable!(),
        }
    }

    pub fn pos_components(&self) -> usize {
        match self.pos_count() {
            PosCount::Xy => 2,
            PosCount::Xyz => 3,
        }
    }

    pub fn pos_data_size(&self) -> usize {
        self.pos_components() * self.pos_format().size()
    }
}

// CP 0x50
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Debug, Clone, Copy)]
pub struct VcdLo {
    #[bits(9..=10)]
    pub position: u8,

    #[bits(13..=14)]
    pub color0: u8,
}

impl VcdLo {
    pub fn pos_attr(&self) -> AttributeType {
        match self.position() {
            0 => AttributeType::None,
            1 => AttributeType::Direct,
            2 => AttributeType::Index8,
            3 => AttributeType::Index16,
            _ => unreachable!(),
        }
    }

    pub fn color0_attr(&self) -> AttributeType {
        match self.color0() {
            0 => AttributeType::None,
            1 => AttributeType::Direct,
            2 => AttributeType::Index8,
            3 => AttributeType::Index16,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
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
