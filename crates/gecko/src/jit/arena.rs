use std::io;
use std::sync::{Arc, Mutex};

use cranelift_jit::{BranchProtection, JITMemoryKind, JITMemoryProvider};
use cranelift_module::ModuleResult;

#[derive(Clone)]
pub struct DenseArena {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    alloc: region::Allocation,
    offset: usize,
    floor: usize,

    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    clean: usize,
}

unsafe impl Send for Inner {}

impl DenseArena {
    pub fn new(reserved: usize) -> io::Result<Self> {
        let alloc = region::alloc(reserved, region::Protection::READ_WRITE_EXECUTE).map_err(io::Error::other)?;

        #[cfg(target_os = "linux")]
        unsafe {
            libc::madvise(
                alloc.as_ptr::<u8>() as *mut libc::c_void,
                alloc.len(),
                libc::MADV_HUGEPAGE,
            );
        }

        let inner = Inner {
            alloc,
            offset: 0,
            floor: 0,

            #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
            clean: 0,
        };

        Ok(DenseArena {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    pub fn set_floor(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.floor = inner.offset;
    }

    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.offset = inner.floor;

        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        {
            inner.clean = inner.floor;
        }
    }
}

impl JITMemoryProvider for DenseArena {
    fn allocate(&mut self, size: usize, align: u64, _kind: JITMemoryKind) -> io::Result<*mut u8> {
        let mut inner = self.inner.lock().unwrap();

        let align = (align as usize).max(1);
        let start = (inner.offset + align - 1) & !(align - 1);
        let end = start + size;
        if end > inner.alloc.len() {
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, "JIT code arena exhausted"));
        }

        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        apple::set_writable();

        inner.offset = end;
        Ok(unsafe { inner.alloc.as_mut_ptr::<u8>().add(start) })
    }

    unsafe fn free_memory(&mut self) {}

    fn finalize(&mut self, _branch_protection: BranchProtection) -> ModuleResult<()> {
        #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
        {
            let mut inner = self.inner.lock().unwrap();

            let dirty = unsafe { inner.alloc.as_ptr::<u8>().add(inner.clean) };
            let len = inner.offset - inner.clean;

            apple::set_executable();
            apple::flush_icache(dirty, len);

            inner.clean = inner.offset;
        }

        Ok(())
    }
}

#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
mod apple {
    use core::ffi::c_void;

    unsafe extern "C" {
        fn sys_icache_invalidate(start: *const c_void, len: usize);
    }

    #[inline]
    pub fn set_writable() {
        unsafe { libc::pthread_jit_write_protect_np(0) };
    }

    #[inline]
    pub fn set_executable() {
        unsafe { libc::pthread_jit_write_protect_np(1) };
    }

    #[inline]
    pub fn flush_icache(start: *const u8, len: usize) {
        if len != 0 {
            unsafe { sys_icache_invalidate(start as *const c_void, len) };
        }
    }
}
