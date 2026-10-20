use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::{io, ptr};

pub(super) struct Shared {
    fd: OwnedFd,
    address: usize,
    len: usize,
}

impl Shared {
    pub(super) fn new(len: usize) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        let fd = unsafe { libc::memfd_create(c"gecko-fastmem".as_ptr(), libc::MFD_CLOEXEC) };

        #[cfg(not(target_os = "linux"))]
        let fd = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let mut result = -1;
            for _ in 0..128 {
                let name = std::ffi::CString::new(format!(
                    "/gecko-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ))
                .unwrap();
                result = unsafe { libc::shm_open(name.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, 0o600) };
                if result >= 0 {
                    unsafe {
                        libc::shm_unlink(name.as_ptr());
                    }
                    break;
                }
                if io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                    break;
                }
            }
            result
        };

        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let address = unsafe {
            libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            fd,
            address: address as usize,
            len,
        })
    }

    pub(super) fn address(&self) -> usize {
        self.address
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.address as *mut libc::c_void, self.len);
        }
    }
}

pub(super) struct Reservation {
    base: usize,
    len: usize,
}

impl Reservation {
    pub(super) fn new(len: usize) -> io::Result<Self> {
        let base = unsafe {
            libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            base: base as usize,
            len,
        })
    }

    pub(super) fn base(&self) -> usize {
        self.base
    }

    pub(super) fn map(&mut self, offset: usize, len: usize, shared: &Shared) -> io::Result<()> {
        assert!(offset <= self.len && len <= self.len - offset);
        let address = unsafe {
            libc::mmap(
                (self.base + offset) as *mut libc::c_void,
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_FIXED,
                shared.fd.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, self.len);
        }
    }
}
