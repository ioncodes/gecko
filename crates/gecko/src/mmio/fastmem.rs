use std::io;
use std::ops::{Deref, DerefMut};

use region::Protection;

use super::constants::*;
use super::{FASTMEM_PAGE_BYTES, FASTMEM_PAGE_SHIFT, PHYS_MASK, RAM_ALIASES};

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as os;

#[cfg(windows)]
use windows as os;

pub(crate) const GUEST_ADDRESS_SPACE: usize = 1 << 32;
pub(crate) const RESERVATION_BYTES: usize = GUEST_ADDRESS_SPACE + VIEW_GRANULARITY;
const VIEW_GRANULARITY: usize = 64 << 10;

pub(crate) fn enabled() -> bool {
    let disabled = std::env::var_os("GECKO_NO_FASTMEM").is_some_and(|value| value == "1");

    !cfg!(feature = "hooks") && !disabled
}

pub struct Memory {
    backing: Backing,
    len: usize,
}

enum Backing {
    Shared(os::Shared),
    Owned(Vec<u8>),
}

impl Memory {
    pub(crate) fn owned(len: usize) -> Self {
        Self {
            backing: Backing::Owned(vec![0; len]),
            len,
        }
    }

    fn shared(len: usize) -> io::Result<Self> {
        if len == 0 {
            return Ok(Self::owned(0));
        }

        let backing = os::Shared::new(len.next_multiple_of(VIEW_GRANULARITY))?;

        Ok(Self {
            backing: Backing::Shared(backing),
            len,
        })
    }
}

impl Deref for Memory {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.backing {
            Backing::Owned(data) => data,
            Backing::Shared(data) => unsafe { std::slice::from_raw_parts(data.address() as *const u8, self.len) },
        }
    }
}

impl DerefMut for Memory {
    fn deref_mut(&mut self) -> &mut [u8] {
        match &mut self.backing {
            Backing::Owned(data) => data,
            Backing::Shared(data) => unsafe { std::slice::from_raw_parts_mut(data.address() as *mut u8, self.len) },
        }
    }
}

struct View {
    ea: u32,
    len: usize,
    backing: usize,
}

impl View {
    fn contains(&self, ea: usize) -> bool {
        (self.ea as usize..self.ea as usize + self.len).contains(&ea)
    }
}

pub(crate) struct Fastmem {
    reservation: os::Reservation,
    views: Vec<View>,
    code_pages: Vec<u16>,
    host_page_bytes: usize,
}

impl Fastmem {
    pub(crate) fn allocate(wii: bool) -> io::Result<(Memory, Memory, Memory, Self)> {
        let host_page_bytes = region::page::size();
        if FASTMEM_PAGE_BYTES % host_page_bytes != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "host page size incompatible with fastmem",
            ));
        }

        let mem2_len = if wii { MEM2_SIZE } else { 0 };
        let ram = Memory::shared(RAM_SIZE)?;
        let mem2 = Memory::shared(mem2_len)?;
        let lcache = Memory::shared(LCACHE_SIZE)?;

        let mut mapping = Self {
            reservation: os::Reservation::new(RESERVATION_BYTES)?,
            views: Vec::new(),
            code_pages: vec![0; (RAM_SIZE + mem2_len) / host_page_bytes],
            host_page_bytes,
        };

        for alias in RAM_ALIASES {
            mapping.map(alias, &ram, true)?;
            if wii {
                mapping.map(MEM2_BASE | alias, &mem2, true)?;
            }
        }
        mapping.map(LCACHE_BASE, &lcache, false)?;

        Ok((ram, mem2, lcache, mapping))
    }

    fn map(&mut self, ea: u32, memory: &Memory, accessible: bool) -> io::Result<()> {
        let Backing::Shared(backing) = &memory.backing else {
            unreachable!("fastmem views need shared backing")
        };

        let len = memory.len.next_multiple_of(VIEW_GRANULARITY);
        self.reservation.map(ea as usize, len, backing)?;
        if !accessible {
            self.protect(ea as usize, len, Protection::NONE)?;
        }

        self.views.push(View {
            ea,
            len: memory.len,
            backing: memory.as_ptr() as usize,
        });

        Ok(())
    }

    pub(crate) fn base(&self) -> usize {
        self.reservation.base()
    }

    fn protect(&self, ea: usize, len: usize, protection: Protection) -> io::Result<()> {
        let address = (self.base() + ea) as *const u8;

        unsafe { region::protect(address, len, protection) }.map_err(io::Error::other)
    }

    pub(crate) fn set_page(&mut self, page: usize, host: usize) -> io::Result<()> {
        let ea = page << FASTMEM_PAGE_SHIFT;
        let Some(view) = self.views.iter().find(|view| view.contains(ea)) else {
            assert_eq!(host, 0, "LUT entry outside fastmem backing views");
            return Ok(());
        };

        assert!(
            host == 0 || host == view.backing + ea - view.ea as usize,
            "fastmem/LUT backing mismatch"
        );

        if host == 0 {
            return self.protect(ea, FASTMEM_PAGE_BYTES, Protection::NONE);
        }

        let protection = |offset: usize| {
            let phys = (ea + offset) as u32 & PHYS_MASK;
            let code = self.code_index(phys).is_some_and(|index| self.code_pages[index] != 0);

            if code { Protection::READ } else { Protection::READ_WRITE }
        };

        let mut start = 0;
        while start < FASTMEM_PAGE_BYTES {
            let run = protection(start);

            let mut end = start + self.host_page_bytes;
            while end < FASTMEM_PAGE_BYTES && protection(end) == run {
                end += self.host_page_bytes;
            }

            self.protect(ea + start, end - start, run)?;
            start = end;
        }

        Ok(())
    }

    fn code_index(&self, phys: u32) -> Option<usize> {
        let offset = if phys <= RAM_END {
            phys as usize
        } else if (MEM2_BASE..=MEM2_END).contains(&phys) {
            RAM_SIZE + (phys - MEM2_BASE) as usize
        } else {
            return None;
        };

        let index = offset / self.host_page_bytes;
        (index < self.code_pages.len()).then_some(index)
    }

    pub(crate) fn code_line_changed(&mut self, lut: &[usize], phys: u32, added: bool) {
        let Some(index) = self.code_index(phys) else {
            return;
        };

        let count = &mut self.code_pages[index];
        let was_code = *count != 0;
        if added {
            *count += 1;
        } else {
            *count = count.checked_sub(1).expect("code page refcount underflow");
        }

        let is_code = *count != 0;
        if was_code != is_code {
            self.protect_code_page(lut, phys, is_code);
        }
    }

    fn protect_code_page(&self, lut: &[usize], phys: u32, code: bool) {
        let page = phys as usize & !(self.host_page_bytes - 1);
        let protection = if code { Protection::READ } else { Protection::READ_WRITE };

        for alias in RAM_ALIASES {
            let ea = page | alias as usize;
            if lut[ea >> FASTMEM_PAGE_SHIFT] != 0 {
                self.protect(ea, self.host_page_bytes, protection)
                    .expect("cannot update JIT code page protection");
            }
        }
    }

    pub(crate) fn clear_code(&mut self, lut: &[usize]) {
        for index in 0..self.code_pages.len() {
            if self.code_pages[index] == 0 {
                continue;
            }

            self.code_pages[index] = 0;

            let offset = index * self.host_page_bytes;
            let phys = if offset < RAM_SIZE {
                offset
            } else {
                MEM2_BASE as usize + offset - RAM_SIZE
            };

            self.protect_code_page(lut, phys as u32, false);
        }
    }
}
