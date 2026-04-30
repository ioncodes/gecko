use crate::hollywood::ipc::{DeviceContext, IPC_EINVAL, IosDevice};
use zerocopy::byteorder::big_endian::U32;
use zerocopy::{FromBytes, Immutable, KnownLayout};

pub const IOCTL_DVD_LOW_UNENCRYPTED_READ: u32 = 0x8D;
pub const IOCTL_DVD_LOW_REQUEST_ERROR: u32 = 0xE0;
pub const IOCTL_DVD_LOW_REPORT_KEY: u32 = 0xA4;

const DI_RET_OK: i32 = 1;
const DI_RET_ERROR: i32 = 2;
const DI_RET_INVALID_RANGE: i32 = 0x20;
const DI_RET_BAD_ALIGNMENT: i32 = 0x80;

const DI_ERROR_OK: u32 = 0x00000000;
const DI_ERROR_LBA_OUT_OF_RANGE: u32 = 0x00052100;
const DI_ERROR_INVALID_COMMAND: u32 = 0x00052000;

pub struct DiskInterface {
    pub last_error: u32,
}

impl DiskInterface {
    pub fn new() -> Self {
        Self {
            last_error: DI_ERROR_OK,
        }
    }
}

impl IosDevice for DiskInterface {
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
            IOCTL_DVD_LOW_UNENCRYPTED_READ => self.dvd_low_unencrypted_read(ctx, in_ptr, out_ptr, out_len),
            IOCTL_DVD_LOW_REQUEST_ERROR => self.dvd_low_request_error(ctx, in_ptr, out_ptr, out_len),
            IOCTL_DVD_LOW_REPORT_KEY => self.dvd_low_report_key(ctx, in_ptr, out_ptr, out_len),
            _ => {
                tracing::warn!(
                    cmd = format!("{cmd:08X}"),
                    in_buf = format!("{:02X?}", ctx.mmio.phys_slice(in_ptr, in_len as usize)),
                    in_len,
                    out_buf = format!("{:02X?}", ctx.mmio.phys_slice(out_ptr, out_len as usize)),
                    out_len,
                    "Unknown IOCTL"
                );
                IPC_EINVAL
            }
        }
    }
}

impl DiskInterface {
    #[inline(always)]
    fn dvd_low_unencrypted_read(
        &mut self,
        ctx: &mut DeviceContext<'_>,
        in_ptr: u32,
        out_ptr: u32,
        out_len: u32,
    ) -> i32 {
        let input = ctx.mmio.phys_read_struct::<DvdLowUnencryptedRead>(in_ptr);
        let length = input.length.get();
        let pos_bytes = input.position_bytes();
        let end_bytes = pos_bytes + length as u64;

        if (out_ptr & 0x1F) != 0 || (length & 0x1F) != 0 {
            return DI_RET_BAD_ALIGNMENT;
        }

        if out_len < length {
            return DI_RET_BAD_ALIGNMENT;
        }

        let range_idx = UNENCRYPTED_RANGES
            .iter()
            .position(|r| pos_bytes >= r.start && end_bytes <= r.end);
        let Some(range_idx) = range_idx else {
            return DI_RET_INVALID_RANGE;
        };

        if range_idx == 0 {
            let Some(dvd) = ctx.di.dvd.as_ref() else {
                self.last_error = DI_ERROR_LBA_OUT_OF_RANGE;
                return DI_RET_ERROR;
            };

            let dst = ctx.mmio.phys_slice_mut(out_ptr, length as usize);
            dvd.read_raw_disc(pos_bytes as usize, dst);
            self.last_error = DI_ERROR_OK;

            tracing::debug!(
                pos = format!("{pos_bytes:#011X}"),
                len = format!("{length:#X}"),
                dst = format!("{out_ptr:#010X}"),
                "DVDLowUnencryptedRead"
            );

            DI_RET_OK
        } else {
            self.last_error = DI_ERROR_LBA_OUT_OF_RANGE;

            tracing::warn!(
                pos = format!("{pos_bytes:#011X}"),
                len = format!("{length:#X}"),
                range_idx,
                "DVDLowUnencryptedRead: LBA out of range, if you see this error it's likely a Nintendo anti-piracy check. Ignore it."
            );

            DI_RET_ERROR
        }
    }

    #[inline(always)]
    fn dvd_low_request_error(&mut self, ctx: &mut DeviceContext<'_>, _in_ptr: u32, out_ptr: u32, _out_len: u32) -> i32 {
        ctx.mmio.phys_write_u32(out_ptr, self.last_error);
        DI_RET_OK
    }

    #[inline(always)]
    fn dvd_low_report_key(&mut self, ctx: &mut DeviceContext<'_>, in_ptr: u32, _out_ptr: u32, _out_len: u32) -> i32 {
        let input = ctx.mmio.phys_read_struct::<DvdLowReportKey>(in_ptr);
        let param1 = input.param1;
        let param2 = input.param2.get();

        // Real retail drives reject this ioctl unconditionally. Nintendo titles
        // probe with (param1=4, param2=0) immediately after the unencrypted
        // read anti-piracy check and require return=2 with the drive error
        // set to "invalid command operation code".
        self.last_error = DI_ERROR_INVALID_COMMAND;

        tracing::warn!(
            param1 = format!("{param1:#04X}"),
            param2 = format!("{param2:#010X}"),
            "DVDLowReportKey"
        );

        DI_RET_ERROR
    }
}

#[repr(C, packed)]
#[derive(FromBytes, KnownLayout, Immutable)]
struct DvdLowUnencryptedRead {
    pub cmd: u8,
    _pad: [u8; 3],
    pub length: U32,
    pub position: U32,
}

impl DvdLowUnencryptedRead {
    fn position_bytes(&self) -> u64 {
        (self.position.get() as u64) << 2
    }
}

#[repr(C, packed)]
#[derive(FromBytes, KnownLayout, Immutable)]
struct DvdLowReportKey {
    pub cmd: u8,
    _pad: [u8; 6],
    pub param1: u8,
    pub param2: U32,
}

struct UnencryptedRange {
    start: u64,
    end: u64,
}

const UNENCRYPTED_RANGES: [UnencryptedRange; 3] = [
    UnencryptedRange {
        start: 0x0_0000_0000,
        end: 0x0_0005_0000,
    },
    UnencryptedRange {
        start: 0x1_1828_0000,
        end: 0x1_1828_0020,
    },
    UnencryptedRange {
        start: 0x1_FB50_0000,
        end: 0x1_FB50_0020,
    },
];
