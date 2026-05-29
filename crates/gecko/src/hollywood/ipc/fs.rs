pub mod host;
pub mod nand;

use crate::hollywood::ipc::{DeviceContext, IosDevice};
use std::path::{Path, PathBuf};

pub const IOCTL_GET_STATS: u32 = 0x2;
pub const IOCTL_CREATE_DIR: u32 = 0x3;
pub const IOCTL_READ_DIR: u32 = 0x4;
pub const IOCTL_GET_ATTR: u32 = 0x6;
pub const IOCTL_DELETE: u32 = 0x7;
pub const IOCTL_RENAME: u32 = 0x8;
pub const IOCTL_CREATE_FILE: u32 = 0x9;
pub const IOCTL_GET_USAGE: u32 = 0xC;

const FS_EINVAL: i32 = -101;
const FS_EEXIST: i32 = -105;
pub const FS_ENOENT: i32 = -106;
const FS_MAX_PATH: usize = 0x40;
const FS_DIRENT_NAME_LEN: usize = 0x13;
const FS_CREATE_INPUT_LEN: usize = 0x4A;
const FS_RENAME_INPUT_LEN: usize = FS_MAX_PATH * 2;
const FS_ATTR_OWNER_ID_OFFSET: u32 = 0x00;
const FS_ATTR_GROUP_ID_OFFSET: u32 = 0x04;
const FS_ATTR_PATH_OFFSET: u32 = 0x06;
const FS_ATTR_OWNER_PERM_OFFSET: u32 = 0x46;
const FS_ATTR_GROUP_PERM_OFFSET: u32 = 0x47;
const FS_ATTR_OTHER_PERM_OFFSET: u32 = 0x48;
const FS_ATTR_ATTRIBUTES_OFFSET: u32 = 0x49;
const FS_CREATE_PATH_OFFSET: u32 = FS_ATTR_PATH_OFFSET;
const FS_DEFAULT_OWNER_ID: u32 = 0;
const FS_DEFAULT_GROUP_ID: u16 = 0;
const FS_DEFAULT_PERM: u8 = 0x03;
const FS_DEFAULT_ATTRIBUTES: u8 = 0;
const FS_USAGE_CLUSTER_SIZE: u64 = 0x4000;
const FS_STATS_LEN: usize = 0x1C;
const FS_STATS_RESERVED_CLUSTERS: u32 = 0x0300;
const FS_STATS_USABLE_CLUSTERS: u32 = 0x7EC0 - FS_STATS_RESERVED_CLUSTERS;
const FS_STATS_TOTAL_INODES: u32 = 0x17FF;

pub struct FileSystem {
    host_root: PathBuf,
}

impl FileSystem {
    pub fn new(host_root: PathBuf) -> Self {
        Self { host_root }
    }
}

impl IosDevice for FileSystem {
    fn ioctl(
        &mut self,
        ctx: &mut DeviceContext<'_>,
        cmd: u32,
        in_ptr: u32,
        in_len: u32,
        out_ptr: u32,
        out_len: u32,
    ) -> i32 {
        match cmd {
            IOCTL_GET_STATS => self.get_stats(ctx, out_ptr, out_len),
            IOCTL_CREATE_DIR => self.create_dir(ctx, in_ptr, in_len),
            IOCTL_GET_ATTR => self.get_attr(ctx, in_ptr, in_len, out_ptr, out_len),
            IOCTL_DELETE => self.delete(ctx, in_ptr, in_len),
            IOCTL_RENAME => self.rename(ctx, in_ptr, in_len),
            IOCTL_CREATE_FILE => self.create_file(ctx, in_ptr, in_len),
            _ => {
                tracing::warn!(
                    device = &ctx.device_path,
                    cmd = format!("{cmd:#010X}"),
                    "FS: unimplemented ioctl"
                );
                FS_EINVAL
            }
        }
    }

    fn ioctlv(&mut self, ctx: &mut DeviceContext<'_>, cmd: u32, in_count: u32, io_count: u32, vec_ptr: u32) -> i32 {
        match cmd {
            IOCTL_READ_DIR => self.read_dir(ctx, in_count, io_count, vec_ptr),
            IOCTL_GET_USAGE => self.get_usage(ctx, in_count, io_count, vec_ptr),
            _ => {
                tracing::warn!(
                    device = &ctx.device_path,
                    cmd = format!("{cmd:#010X}"),
                    in_count,
                    io_count,
                    vec_ptr = format!("{vec_ptr:#010X}"),
                    "FS: unimplemented ioctlv"
                );
                FS_EINVAL
            }
        }
    }
}

impl FileSystem {
    fn read_dir(&self, ctx: &mut DeviceContext<'_>, in_count: u32, io_count: u32, vec_ptr: u32) -> i32 {
        let with_names = match (in_count, io_count) {
            (1, 1) => false,
            (2, 2) => true,
            _ => {
                tracing::warn!(in_count, io_count, "FS_ReadDir: unexpected vector counts");
                return FS_EINVAL;
            }
        };

        let path_ptr = self::vec_data(ctx, vec_ptr, 0);
        let path_len = self::vec_len(ctx, vec_ptr, 0) as usize;
        let count_idx = if with_names { 3 } else { 1 };
        let count_ptr = self::vec_data(ctx, vec_ptr, count_idx);
        let count_len = self::vec_len(ctx, vec_ptr, count_idx);

        if path_len < FS_MAX_PATH || count_len < 4 {
            tracing::warn!(path_len, count_len, "FS_ReadDir: invalid vector sizes");
            return FS_EINVAL;
        }

        let Some(path) = self::read_guest_path(ctx, path_ptr, FS_MAX_PATH) else {
            tracing::warn!(
                path_ptr = format!("{path_ptr:#010X}"),
                path_len,
                "FS_ReadDir: invalid path"
            );
            return FS_EINVAL;
        };

        let host_path = host::nand_to_host(&self.host_root, &path);
        let metadata = match std::fs::metadata(&host_path) {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(
                    %path,
                    host_path = format!("{}", host_path.display()),
                    %err,
                    "FS_ReadDir: path not found"
                );
                return FS_ENOENT;
            }
        };
        if !metadata.is_dir() {
            tracing::debug!(
                %path,
                host_path = format!("{}", host_path.display()),
                "FS_ReadDir: path is not a directory"
            );
            return FS_EINVAL;
        }

        let entries = match self::read_dir_names(&host_path) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!(
                    %path,
                    host_path = format!("{}", host_path.display()),
                    %err,
                    "FS_ReadDir: failed to list directory"
                );
                return FS_ENOENT;
            }
        };

        if !with_names {
            ctx.mmio.phys_write_u32(count_ptr, entries.len() as u32);
            tracing::debug!(
                %path,
                host_path = format!("{}", host_path.display()),
                entries = entries.len(),
                "FS_ReadDir: count"
            );
            return 0;
        }

        let max_count_ptr = self::vec_data(ctx, vec_ptr, 1);
        let max_count_len = self::vec_len(ctx, vec_ptr, 1);
        let names_ptr = self::vec_data(ctx, vec_ptr, 2);
        let names_len = self::vec_len(ctx, vec_ptr, 2);

        if max_count_len < 4 {
            tracing::warn!(max_count_len, "FS_ReadDir: invalid max count vector size");
            return FS_EINVAL;
        }

        let max_entries = ctx.mmio.phys_read_u32(max_count_ptr);
        let Some(required_names_len) = max_entries.checked_mul(FS_DIRENT_NAME_LEN as u32) else {
            tracing::warn!(max_entries, "FS_ReadDir: name output size overflow");
            return FS_EINVAL;
        };

        if names_len < required_names_len {
            tracing::warn!(
                names_len,
                required_names_len,
                "FS_ReadDir: name output vector too small"
            );
            return FS_EINVAL;
        }

        let returned_count = entries.len().min(max_entries as usize);
        {
            let out = ctx.mmio.phys_slice_mut(names_ptr, required_names_len as usize);
            self::write_dir_names(out, &entries[..returned_count]);
        }
        ctx.mmio.phys_write_u32(count_ptr, returned_count as u32);

        tracing::debug!(
            %path,
            host_path = format!("{}", host_path.display()),
            entries = entries.len(),
            returned = returned_count,
            "FS_ReadDir"
        );

        0
    }

    fn get_attr(&self, ctx: &mut DeviceContext<'_>, in_ptr: u32, in_len: u32, out_ptr: u32, out_len: u32) -> i32 {
        if (in_len as usize) < FS_MAX_PATH || (out_len as usize) < FS_CREATE_INPUT_LEN {
            tracing::warn!(in_len, out_len, "FS_GetAttr: invalid buffer size");
            return FS_EINVAL;
        }

        let Some(path) = self::read_guest_path(ctx, in_ptr, FS_MAX_PATH) else {
            tracing::warn!(in_ptr = format!("{in_ptr:#010X}"), "FS_GetAttr: invalid path");
            return FS_EINVAL;
        };

        let host_path = host::nand_to_host(&self.host_root, &path);
        if let Err(err) = std::fs::metadata(&host_path) {
            tracing::warn!(
                %path,
                host_path = format!("{}", host_path.display()),
                %err,
                "FS_GetAttr: path not found"
            );
            return FS_ENOENT;
        }

        self::write_attr(ctx, out_ptr, &path);

        tracing::debug!(
            %path,
            host_path = format!("{}", host_path.display()),
            "FS_GetAttr"
        );

        0
    }

    fn get_stats(&self, ctx: &mut DeviceContext<'_>, out_ptr: u32, out_len: u32) -> i32 {
        if (out_len as usize) < FS_STATS_LEN {
            tracing::warn!(out_len, "FS_GetStats: output buffer too small");
            return FS_EINVAL;
        }

        let (used_clusters, used_inodes) = self::path_usage(&self.host_root).unwrap_or((0, 0));
        let used_clusters = self::saturating_u32(used_clusters).min(FS_STATS_USABLE_CLUSTERS);
        let used_inodes = self::saturating_u32(used_inodes).min(FS_STATS_TOTAL_INODES);
        let free_clusters = FS_STATS_USABLE_CLUSTERS - used_clusters;
        let free_inodes = FS_STATS_TOTAL_INODES - used_inodes;

        ctx.mmio.phys_write_u32(out_ptr + 0x00, FS_USAGE_CLUSTER_SIZE as u32);
        ctx.mmio.phys_write_u32(out_ptr + 0x04, free_clusters);
        ctx.mmio.phys_write_u32(out_ptr + 0x08, used_clusters);
        ctx.mmio.phys_write_u32(out_ptr + 0x0C, 0); // bad clusters
        ctx.mmio.phys_write_u32(out_ptr + 0x10, FS_STATS_RESERVED_CLUSTERS);
        ctx.mmio.phys_write_u32(out_ptr + 0x14, free_inodes);
        ctx.mmio.phys_write_u32(out_ptr + 0x18, used_inodes);

        tracing::debug!(free_clusters, used_clusters, free_inodes, used_inodes, "FS_GetStats");

        0
    }

    fn get_usage(&self, ctx: &mut DeviceContext<'_>, in_count: u32, io_count: u32, vec_ptr: u32) -> i32 {
        if in_count != 1 || io_count != 2 {
            tracing::warn!(in_count, io_count, "FS_GetUsage: unexpected vector counts");
            return FS_EINVAL;
        }

        let path_ptr = self::vec_data(ctx, vec_ptr, 0);
        let path_len = self::vec_len(ctx, vec_ptr, 0) as usize;
        let used_clusters_ptr = self::vec_data(ctx, vec_ptr, 1);
        let used_clusters_len = self::vec_len(ctx, vec_ptr, 1);
        let used_inodes_ptr = self::vec_data(ctx, vec_ptr, 2);
        let used_inodes_len = self::vec_len(ctx, vec_ptr, 2);

        if path_len < FS_MAX_PATH || used_clusters_len < 4 || used_inodes_len < 4 {
            tracing::warn!(
                path_len,
                used_clusters_len,
                used_inodes_len,
                "FS_GetUsage: invalid vector sizes"
            );
            return FS_EINVAL;
        }

        let Some(path) = self::read_guest_path(ctx, path_ptr, FS_MAX_PATH) else {
            tracing::warn!(
                path_ptr = format!("{path_ptr:#010X}"),
                path_len,
                "FS_GetUsage: invalid path"
            );
            return FS_EINVAL;
        };

        let host_path = host::nand_to_host(&self.host_root, &path);
        let (used_clusters, used_inodes) = match self::path_usage(&host_path) {
            Ok((used_clusters, used_inodes)) => {
                (self::saturating_u32(used_clusters), self::saturating_u32(used_inodes))
            }
            Err(err) => {
                tracing::warn!(
                    %path,
                    host_path = format!("{}", host_path.display()),
                    %err,
                    "FS_GetUsage: failed"
                );
                return FS_ENOENT;
            }
        };

        ctx.mmio.phys_write_u32(used_clusters_ptr, used_clusters);
        ctx.mmio.phys_write_u32(used_inodes_ptr, used_inodes);

        tracing::debug!(
            %path,
            host_path = format!("{}", host_path.display()),
            used_clusters,
            used_inodes,
            "FS_GetUsage"
        );

        0
    }

    fn delete(&self, ctx: &mut DeviceContext<'_>, in_ptr: u32, in_len: u32) -> i32 {
        if (in_len as usize) < FS_MAX_PATH {
            tracing::warn!(in_len, "FS_Delete: input buffer too small");
            return FS_EINVAL;
        }

        let Some(path) = self::read_guest_path(ctx, in_ptr, FS_MAX_PATH) else {
            tracing::warn!(in_ptr = format!("{in_ptr:#010X}"), "FS_Delete: invalid path");
            return FS_EINVAL;
        };

        let host_path = host::nand_to_host(&self.host_root, &path);
        if std::fs::remove_file(&host_path)
            .or_else(|_| std::fs::remove_dir(&host_path))
            .is_ok()
        {
            tracing::debug!(
            %path,
            host_path = format!("{}", host_path.display()),
            "FS_Delete: success"
            );
            return 0;
        }

        tracing::warn!(
            %path,
            host_path = format!("{}", host_path.display()),
            "FS_Delete: failed to delete"
        );
        FS_ENOENT
    }

    fn rename(&self, ctx: &mut DeviceContext<'_>, in_ptr: u32, in_len: u32) -> i32 {
        if (in_len as usize) < FS_RENAME_INPUT_LEN {
            tracing::warn!(in_len, "FS_Rename: input buffer too small");
            return FS_EINVAL;
        }

        let Some(src_path) = self::read_guest_path(ctx, in_ptr, FS_MAX_PATH) else {
            tracing::warn!(in_ptr = format!("{in_ptr:#010X}"), "FS_Rename: invalid source path");
            return FS_EINVAL;
        };
        let dst_ptr = in_ptr + FS_MAX_PATH as u32;
        let Some(dst_path) = self::read_guest_path(ctx, dst_ptr, FS_MAX_PATH) else {
            tracing::warn!(
                dst_ptr = format!("{dst_ptr:#010X}"),
                "FS_Rename: invalid destination path"
            );
            return FS_EINVAL;
        };

        let src_host_path = host::nand_to_host(&self.host_root, &src_path);
        let dst_host_path = host::nand_to_host(&self.host_root, &dst_path);

        if !src_host_path.exists() {
            tracing::warn!(
                src = %src_path,
                src_host_path = format!("{}", src_host_path.display()),
                "FS_Rename: source does not exist"
            );
            return FS_ENOENT;
        }

        let mut replaced_existing = false;
        if dst_host_path.exists() {
            if self::same_host_path(&src_host_path, &dst_host_path) {
                tracing::debug!(
                    src = %src_path,
                    dst = %dst_path,
                    host_path = format!("{}", src_host_path.display()),
                    "FS_Rename: source and destination are the same"
                );
                return 0;
            }

            if let Err(err) = self::remove_host_path(&dst_host_path) {
                tracing::warn!(
                    src = %src_path,
                    dst = %dst_path,
                    dst_host_path = format!("{}", dst_host_path.display()),
                    %err,
                    "FS_Rename: failed to replace destination"
                );
                return FS_ENOENT;
            }

            replaced_existing = true;
        }

        match std::fs::rename(&src_host_path, &dst_host_path) {
            Ok(()) => {
                tracing::debug!(
                    src = %src_path,
                    dst = %dst_path,
                    src_host_path = format!("{}", src_host_path.display()),
                    dst_host_path = format!("{}", dst_host_path.display()),
                    replaced_existing,
                    "FS_Rename"
                );
                0
            }
            Err(err) => {
                tracing::warn!(
                    src = %src_path,
                    dst = %dst_path,
                    src_host_path = format!("{}", src_host_path.display()),
                    dst_host_path = format!("{}", dst_host_path.display()),
                    %err,
                    "FS_Rename: failed"
                );
                FS_ENOENT
            }
        }
    }

    fn create_dir(&self, ctx: &mut DeviceContext<'_>, in_ptr: u32, in_len: u32) -> i32 {
        if (in_len as usize) < FS_CREATE_INPUT_LEN {
            tracing::warn!(in_len, "FS_CreateDir: input buffer too small");
            return FS_EINVAL;
        }

        let Some(path) = self::read_guest_path(ctx, in_ptr + FS_CREATE_PATH_OFFSET, FS_MAX_PATH) else {
            tracing::warn!(in_ptr = format!("{in_ptr:#010X}"), "FS_CreateDir: invalid path");
            return FS_EINVAL;
        };

        let host_path = host::nand_to_host(&self.host_root, &path);
        match std::fs::create_dir(&host_path) {
            Ok(()) => {
                tracing::debug!(
                    %path,
                    host_path = format!("{}", host_path.display()),
                    "FS_CreateDir"
                );
                0
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                tracing::debug!(%path, "FS_CreateDir: already exists");
                FS_EEXIST
            }
            Err(err) => {
                tracing::warn!(
                    %path,
                    host_path = format!("{}", host_path.display()),
                    %err,
                    "FS_CreateDir: failed"
                );
                FS_ENOENT
            }
        }
    }

    fn create_file(&self, ctx: &mut DeviceContext<'_>, in_ptr: u32, in_len: u32) -> i32 {
        if (in_len as usize) < FS_CREATE_INPUT_LEN {
            tracing::warn!(in_len, "FS_CreateFile: input buffer too small");
            return FS_EINVAL;
        }

        let Some(path) = self::read_guest_path(ctx, in_ptr + FS_CREATE_PATH_OFFSET, FS_MAX_PATH) else {
            tracing::warn!(in_ptr = format!("{in_ptr:#010X}"), "FS_CreateFile: invalid path");
            return FS_EINVAL;
        };

        let host_path = host::nand_to_host(&self.host_root, &path);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&host_path)
        {
            Ok(_) => {
                tracing::debug!(
                    %path,
                    host_path = format!("{}", host_path.display()),
                    "FS_CreateFile"
                );
                0
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                tracing::debug!(%path, "FS_CreateFile: already exists");
                FS_EEXIST
            }
            Err(err) => {
                tracing::warn!(
                    %path,
                    host_path = format!("{}", host_path.display()),
                    %err,
                    "FS_CreateFile: failed"
                );
                FS_ENOENT
            }
        }
    }
}

fn read_dir_names(path: &Path) -> std::io::Result<Vec<String>> {
    let mut entries = std::fs::read_dir(path)?
        .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

fn write_dir_names(out: &mut [u8], entries: &[String]) {
    out.fill(0);

    for (slot, name) in out.chunks_exact_mut(FS_DIRENT_NAME_LEN).zip(entries) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(FS_DIRENT_NAME_LEN - 1);
        slot[..len].copy_from_slice(&bytes[..len]);
    }
}

fn write_attr(ctx: &mut DeviceContext<'_>, out_ptr: u32, path: &str) {
    {
        let out = ctx.mmio.phys_slice_mut(out_ptr, FS_CREATE_INPUT_LEN);
        out.fill(0);

        let path_start = FS_ATTR_PATH_OFFSET as usize;
        let bytes = path.as_bytes();
        let len = bytes.len().min(FS_MAX_PATH - 1);
        out[path_start..path_start + len].copy_from_slice(&bytes[..len]);
    }

    ctx.mmio
        .phys_write_u32(out_ptr + FS_ATTR_OWNER_ID_OFFSET, FS_DEFAULT_OWNER_ID);
    ctx.mmio
        .phys_write_u16(out_ptr + FS_ATTR_GROUP_ID_OFFSET, FS_DEFAULT_GROUP_ID);
    ctx.mmio
        .phys_write_u8(out_ptr + FS_ATTR_OWNER_PERM_OFFSET, FS_DEFAULT_PERM);
    ctx.mmio
        .phys_write_u8(out_ptr + FS_ATTR_GROUP_PERM_OFFSET, FS_DEFAULT_PERM);
    ctx.mmio
        .phys_write_u8(out_ptr + FS_ATTR_OTHER_PERM_OFFSET, FS_DEFAULT_PERM);
    ctx.mmio
        .phys_write_u8(out_ptr + FS_ATTR_ATTRIBUTES_OFFSET, FS_DEFAULT_ATTRIBUTES);
}

fn path_usage(path: &Path) -> std::io::Result<(u64, u64)> {
    let metadata = std::fs::metadata(path)?;
    let mut used_clusters = if metadata.is_file() {
        file_clusters(metadata.len())
    } else {
        0
    };
    let mut used_inodes = 1u64;

    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let (child_clusters, child_inodes) = self::path_usage(&entry?.path())?;
            used_clusters = used_clusters.saturating_add(child_clusters);
            used_inodes = used_inodes.saturating_add(child_inodes);
        }
    }

    Ok((used_clusters, used_inodes))
}

#[inline(always)]
fn remove_host_path(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    if metadata.is_dir() {
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    }
}

#[inline(always)]
fn same_host_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }

    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[inline(always)]
fn file_clusters(len: u64) -> u64 {
    if len == 0 {
        0
    } else {
        len.saturating_add(FS_USAGE_CLUSTER_SIZE - 1) / FS_USAGE_CLUSTER_SIZE
    }
}

#[inline(always)]
fn saturating_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}

#[inline(always)]
fn read_guest_path(ctx: &DeviceContext<'_>, ptr: u32, len: usize) -> Option<String> {
    if len == 0 {
        return None;
    }

    let bytes = ctx.mmio.phys_slice(ptr, len);
    let end = bytes.iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

#[inline(always)]
fn vec_data(ctx: &DeviceContext<'_>, vec_ptr: u32, idx: u32) -> u32 {
    ctx.mmio.phys_read_u32(vec_ptr + idx * 8)
}

#[inline(always)]
fn vec_len(ctx: &DeviceContext<'_>, vec_ptr: u32, idx: u32) -> u32 {
    ctx.mmio.phys_read_u32(vec_ptr + idx * 8 + 4)
}
