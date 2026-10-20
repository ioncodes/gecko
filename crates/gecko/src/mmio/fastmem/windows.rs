use std::ffi::c_void;
use std::{io, ptr};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::*;

type Alloc =
    unsafe extern "system" fn(HANDLE, *const c_void, usize, u32, u32, *mut MEM_EXTENDED_PARAMETER, u32) -> *mut c_void;
type Map = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *const c_void,
    u64,
    usize,
    u32,
    u32,
    *mut MEM_EXTENDED_PARAMETER,
    u32,
) -> MEMORY_MAPPED_VIEW_ADDRESS;

pub(super) struct Shared {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    pub(super) fn new(len: usize) -> io::Result<Self> {
        unsafe {
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                ptr::null(),
                PAGE_READWRITE,
                0,
                len as u32,
                ptr::null(),
            );
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }

            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, len);
            if view.Value.is_null() {
                let error = io::Error::last_os_error();
                CloseHandle(handle);
                return Err(error);
            }
            Ok(Self { handle, view })
        }
    }

    pub(super) fn address(&self) -> usize {
        self.view.Value as usize
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.view);
            CloseHandle(self.handle);
        }
    }
}

struct Part {
    offset: usize,
    len: usize,
    mapped: bool,
}

pub(super) struct Reservation {
    base: usize,
    parts: Vec<Part>,
    map_view: Map,
}

impl Reservation {
    pub(super) fn new(len: usize) -> io::Result<Self> {
        unsafe {
            let module = GetModuleHandleA(c"KernelBase.dll".as_ptr().cast());
            let alloc = GetProcAddress(module, c"VirtualAlloc2".as_ptr().cast())
                .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "VirtualAlloc2 unavailable"))?;
            let map = GetProcAddress(module, c"MapViewOfFile3".as_ptr().cast())
                .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "MapViewOfFile3 unavailable"))?;
            let alloc: Alloc = std::mem::transmute(alloc);
            let map_view: Map = std::mem::transmute(map);
            let base = alloc(
                ptr::null_mut(),
                ptr::null(),
                len,
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS,
                ptr::null_mut(),
                0,
            );
            if base.is_null() {
                return Err(io::Error::last_os_error());
            }

            Ok(Self {
                base: base as usize,
                parts: vec![Part {
                    offset: 0,
                    len,
                    mapped: false,
                }],
                map_view,
            })
        }
    }

    pub(super) fn base(&self) -> usize {
        self.base
    }

    fn split(&mut self, offset: usize) -> io::Result<()> {
        let Some(index) = self
            .parts
            .iter()
            .position(|p| offset > p.offset && offset < p.offset + p.len)
        else {
            return Ok(());
        };

        let part = &self.parts[index];
        assert!(!part.mapped);

        let first = offset - part.offset;
        let rest = part.len - first;
        if unsafe {
            VirtualFree(
                (self.base + part.offset) as *mut c_void,
                first,
                MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }

        self.parts[index].len = first;
        self.parts.insert(
            index + 1,
            Part {
                offset,
                len: rest,
                mapped: false,
            },
        );

        Ok(())
    }

    pub(super) fn map(&mut self, offset: usize, len: usize, shared: &Shared) -> io::Result<()> {
        self.split(offset)?;
        self.split(offset + len)?;

        let part = self.parts.iter_mut().find(|p| p.offset == offset).unwrap();
        assert!(!part.mapped && part.len == len);

        let view = unsafe {
            (self.map_view)(
                shared.handle,
                ptr::null_mut(),
                (self.base + offset) as *const c_void,
                0,
                len,
                MEM_REPLACE_PLACEHOLDER,
                PAGE_READWRITE,
                ptr::null_mut(),
                0,
            )
        };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error());
        }

        part.mapped = true;
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        for part in &self.parts {
            unsafe {
                let address = (self.base + part.offset) as *mut c_void;
                if part.mapped {
                    UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: address });
                } else {
                    VirtualFree(address, 0, MEM_RELEASE);
                }
            }
        }
    }
}
