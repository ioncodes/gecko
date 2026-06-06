use std::io;

use cranelift_jit::{BranchProtection, JITMemoryKind, JITMemoryProvider};
use cranelift_module::ModuleResult;

pub struct DenseArena {
    alloc: region::Allocation,
    offset: usize,
}

unsafe impl Send for DenseArena {}

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

        Ok(DenseArena { alloc, offset: 0 })
    }
}

impl JITMemoryProvider for DenseArena {
    fn allocate(&mut self, size: usize, align: u64, kind: JITMemoryKind) -> io::Result<*mut u8> {
        debug_assert!(matches!(kind, JITMemoryKind::Executable));

        let align = (align as usize).max(1);
        let start = (self.offset + align - 1) & !(align - 1);
        let end = start + size;
        if end > self.alloc.len() {
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, "JIT code arena exhausted"));
        }

        self.offset = end;
        Ok(unsafe { (self.alloc.as_mut_ptr::<u8>()).add(start) })
    }

    unsafe fn free_memory(&mut self) {}

    fn finalize(&mut self, _branch_protection: BranchProtection) -> ModuleResult<()> {
        Ok(())
    }
}
