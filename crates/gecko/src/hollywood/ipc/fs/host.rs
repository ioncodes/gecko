use crate::hollywood::ipc::{DeviceContext, IPC_EINVAL, IosDevice};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const SEEK_SET: i32 = 0;
const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

const IOCTL_GET_FILE_STATS: u32 = 0xB;

pub struct HostBackedFile {
    path: PathBuf,
    file: File,
    mode: u32,
}

impl HostBackedFile {
    pub fn try_open(host_root: &Path, nand_path: &str, mode: u32) -> Option<Self> {
        let host_path = self::nand_to_host(host_root, nand_path);
        if !host_path.is_file() {
            return None;
        }

        let mut opts = OpenOptions::new();
        match mode {
            0 | 1 => {
                opts.read(true);
            }
            2 => {
                opts.write(true);
            }
            _ => {
                opts.read(true).write(true);
            }
        };

        match opts.open(&host_path) {
            Ok(file) => Some(Self {
                path: host_path,
                file,
                mode,
            }),
            Err(err) => {
                tracing::warn!(
                    path = format!("{}", host_path.display()),
                    %err,
                    "HostBackedFile: open failed"
                );
                None
            }
        }
    }

    pub fn reopen(host_root: &Path, nand_path: &str, mode: u32, offset: u64) -> Option<Self> {
        let mut this = Self::try_open(host_root, nand_path, mode)?;

        if let Err(err) = this.file.seek(SeekFrom::Start(offset)) {
            tracing::warn!(path = format!("{}", this.path.display()), %err, "HostBackedFile: reopen seek failed");
        }

        Some(this)
    }
}

impl IosDevice for HostBackedFile {
    fn read(&mut self, ctx: &mut DeviceContext<'_>, out_ptr: u32, out_len: u32) -> i32 {
        let mut buf = vec![0u8; out_len as usize];
        let n = match self.file.read(&mut buf) {
            Ok(n) => n,
            Err(err) => {
                tracing::warn!(path = format!("{}", self.path.display()), %err, "HostBackedFile: read failed");
                return IPC_EINVAL;
            }
        };
        ctx.mmio.phys_slice_mut(out_ptr, n).copy_from_slice(&buf[..n]);
        n as i32
    }

    fn write(&mut self, ctx: &mut DeviceContext<'_>, in_ptr: u32, in_len: u32) -> i32 {
        let buf = ctx.mmio.phys_slice(in_ptr, in_len as usize).to_vec();
        match self.file.write_all(&buf) {
            Ok(()) => in_len as i32,
            Err(err) => {
                tracing::warn!(path = format!("{}", self.path.display()), %err, "HostBackedFile: write failed");
                IPC_EINVAL
            }
        }
    }

    fn seek(&mut self, _ctx: &mut DeviceContext<'_>, offset: i32, whence: i32) -> i32 {
        let target = match whence {
            SEEK_SET => SeekFrom::Start(offset as u64),
            SEEK_CUR => SeekFrom::Current(offset as i64),
            SEEK_END => SeekFrom::End(offset as i64),
            _ => {
                tracing::warn!(whence, "HostBackedFile: invalid whence");
                return IPC_EINVAL;
            }
        };

        match self.file.seek(target) {
            Ok(pos) => pos as i32,
            Err(err) => {
                tracing::warn!(path = format!("{}", self.path.display()), %err, "HostBackedFile: seek failed");
                IPC_EINVAL
            }
        }
    }

    fn ioctl(
        &mut self,
        ctx: &mut DeviceContext<'_>,
        cmd: u32,
        _in_ptr: u32,
        _in_len: u32,
        out_ptr: u32,
        _out_len: u32,
    ) -> i32 {
        match cmd {
            IOCTL_GET_FILE_STATS => {
                let size = self.file.metadata().map(|m| m.len()).unwrap_or(0) as u32;
                let pos = self.file.stream_position().unwrap_or(0) as u32;
                ctx.mmio.phys_write_u32(out_ptr, size);
                ctx.mmio.phys_write_u32(out_ptr + 4, pos);
                0
            }
            _ => {
                tracing::warn!(
                    device = &ctx.device_path,
                    path = format!("{}", self.path.display()),
                    cmd = format!("{cmd:#X}"),
                    "HostBackedFile: unhandled ioctl"
                );
                IPC_EINVAL
            }
        }
    }

    fn host_file_state(&mut self) -> Option<(u32, u64)> {
        let offset = self.file.stream_position().unwrap_or(0);
        Some((self.mode, offset))
    }
}

pub(super) fn nand_to_host(host_root: &Path, nand_path: &str) -> PathBuf {
    let trimmed = nand_path.trim_start_matches('/');
    host_root.join(trimmed)
}
