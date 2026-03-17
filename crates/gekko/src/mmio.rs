#[cfg(test)]
mod tests;

pub mod bus;
pub mod constants;
pub mod fp;
pub mod macros;
pub mod traits;

use constants::*;

pub struct Mmio {
    pub ram: Vec<u8>,
    pub efb: Vec<u8>,
    pub hwr: Vec<u8>,
    pub ipl: Vec<u8>,
}

impl Mmio {
    pub fn new() -> Self {
        Mmio {
            ram: vec![0; RAM_SIZE],
            efb: vec![0; EFB_SIZE],
            hwr: vec![0; HW_REG_SIZE],
            ipl: vec![0; 0],
        }
    }

    /// Resolve a physical address to a `(backing_slice, offset)` pair
    /// This is the one place that maps physical addresses to memory regions
    #[inline]
    fn resolve(&self, phys: u32) -> (&[u8], usize) {
        match phys {
            RAM_BASE..=RAM_END => (&self.ram, phys as usize),
            EFB_BASE..=EFB_END => (&self.efb, (phys - EFB_BASE) as usize),
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(phys_addr = format!("{:08X}", phys), "read from mmio");
                (&self.hwr, (phys - HW_REG_BASE) as usize)
            }
            IPL_BASE..=IPL_END => (&self.ipl, (phys - IPL_BASE) as usize),
            _ => {
                tracing::error!(phys_addr = format!("{:08X}", phys), "unmapped physical read");
                (&self.ram, 0)
            }
        }
    }

    /// Resolve a physical address to a `(backing_slice, offset)` pair
    /// This is the one place that maps physical addresses to memory regions
    /// Returns a mutable slice for write operations
    #[inline]
    fn resolve_mut(&mut self, phys: u32) -> (&mut [u8], usize) {
        match phys {
            RAM_BASE..=RAM_END => (&mut self.ram, phys as usize),
            EFB_BASE..=EFB_END => (&mut self.efb, (phys - EFB_BASE) as usize),
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(phys_addr = format!("{:08X}", phys), "write to mmio");
                (&mut self.hwr, (phys - HW_REG_BASE) as usize)
            }
            IPL_BASE..=IPL_END => (&mut self.ipl, (phys - IPL_BASE) as usize),
            _ => {
                tracing::error!(phys_addr = format!("{:08X}", phys), "unmapped physical write");
                (&mut self.ram, 0)
            }
        }
    }

    #[inline]
    pub fn phys_read_u8(&self, addr: u32) -> u8 {
        let (slice, offset) = self.resolve(addr);
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:02X}", slice[offset]),
            "read_u8"
        );
        slice[offset]
    }

    #[inline]
    pub fn phys_read_u16(&self, addr: u32) -> u16 {
        let (slice, offset) = self.resolve(addr);
        let value = u16::from_be_bytes(slice[offset..offset + 2].try_into().unwrap());
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:04X}", value),
            "read_u16"
        );
        value
    }

    #[inline]
    pub fn phys_read_u32(&self, addr: u32) -> u32 {
        let (slice, offset) = self.resolve(addr);
        let value = u32::from_be_bytes(slice[offset..offset + 4].try_into().unwrap());
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:08X}", value),
            "read_u32"
        );
        value
    }

    #[inline]
    pub fn phys_write_u8(&mut self, addr: u32, value: u8) {
        let (slice, offset) = self.resolve_mut(addr);
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:02X}", value),
            "write_u8"
        );
        slice[offset] = value;
    }

    #[inline]
    pub fn phys_write_u16(&mut self, addr: u32, value: u16) {
        let (slice, offset) = self.resolve_mut(addr);
        let bytes = value.to_be_bytes();
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:04X}", value),
            "write_u16"
        );
        slice[offset..offset + 2].copy_from_slice(&bytes);
    }

    #[inline]
    pub fn phys_write_u32(&mut self, addr: u32, value: u32) {
        let (slice, offset) = self.resolve_mut(addr);
        let bytes = value.to_be_bytes();
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:08X}", value),
            "write_u32"
        );
        slice[offset..offset + 4].copy_from_slice(&bytes);
    }

    #[inline]
    pub fn virt_read_u8(&self, addr: u32) -> u8 {
        self.phys_read_u8(Self::virt_to_phys(addr))
    }

    #[inline]
    pub fn virt_read_u16(&self, addr: u32) -> u16 {
        self.phys_read_u16(Self::virt_to_phys(addr))
    }

    #[inline]
    pub fn virt_read_u32(&self, addr: u32) -> u32 {
        self.phys_read_u32(Self::virt_to_phys(addr))
    }

    #[inline]
    pub fn virt_write_u8(&mut self, addr: u32, value: u8) {
        self.phys_write_u8(Self::virt_to_phys(addr), value);
    }

    #[inline]
    pub fn virt_write_u16(&mut self, addr: u32, value: u16) {
        self.phys_write_u16(Self::virt_to_phys(addr), value);
    }

    #[inline]
    pub fn virt_write_u32(&mut self, addr: u32, value: u32) {
        self.phys_write_u32(Self::virt_to_phys(addr), value);
    }

    /// Return a slice of physical memory starting at `addr` with length `len`
    /// Useful for bulk reads (e.g. disassembler)
    #[inline]
    pub fn phys_slice(&self, addr: u32, len: usize) -> &[u8] {
        let (slice, offset) = self.resolve(addr);
        &slice[offset..offset + len]
    }

    #[inline]
    pub fn phys_slice_mut(&mut self, addr: u32, len: usize) -> &mut [u8] {
        let (slice, offset) = self.resolve_mut(addr);
        &mut slice[offset..offset + len]
    }

    /// Return a slice of virtual memory starting at `addr` with length `len`
    /// This is just a thin wrapper around `phys_slice` that applies virtual-to-physical translation
    #[inline]
    pub fn virt_slice(&self, addr: u32, len: usize) -> &[u8] {
        self.phys_slice(Self::virt_to_phys(addr), len)
    }

    /// Read a typed MMIO register from its physical address
    pub fn read_register<T: traits::MmioRegister>(&self) -> T {
        let raw = match T::SIZE {
            1 => self.phys_read_u8(T::ADDR) as u32,
            2 => self.phys_read_u16(T::ADDR) as u32,
            4 => self.phys_read_u32(T::ADDR),
            _ => panic!("unsupported register size {}", T::SIZE),
        };
        T::from_raw(raw)
    }

    /// Write a typed MMIO register to its physical address
    pub fn write_register<T: traits::MmioRegister>(&mut self, value: T) {
        let raw = value.to_raw();
        match T::SIZE {
            1 => self.phys_write_u8(T::ADDR, raw as u8),
            2 => self.phys_write_u16(T::ADDR, raw as u16),
            4 => self.phys_write_u32(T::ADDR, raw),
            _ => panic!("unsupported register size {}", T::SIZE),
        }
    }

    // Simple virtual to physical translation that ignores caching and other MMIO features
    #[inline]
    pub const fn virt_to_phys(addr: u32) -> u32 {
        addr & 0x3FFFFFFF
    }
}
