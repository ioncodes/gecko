pub mod bus;
pub mod constants;
pub mod fp;
pub mod macros;
pub mod traits;

use crate::system::{SystemId, WII};
use constants::*;

pub const FASTMEM_LUT_PAGES: usize = 1 << 15;
pub const FASTMEM_PAGE_BYTES: usize = 1 << 17;
pub const FASTMEM_PAGE_MASK: u32 = (FASTMEM_PAGE_BYTES as u32) - 1;
pub const FASTMEM_PAGE_SHIFT: u32 = 17;

pub struct Mmio<const SYSTEM: SystemId> {
    pub ram: Vec<u8>,
    pub efb: Vec<u8>,
    pub hwr: Vec<u8>,
    pub ipl: Vec<u8>,
    pub lcache: Vec<u8>,
    pub mem2: Vec<u8>,
    pub ram_ptr: usize,
    pub fastmem_lut: Vec<usize>,
    pub fastmem_lut_ptr: usize,
}

/// Read-only view over MEM1 plus (on Wii) MEM2, addressed by physical
/// address. Lets GP code that doesn't care about the bank just pass the
/// view through and call `.slice(addr, len)` to resolve.
pub struct RamView<'a> {
    pub mem1: &'a [u8],
    pub mem2: &'a [u8],
}

impl<'a> RamView<'a> {
    /// Resolve `[addr..addr+len]` to a slice in whichever bank holds it.
    /// Returns `None` when the range is outside both banks or crosses a
    /// bank boundary.
    #[inline(always)]
    pub fn slice(&self, addr: usize, len: usize) -> Option<&'a [u8]> {
        let end = addr.checked_add(len)?;
        if addr < self.mem1.len() {
            (end <= self.mem1.len()).then(|| &self.mem1[addr..end])
        } else if (MEM2_BASE as usize..).contains(&addr) {
            let off = addr - MEM2_BASE as usize;
            let end_off = off.checked_add(len)?;
            (end_off <= self.mem2.len()).then(|| &self.mem2[off..end_off])
        } else {
            None
        }
    }
}

/// Mutable counterpart of `RamView`. Used by EFB writeback into RAM.
pub struct RamViewMut<'a> {
    pub mem1: &'a mut [u8],
    pub mem2: &'a mut [u8],
}

impl<'a> RamViewMut<'a> {
    #[inline(always)]
    pub fn as_view(&self) -> RamView<'_> {
        RamView {
            mem1: self.mem1,
            mem2: self.mem2,
        }
    }

    #[inline(always)]
    pub fn slice_mut(&mut self, addr: usize, len: usize) -> Option<&mut [u8]> {
        let end = addr.checked_add(len)?;
        if addr < self.mem1.len() {
            (end <= self.mem1.len()).then(|| &mut self.mem1[addr..end])
        } else if (MEM2_BASE as usize..).contains(&addr) {
            let off = addr - MEM2_BASE as usize;
            let end_off = off.checked_add(len)?;
            (end_off <= self.mem2.len()).then(|| &mut self.mem2[off..end_off])
        } else {
            None
        }
    }
}

impl<const SYSTEM: SystemId> Mmio<SYSTEM> {
    pub fn new() -> Self {
        let ram = vec![0u8; RAM_SIZE];
        let ram_ptr = ram.as_ptr() as usize;
        let mem2 = if SYSTEM == WII { vec![0; MEM2_SIZE] } else { Vec::new() };

        let mut fastmem_lut = vec![0usize; FASTMEM_LUT_PAGES];

        // Map MEM1 (24 MiB) at all three aliases:
        //   * physical 0x0000_0000..0x017F_FFFF
        //   * cached   0x8000_0000..0x817F_FFFF
        //   * uncached 0xC000_0000..0xC17F_FFFF
        let mem1_pages = (RAM_SIZE / FASTMEM_PAGE_BYTES) as u32;
        let bases = [0x0000_0000u32, 0x8000_0000, 0xC000_0000];
        for base in bases {
            let lut_base = (base >> FASTMEM_PAGE_SHIFT) as usize;
            for i in 0..mem1_pages {
                let host_addr = ram_ptr + (i as usize) * FASTMEM_PAGE_BYTES;
                fastmem_lut[lut_base + i as usize] = host_addr;
            }
        }

        // Wii: also map MEM2 (64 MiB) at its three aliases:
        //   * physical 0x1000_0000..0x13FF_FFFF
        //   * cached   0x9000_0000..0x93FF_FFFF
        //   * uncached 0xD000_0000..0xD3FF_FFFF
        if SYSTEM == WII {
            let mem2_pages = (MEM2_SIZE / FASTMEM_PAGE_BYTES) as u32;
            let mem2_ptr = mem2.as_ptr() as usize;
            let bases = [0x1000_0000u32, 0x9000_0000, 0xD000_0000];
            for base in bases {
                let lut_base = (base >> FASTMEM_PAGE_SHIFT) as usize;
                for i in 0..mem2_pages {
                    let host_addr = mem2_ptr + (i as usize) * FASTMEM_PAGE_BYTES;
                    fastmem_lut[lut_base + i as usize] = host_addr;
                }
            }
        }

        let fastmem_lut_ptr = fastmem_lut.as_ptr() as usize;

        Mmio {
            ram,
            efb: vec![0; EFB_SIZE],
            hwr: vec![0; HW_REG_SIZE],
            ipl: vec![0; 0],
            lcache: vec![0; LCACHE_SIZE],
            mem2,
            ram_ptr,
            fastmem_lut,
            fastmem_lut_ptr,
        }
    }

    /// Resolve a physical address to a `(backing_slice, offset)` pair
    /// This is the one place that maps physical addresses to memory regions
    #[inline(always)]
    fn resolve(&self, phys: u32) -> (&[u8], usize) {
        match phys {
            RAM_BASE..=RAM_END => (&self.ram, phys as usize),
            EFB_BASE..=EFB_END => (&self.efb, (phys - EFB_BASE) as usize),
            HW_REG_BASE..=HW_REG_END => (&self.hwr, (phys - HW_REG_BASE) as usize),
            IPL_BASE..=IPL_END => (&self.ipl, (phys - IPL_BASE) as usize),
            LCACHE_BASE..=LCACHE_END => (&self.lcache, (phys - LCACHE_BASE) as usize),
            MEM2_BASE..=MEM2_END if SYSTEM == WII => (&self.mem2, (phys - MEM2_BASE) as usize),
            _ => {
                tracing::error!(phys_addr = format!("{:08X}", phys), "unmapped physical read");
                (&self.ram, 0)
            }
        }
    }

    /// Resolve a physical address to a `(backing_slice, offset)` pair
    /// This is the one place that maps physical addresses to memory regions
    /// Returns a mutable slice for write operations
    #[inline(always)]
    fn resolve_mut(&mut self, phys: u32) -> (&mut [u8], usize) {
        match phys {
            RAM_BASE..=RAM_END => (&mut self.ram, phys as usize),
            EFB_BASE..=EFB_END => (&mut self.efb, (phys - EFB_BASE) as usize),
            HW_REG_BASE..=HW_REG_END => (&mut self.hwr, (phys - HW_REG_BASE) as usize),
            IPL_BASE..=IPL_END => (&mut self.ipl, (phys - IPL_BASE) as usize),
            LCACHE_BASE..=LCACHE_END => (&mut self.lcache, (phys - LCACHE_BASE) as usize),
            MEM2_BASE..=MEM2_END if SYSTEM == WII => (&mut self.mem2, (phys - MEM2_BASE) as usize),
            _ => {
                tracing::error!(phys_addr = format!("{:08X}", phys), "unmapped physical write");
                (&mut self.ram, 0)
            }
        }
    }

    #[inline(always)]
    pub fn phys_read_u8(&self, addr: u32) -> u8 {
        let (slice, offset) = self.resolve(addr);
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:02X}", slice[offset]),
            "read_u8"
        );
        slice[offset]
    }

    #[inline(always)]
    pub fn phys_read_u16(&self, addr: u32) -> u16 {
        let (slice, offset) = self.resolve(addr);
        let value = unsafe { read_be_u16_unchecked(slice.as_ptr().add(offset)) };
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:04X}", value),
            "read_u16"
        );
        value
    }

    #[inline(always)]
    pub fn phys_read_u32(&self, addr: u32) -> u32 {
        let (slice, offset) = self.resolve(addr);
        let value = unsafe { read_be_u32_unchecked(slice.as_ptr().add(offset)) };
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:08X}", value),
            "read_u32"
        );
        value
    }

    #[inline(always)]
    pub fn phys_write_u8(&mut self, addr: u32, value: u8) {
        let (slice, offset) = self.resolve_mut(addr);
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:02X}", value),
            "write_u8"
        );
        slice[offset] = value;
    }

    #[inline(always)]
    pub fn phys_write_u16(&mut self, addr: u32, value: u16) {
        let (slice, offset) = self.resolve_mut(addr);
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:04X}", value),
            "write_u16"
        );
        unsafe { write_be_u16_unchecked(slice.as_mut_ptr().add(offset), value) };
    }

    #[inline(always)]
    pub fn phys_write_u32(&mut self, addr: u32, value: u32) {
        let (slice, offset) = self.resolve_mut(addr);
        tracing::trace!(
            phys_addr = format!("{:08X}", addr),
            value = format!("{:08X}", value),
            "write_u32"
        );
        unsafe { write_be_u32_unchecked(slice.as_mut_ptr().add(offset), value) };
    }

    #[inline(always)]
    pub fn virt_read_u8(&self, addr: u32) -> u8 {
        self.phys_read_u8(virt_to_phys(addr))
    }

    #[inline(always)]
    pub fn virt_read_u16(&self, addr: u32) -> u16 {
        self.phys_read_u16(virt_to_phys(addr))
    }

    #[inline(always)]
    pub fn virt_read_u32(&self, addr: u32) -> u32 {
        self.phys_read_u32(virt_to_phys(addr))
    }

    #[inline(always)]
    pub fn virt_write_u8(&mut self, addr: u32, value: u8) {
        self.phys_write_u8(virt_to_phys(addr), value);
    }

    #[inline(always)]
    pub fn virt_write_u16(&mut self, addr: u32, value: u16) {
        self.phys_write_u16(virt_to_phys(addr), value);
    }

    #[inline(always)]
    pub fn virt_write_u32(&mut self, addr: u32, value: u32) {
        self.phys_write_u32(virt_to_phys(addr), value);
    }

    /// Return a slice of physical memory starting at `addr` with length `len`
    /// Useful for bulk reads (e.g. disassembler)
    #[inline(always)]
    pub fn phys_slice(&self, addr: u32, len: usize) -> &[u8] {
        let (slice, offset) = self.resolve(addr);
        &slice[offset..offset + len]
    }

    #[inline(always)]
    pub fn phys_slice_mut(&mut self, addr: u32, len: usize) -> &mut [u8] {
        let (slice, offset) = self.resolve_mut(addr);
        &mut slice[offset..offset + len]
    }

    /// Read-only view spanning MEM1 and (on Wii) MEM2. Used by the GP path
    /// so texture / vertex / TLUT reads can resolve addresses in either bank.
    #[inline(always)]
    pub fn ram_view(&self) -> RamView<'_> {
        RamView {
            mem1: &self.ram,
            mem2: &self.mem2,
        }
    }

    /// Mutable counterpart.
    #[inline(always)]
    pub fn ram_view_mut(&mut self) -> RamViewMut<'_> {
        RamViewMut {
            mem1: &mut self.ram,
            mem2: &mut self.mem2,
        }
    }

    /// Copy `size_of::<T>()` bytes starting at `addr` into a fresh `T`. The
    /// caller is responsible for declaring `T` with `#[repr(C)]` and using
    /// big-endian wrappers (e.g. `zerocopy::byteorder::big_endian::U32`) for
    /// any multibyte integer fields, since PPC memory is big-endian.
    #[inline(always)]
    pub fn phys_read_struct<T>(&self, addr: u32) -> T
    where
        T: zerocopy::FromBytes + zerocopy::KnownLayout + zerocopy::Immutable,
    {
        let bytes = self.phys_slice(addr, core::mem::size_of::<T>());
        T::read_from_bytes(bytes).expect("phys_read_struct: layout error")
    }

    /// Return a slice of virtual memory starting at `addr` with length `len`
    /// This is just a thin wrapper around `phys_slice` that applies virtual-to-physical translation
    #[inline(always)]
    pub fn virt_slice(&self, addr: u32, len: usize) -> &[u8] {
        self.phys_slice(virt_to_phys(addr), len)
    }

    #[inline(always)]
    pub fn virt_slice_mut(&mut self, addr: u32, len: usize) -> &mut [u8] {
        self.phys_slice_mut(virt_to_phys(addr), len)
    }

    /// Read a typed MMIO register from its physical address
    #[inline(always)]
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
    #[inline(always)]
    pub fn write_register<T: traits::MmioRegister>(&mut self, value: T) {
        let raw = value.to_raw();
        match T::SIZE {
            1 => self.phys_write_u8(T::ADDR, raw as u8),
            2 => self.phys_write_u16(T::ADDR, raw as u16),
            4 => self.phys_write_u32(T::ADDR, raw),
            _ => panic!("unsupported register size {}", T::SIZE),
        }
    }

    /// Process a locked cache DMA transfer triggered by writing to SPR DMAL (923).
    pub fn process_locked_cache_dma(&mut self, dmau: &crate::gekko::spr::DmaUpper, dmal: &crate::gekko::spr::DmaLower) {
        let ram_addr = dmau.ram_addr() << 5;
        let lcache_vaddr = dmal.lcache_addr() << 5;
        let lcache_paddr = (lcache_vaddr - LCACHE_BASE) as usize;
        let block_count = ((dmau.length_hi() as usize) << 2) | dmal.length_lo() as usize;
        let length = if block_count == 0 { 128 } else { block_count } * 32;

        tracing::debug!(
            ram_addr = format!("{ram_addr:08X}"),
            lcache_vaddr = format!("{lcache_vaddr:08X}"),
            lcache_paddr = format!("{lcache_paddr:08X}"),
            length,
            direction = if dmal.load() { "mem -> lcache" } else { "lcache -> mem" },
            "locked cache DMA"
        );

        if dmal.load() {
            let src = self.virt_slice(ram_addr, length).to_vec();
            self.lcache[lcache_paddr..lcache_paddr + length].copy_from_slice(&src);
        } else {
            let src = self.lcache[lcache_paddr..lcache_paddr + length].to_vec();
            self.virt_slice_mut(ram_addr, length).copy_from_slice(&src);
        }
    }

    #[inline(always)]
    pub fn fetch_instruction(&self, addr: u32) -> u32 {
        let phys = virt_to_phys(addr);
        if phys <= RAM_END - 3 {
            self.ram_read_u32(phys)
        } else {
            self.phys_read_u32(phys)
        }
    }

    #[inline(always)]
    pub fn ram_read_u8(&self, phys: u32) -> u8 {
        let value = self.ram[phys as usize];
        tracing::trace!(
            phys_addr = format!("{:08X}", phys),
            value = format!("{:02X}", value),
            "read_u8"
        );
        value
    }

    #[inline(always)]
    pub fn ram_read_u16(&self, phys: u32) -> u16 {
        debug_assert!(phys <= RAM_END - 1);
        let value = unsafe { read_be_u16_unchecked(self.ram.as_ptr().add(phys as usize)) };
        tracing::trace!(
            phys_addr = format!("{:08X}", phys),
            value = format!("{:04X}", value),
            "read_u16"
        );
        value
    }

    #[inline(always)]
    pub fn ram_read_u32(&self, phys: u32) -> u32 {
        debug_assert!(phys <= RAM_END - 3);
        let value = unsafe { read_be_u32_unchecked(self.ram.as_ptr().add(phys as usize)) };
        tracing::trace!(
            phys_addr = format!("{:08X}", phys),
            value = format!("{:08X}", value),
            "read_u32"
        );
        value
    }

    #[inline(always)]
    pub fn ram_read_u64(&self, phys: u32) -> u64 {
        debug_assert!(phys <= RAM_END - 7);
        unsafe { read_be_u64_unchecked(self.ram.as_ptr().add(phys as usize)) }
    }

    #[inline(always)]
    pub fn ram_write_u8(&mut self, phys: u32, value: u8) {
        tracing::trace!(
            phys_addr = format!("{:08X}", phys),
            value = format!("{:02X}", value),
            "write_u8"
        );
        self.ram[phys as usize] = value;
    }

    #[inline(always)]
    pub fn ram_write_u16(&mut self, phys: u32, value: u16) {
        debug_assert!(phys <= RAM_END - 1);
        tracing::trace!(
            phys_addr = format!("{:08X}", phys),
            value = format!("{:04X}", value),
            "write_u16"
        );
        unsafe { write_be_u16_unchecked(self.ram.as_mut_ptr().add(phys as usize), value) };
    }

    #[inline(always)]
    pub fn ram_write_u32(&mut self, phys: u32, value: u32) {
        debug_assert!(phys <= RAM_END - 3);
        tracing::trace!(
            phys_addr = format!("{:08X}", phys),
            value = format!("{:08X}", value),
            "write_u32"
        );
        unsafe { write_be_u32_unchecked(self.ram.as_mut_ptr().add(phys as usize), value) };
    }

    #[inline(always)]
    pub fn ram_write_u64(&mut self, phys: u32, value: u64) {
        debug_assert!(phys <= RAM_END - 7);
        unsafe { write_be_u64_unchecked(self.ram.as_mut_ptr().add(phys as usize), value) };
    }
}

#[inline(always)]
pub const fn virt_to_phys(addr: u32) -> u32 {
    addr & 0x3FFFFFFF
}

#[inline(always)]
unsafe fn read_be_u16_unchecked(ptr: *const u8) -> u16 {
    u16::from_be(unsafe { std::ptr::read_unaligned(ptr.cast::<u16>()) })
}

#[inline(always)]
unsafe fn read_be_u32_unchecked(ptr: *const u8) -> u32 {
    u32::from_be(unsafe { std::ptr::read_unaligned(ptr.cast::<u32>()) })
}

#[inline(always)]
unsafe fn read_be_u64_unchecked(ptr: *const u8) -> u64 {
    u64::from_be(unsafe { std::ptr::read_unaligned(ptr.cast::<u64>()) })
}

#[inline(always)]
unsafe fn write_be_u16_unchecked(ptr: *mut u8, value: u16) {
    unsafe { std::ptr::write_unaligned(ptr.cast::<u16>(), value.to_be()) };
}

#[inline(always)]
unsafe fn write_be_u32_unchecked(ptr: *mut u8, value: u32) {
    unsafe { std::ptr::write_unaligned(ptr.cast::<u32>(), value.to_be()) };
}

#[inline(always)]
unsafe fn write_be_u64_unchecked(ptr: *mut u8, value: u64) {
    unsafe { std::ptr::write_unaligned(ptr.cast::<u64>(), value.to_be()) };
}
