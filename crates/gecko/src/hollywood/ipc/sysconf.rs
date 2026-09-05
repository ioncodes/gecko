use crate::hollywood::ipc::fs::nand;
use crate::hollywood::ipc::{IPC_EINVAL, IosDevice};
use std::path::Path;

const SYSCONF_SIZE: usize = 0x4000;
const FOOTER_OFFSET: usize = 0x3FFC;

const SEEK_SET: i32 = 0;
const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

const IOCTL_GET_FILE_STATS: u32 = 0xB;

const TYPE_BIGARRAY: u8 = 1;
const TYPE_BYTE: u8 = 3;
const TYPE_LONG: u8 = 5;
const TYPE_BOOL: u8 = 7;

const BT_DINF_SLOT_SIZE: usize = 0x46;
const BT_DINF_REGISTERED_SLOTS: usize = 10;
const BT_DINF_ACTIVE_SLOTS: usize = 6;
const BT_DINF_TOTAL_SLOTS: usize = BT_DINF_REGISTERED_SLOTS + BT_DINF_ACTIVE_SLOTS;

pub struct SysConf {
    blob: Vec<u8>,
    pos: u64,
}

impl SysConf {
    pub fn new(host_fs_root: &Path) -> Self {
        Self::with_blob(host_fs_root, self::build_blob(1))
    }

    pub fn for_game_code(host_fs_root: &Path, game_code: [u8; 4]) -> Self {
        let region = nand::RegionSetting::from_game_code(game_code);
        Self::with_blob(host_fs_root, self::build_blob(region.language()))
    }

    fn with_blob(host_fs_root: &Path, blob: Vec<u8>) -> Self {
        let path = host_fs_root.join("shared2/sys/SYSCONF");
        if !path.exists() {
            nand::write_new(&path, &blob);
        }

        Self { blob, pos: 0 }
    }
}

impl IosDevice for SysConf {
    fn open(&mut self, _ctx: &mut super::DeviceContext<'_>, _mode: u32) -> i32 {
        self.pos = 0;
        0
    }

    fn save_state(&mut self, w: &mut crate::savestate::StateWriter) {
        w.bytes(&self.blob);
        w.u64(self.pos);
    }

    fn load_state(&mut self, r: &mut crate::savestate::StateReader<'_>) -> Result<(), crate::savestate::StateError> {
        r.bytes_into(&mut self.blob)?;
        self.pos = r.u64()?;
        Ok(())
    }

    fn read(&mut self, ctx: &mut super::DeviceContext<'_>, out_ptr: u32, out_len: u32) -> i32 {
        let start = self.pos as usize;
        let n = (out_len as usize).min(self.blob.len().saturating_sub(start));
        if n == 0 {
            return 0;
        }

        ctx.mmio
            .phys_slice_mut(out_ptr, n)
            .copy_from_slice(&self.blob[start..start + n]);
        self.pos += n as u64;

        n as i32
    }

    fn write(&mut self, _ctx: &mut super::DeviceContext<'_>, _in_ptr: u32, in_len: u32) -> i32 {
        // what the fuck
        in_len as i32
    }

    fn seek(&mut self, _ctx: &mut super::DeviceContext<'_>, offset: i32, whence: i32) -> i32 {
        let new_pos: i64 = match whence {
            SEEK_SET => offset as i64,
            SEEK_CUR => self.pos as i64 + offset as i64,
            SEEK_END => self.blob.len() as i64 + offset as i64,
            _ => return IPC_EINVAL,
        };

        if new_pos < 0 || new_pos > self.blob.len() as i64 {
            return IPC_EINVAL;
        }

        self.pos = new_pos as u64;
        self.pos as i32
    }

    fn ioctl(
        &mut self,
        ctx: &mut super::DeviceContext<'_>,
        cmd: u32,
        _in_ptr: u32,
        _in_len: u32,
        out_ptr: u32,
        _out_len: u32,
    ) -> i32 {
        if cmd == IOCTL_GET_FILE_STATS {
            ctx.mmio.phys_write_u32(out_ptr, self.blob.len() as u32);
            ctx.mmio.phys_write_u32(out_ptr + 4, self.pos as u32);
            return 0;
        }

        tracing::warn!(
            device = &ctx.device_path,
            cmd = format!("{cmd:#010X}"),
            out_buf = format!("{out_ptr:#010X}"),
            "SysConf: unimplemented ioctl"
        );
        IPC_EINVAL
    }
}

enum EntryData {
    Byte(u8),
    Bool(bool),
    Long(u32),
    BigArray(Vec<u8>),
}

impl EntryData {
    fn type_code(&self) -> u8 {
        match self {
            EntryData::Byte(_) => TYPE_BYTE,
            EntryData::Bool(_) => TYPE_BOOL,
            EntryData::Long(_) => TYPE_LONG,
            EntryData::BigArray(_) => TYPE_BIGARRAY,
        }
    }

    fn write_into(&self, dst: &mut [u8]) -> usize {
        match self {
            EntryData::Byte(b) => {
                dst[0] = *b;
                1
            }
            EntryData::Bool(b) => {
                dst[0] = u8::from(*b);
                1
            }
            EntryData::Long(v) => {
                dst[0..4].copy_from_slice(&v.to_be_bytes());
                4
            }
            EntryData::BigArray(data) => {
                let prefix = ((data.len() - 1) as u16).to_be_bytes();
                dst[0..2].copy_from_slice(&prefix);
                dst[2..2 + data.len()].copy_from_slice(data);
                2 + data.len()
            }
        }
    }
}

fn build_blob(language: u8) -> Vec<u8> {
    let entries: [(&[u8], EntryData); 11] = [
        (b"BT.DINF", EntryData::BigArray(self::build_bt_dinf())),
        (b"BT.MOT", EntryData::Byte(1)),
        (b"BT.SENS", EntryData::Long(3)),
        (b"BT.BAR", EntryData::Byte(1)),
        (b"BT.SPKV", EntryData::Byte(0x58)),
        (b"IPL.LNG", EntryData::Byte(language)),
        (b"IPL.AR", EntryData::Byte(0)),
        (b"IPL.E60", EntryData::Bool(false)),
        (b"IPL.PGS", EntryData::Bool(false)),
        (b"IPL.SND", EntryData::Byte(1)),
        (b"IPL.CD", EntryData::Bool(true)),
    ];

    let mut blob = vec![0u8; SYSCONF_SIZE];
    blob[0..4].copy_from_slice(b"SCv0");
    blob[4..6].copy_from_slice(&(entries.len() as u16).to_be_bytes());

    let dir_off = 0x06;
    let mut entry_off = dir_off + entries.len() * 2 + 2;

    for (i, (name, data)) in entries.iter().enumerate() {
        let dir_pos = dir_off + i * 2;
        blob[dir_pos..dir_pos + 2].copy_from_slice(&(entry_off as u16).to_be_bytes());

        blob[entry_off] = self::encode_type_namelen(data.type_code(), name.len());
        entry_off += 1;
        blob[entry_off..entry_off + name.len()].copy_from_slice(name);
        entry_off += name.len();
        entry_off += data.write_into(&mut blob[entry_off..]);
    }

    let past_last = dir_off + entries.len() * 2;
    blob[past_last..past_last + 2].copy_from_slice(&(entry_off as u16).to_be_bytes());

    blob[FOOTER_OFFSET..FOOTER_OFFSET + 4].copy_from_slice(b"SCed");
    blob
}

fn encode_type_namelen(type_code: u8, name_len: usize) -> u8 {
    (type_code << 5) | ((name_len - 1) as u8 & 0x1F)
}

fn build_bt_dinf() -> Vec<u8> {
    let mut wiimote_addr = super::usb::WIIMOTE_ADDR;
    wiimote_addr.reverse();

    let mut data = vec![0u8; 1 + BT_DINF_SLOT_SIZE * BT_DINF_TOTAL_SLOTS];
    data[0] = 1;

    let slot = &mut data[1..1 + BT_DINF_SLOT_SIZE];
    slot[0..6].copy_from_slice(&wiimote_addr);
    slot[6..6 + super::usb::WIIMOTE_NAME.len()].copy_from_slice(super::usb::WIIMOTE_NAME);

    data
}
