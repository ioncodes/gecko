pub trait MmioRw {
    const BASE: u32;
    const NAME: &'static str;

    fn read_raw(&mut self, addr: u32, access_size: u32) -> Option<u32>;
    fn write_raw(&mut self, addr: u32, access_size: u32, val: u32) -> bool;

    fn mmio_read_u8(&mut self, offset: u32) -> u8 {
        self.read_raw(Self::BASE + offset, 1).unwrap_or_else(|| {
            tracing::error!(
                peripheral = Self::NAME,
                offset = format!("{offset:08X}"),
                "unhandled mmio read_u8"
            );
            0
        }) as u8
    }

    fn mmio_write_u8(&mut self, offset: u32, val: u8) {
        if !self.write_raw(Self::BASE + offset, 1, val as u32) {
            tracing::error!(
                peripheral = Self::NAME,
                offset = format!("{offset:08X}"),
                value = format!("{val:02X}"),
                "unhandled mmio write_u8"
            );
        }
    }

    fn mmio_read_u16(&mut self, offset: u32) -> u16 {
        self.read_raw(Self::BASE + offset, 2).unwrap_or_else(|| {
            tracing::error!(
                peripheral = Self::NAME,
                offset = format!("{offset:08X}"),
                "unhandled mmio read_u16"
            );
            0
        }) as u16
    }

    fn mmio_write_u16(&mut self, offset: u32, val: u16) {
        if !self.write_raw(Self::BASE + offset, 2, val as u32) {
            tracing::error!(
                peripheral = Self::NAME,
                offset = format!("{offset:08X}"),
                value = format!("{val:04X}"),
                "unhandled mmio write_u16"
            );
        }
    }

    fn mmio_read_u32(&mut self, offset: u32) -> u32 {
        self.read_raw(Self::BASE + offset, 4).unwrap_or_else(|| {
            tracing::error!(
                peripheral = Self::NAME,
                offset = format!("{offset:08X}"),
                "unhandled mmio read_u32"
            );
            0
        })
    }

    fn mmio_write_u32(&mut self, offset: u32, val: u32) {
        if !self.write_raw(Self::BASE + offset, 4, val) {
            tracing::error!(
                peripheral = Self::NAME,
                offset = format!("{offset:08X}"),
                value = format!("{val:08X}"),
                "unhandled mmio write_u32"
            );
        }
    }
}

pub trait MmioRegister: Sized {
    const ADDR: u32;
    const SIZE: usize;

    fn from_raw(raw: u32) -> Self;
    fn to_raw(self) -> u32;

    fn contains(addr: u32) -> bool {
        addr >= Self::ADDR && addr < Self::ADDR + Self::SIZE as u32
    }

    fn fits(addr: u32, access_size: u32) -> bool {
        addr >= Self::ADDR && addr + access_size <= Self::ADDR + Self::SIZE as u32
    }

    /// Extract `access_size` bytes at `addr` from `raw` (big-endian register of `SIZE` bytes)
    fn read_sub(raw: u32, addr: u32, access_size: u32) -> u32 {
        let sub_offset = addr - Self::ADDR;
        let shift = (Self::SIZE as u32 - sub_offset - access_size) * 8;
        let mask = ((1u64 << (access_size * 8)) - 1) as u32;
        (raw >> shift) & mask
    }

    /// Return `full` with `access_size` bytes at `addr` replaced by `val` (big-endian)
    fn write_sub(full: u32, addr: u32, access_size: u32, val: u32) -> u32 {
        let sub_offset = addr - Self::ADDR;
        let shift = (Self::SIZE as u32 - sub_offset - access_size) * 8;
        let mask = ((1u64 << (access_size * 8)) - 1) as u32;
        (full & !(mask << shift)) | ((val & mask) << shift)
    }
}

pub trait MmioAccess<C>: MmioRegister {
    fn read(component: &C) -> Self;
    fn write(self, component: &mut C);

    /// Read `access_size` bytes from this register in `component` at physical address `addr`
    fn read_at(component: &mut C, addr: u32, access_size: u32) -> u32 {
        Self::read_sub(Self::read(component).to_raw(), addr, access_size)
    }

    /// Write `access_size` bytes at `addr` into this register in `component`
    /// Unaffected bytes in the register are preserved
    fn write_at(component: &mut C, addr: u32, access_size: u32, val: u32) {
        let merged = Self::write_sub(Self::read(component).to_raw(), addr, access_size, val);
        Self::from_raw(merged).write(component);
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WriteMask(u8);

impl WriteMask {
    pub const FULL_U8: Self = Self(0b0001);
    pub const FULL_U16: Self = Self(0b0011);
    pub const FULL_U32: Self = Self(0b1111);

    /// Build a mask from the byte offset within the register and the access size in bytes.
    #[inline(always)]
    pub const fn from_offset_size(sub_offset: u32, access_size: u32) -> Self {
        Self((((1u32 << access_size) - 1) << sub_offset) as u8)
    }

    /// Was the byte at `idx` (offset from the register base) written?
    #[inline(always)]
    pub const fn byte(self, idx: u32) -> bool {
        (self.0 >> idx) & 1 != 0
    }

    /// Did this write touch any byte in `[start, end]`?
    #[inline(always)]
    pub const fn any(self, start: u32, end: u32) -> bool {
        let m = (((1u32 << (end - start)) - 1) << start) as u8;
        self.0 & m != 0
    }

    #[inline(always)]
    pub const fn raw(self) -> u8 {
        self.0
    }
}

pub trait MmioHandler<C>: MmioRegister {
    fn read(c: &mut C) -> Self;
    fn write(self, c: &mut C, mask: WriteMask);

    #[inline(always)]
    fn read_at(c: &mut C, addr: u32, access_size: u32) -> u32 {
        Self::read_sub(Self::read(c).to_raw(), addr, access_size)
    }

    #[inline(always)]
    fn write_at(c: &mut C, addr: u32, access_size: u32, val: u32) {
        let current = Self::read(c).to_raw();
        let merged = Self::write_sub(current, addr, access_size, val);
        let mask = WriteMask::from_offset_size(addr - Self::ADDR, access_size);
        Self::from_raw(merged).write(c, mask);
    }
}
