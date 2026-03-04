#[derive(Debug)]
pub enum BranchControl {
    BranchIfConditionTrue,
    BranchIfConditionFalse,
    BranchAlways,
    DecrementBranchIfNotZeroAndConditionFalse,
    DecrementBranchIfZeroAndConditionFalse,
    DecrementBranchIfNotZeroAndConditionTrue,
    DecrementBranchIfZeroAndConditionTrue,
    DecrementBranchIfNotZero,
    DecrementBranchIfZero,
}

impl BranchControl {
    pub fn from_bo(value: u8) -> Self {
        let branch_hint = value & 0b1 == 0;
        tracing::trace!("Branch hint: {branch_hint}");

        match value & 0b11110 {
            0b00000 => Self::DecrementBranchIfNotZeroAndConditionFalse,
            0b00010 => Self::DecrementBranchIfZeroAndConditionFalse,
            0b00100 | 0b00110 => Self::BranchIfConditionFalse,
            0b01000 => Self::DecrementBranchIfNotZeroAndConditionTrue,
            0b01010 => Self::DecrementBranchIfZeroAndConditionTrue,
            0b01100 | 0b01110 => Self::BranchIfConditionTrue,
            0b10000 | 0b11000 => Self::DecrementBranchIfNotZero,
            0b10010 | 0b11010 => Self::DecrementBranchIfZero,
            _ => Self::BranchAlways,
        }
    }

    pub fn should_branch(&self, ctr: u32, condition: bool) -> bool {
        match self {
            Self::BranchIfConditionTrue => condition,
            Self::BranchIfConditionFalse => !condition,
            Self::BranchAlways => true,
            Self::DecrementBranchIfNotZeroAndConditionFalse => ctr != 0 && !condition,
            Self::DecrementBranchIfZeroAndConditionFalse => ctr == 0 && !condition,
            Self::DecrementBranchIfNotZeroAndConditionTrue => ctr != 0 && condition,
            Self::DecrementBranchIfZeroAndConditionTrue => ctr == 0 && condition,
            Self::DecrementBranchIfNotZero => ctr != 0,
            Self::DecrementBranchIfZero => ctr == 0,
        }
    }

    pub fn should_decrement_ctr(&self) -> bool {
        matches!(
            self,
            Self::DecrementBranchIfNotZeroAndConditionFalse
                | Self::DecrementBranchIfZeroAndConditionFalse
                | Self::DecrementBranchIfNotZeroAndConditionTrue
                | Self::DecrementBranchIfZeroAndConditionTrue
                | Self::DecrementBranchIfNotZero
                | Self::DecrementBranchIfZero
        )
    }
}

#[chapa::bitfield(u8, width = 4, order = msb0)]
#[derive(Clone, Copy)]
pub struct ConditionField {
    #[bits(0, alias = ["less_than", "fx", "fp_exception"])]
    pub lt: bool,
    #[bits(1, alias = ["greater_than", "fex", "fp_enabled_exception"])]
    pub gt: bool,
    #[bits(2, alias = ["equal", "vx", "fp_invalid_exception"])]
    pub eq: bool,
    #[bits(3, alias = ["summary_overflow", "ox", "fp_overflow_exception"])]
    pub so: bool,
}

#[chapa::bitfield(u32, order = msb0)]
#[derive(Clone, Copy)]
pub struct ConditionRegister {
    #[bits(0..=3)]
    pub cr0: ConditionField,
    #[bits(4..=7)]
    pub cr1: ConditionField,
    #[bits(8..=11)]
    pub cr2: ConditionField,
    #[bits(12..=15)]
    pub cr3: ConditionField,
    #[bits(16..=19)]
    pub cr4: ConditionField,
    #[bits(20..=23)]
    pub cr5: ConditionField,
    #[bits(24..=27)]
    pub cr6: ConditionField,
    #[bits(28..=31)]
    pub cr7: ConditionField,
}

impl ConditionRegister {
    #[inline]
    pub fn set_field(&mut self, index: u8, value: ConditionField) {
        match index {
            0 => self.set_cr0(value),
            1 => self.set_cr1(value),
            2 => self.set_cr2(value),
            3 => self.set_cr3(value),
            4 => self.set_cr4(value),
            5 => self.set_cr5(value),
            6 => self.set_cr6(value),
            7 => self.set_cr7(value),
            _ => panic!("Invalid CR field index: {}", index),
        }
    }

    #[inline]
    pub fn get_bit(&self, bit: u8) -> bool {
        (self.raw() >> (31 - bit)) & 1 != 0
    }

    #[inline]
    pub fn set_bit(&mut self, bit: u8, val: bool) {
        let mask = 1u32 << (31 - bit);
        let raw = if val { self.raw() | mask } else { self.raw() & !mask };
        *self = Self::from(raw);
    }

    pub fn field_from_ord(ord: std::cmp::Ordering) -> ConditionField {
        match ord {
            std::cmp::Ordering::Less => ConditionField::new().with_lt(true),
            std::cmp::Ordering::Equal => ConditionField::new().with_eq(true),
            std::cmp::Ordering::Greater => ConditionField::new().with_gt(true),
        }
    }

    #[inline]
    pub fn get_field(&self, index: u8) -> ConditionField {
        match index {
            0 => self.cr0(),
            1 => self.cr1(),
            2 => self.cr2(),
            3 => self.cr3(),
            4 => self.cr4(),
            5 => self.cr5(),
            6 => self.cr6(),
            7 => self.cr7(),
            _ => panic!("Invalid CR field index: {}", index),
        }
    }
}
