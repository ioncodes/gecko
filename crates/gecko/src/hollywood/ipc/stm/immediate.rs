use crate::hollywood::ipc::{DeviceContext, IPC_EINVAL, IosDevice};

const IOCTL_VIDIMMING: u32 = 0x5001;

pub struct Immediate;

impl IosDevice for Immediate {
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
            IOCTL_VIDIMMING => 0,
            _ => {
                tracing::warn!(
                    cmd = format!("{cmd:#010X}"),
                    in_buf = format!("{:02X?}", ctx.mmio.phys_slice(in_ptr, in_len as usize)),
                    in_len,
                    out_buf = format!("{:02X?}", ctx.mmio.phys_slice(out_ptr, out_len as usize)),
                    out_len,
                    "STM_Immediate: unimplemented ioctl"
                );
                IPC_EINVAL
            }
        }
    }
}
