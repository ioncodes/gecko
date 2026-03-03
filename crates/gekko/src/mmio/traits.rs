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
    fn read_at(component: &C, addr: u32, access_size: u32) -> u32 {
        Self::read_sub(Self::read(component).to_raw(), addr, access_size)
    }

    /// Write `access_size` bytes at `addr` into this register in `component`
    /// Unaffected bytes in the register are preserved
    fn write_at(component: &mut C, addr: u32, access_size: u32, val: u32) {
        let merged = Self::write_sub(Self::read(component).to_raw(), addr, access_size, val);
        Self::from_raw(merged).write(component);
    }
}
