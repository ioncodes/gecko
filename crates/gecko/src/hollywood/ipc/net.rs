use super::fs::nand;
use super::{DeviceContext, IPC_EINVAL, IosDevice};
use std::path::{Path, PathBuf};

const IOCTL_SUSPEND_SCHEDULER: u32 = 0x01;
const IOCTL_TRY_SUSPEND_SCHEDULER: u32 = 0x02;
const IOCTL_RESUME_SCHEDULER: u32 = 0x03;
const IOCTL_REQUEST_GENERATED_USER_ID: u32 = 0x0F;
const IOCTL_SET_RTC_COUNTER: u32 = 0x17;

const WC24_ID_REGISTERED: i32 = -36;
const WC24_ID_GENERATED: i32 = -35;

pub struct Wc24 {
    config: PathBuf,
}

impl Wc24 {
    pub fn new(root: &Path) -> Self {
        Self {
            config: root.join(nand::WC24_CONFIG_PATH),
        }
    }
}

impl IosDevice for Wc24 {
    fn ioctl(&mut self, ctx: &mut DeviceContext<'_>, cmd: u32, _: u32, _: u32, out: u32, len: u32) -> i32 {
        if out == 0 || len < 32 {
            return IPC_EINVAL;
        }

        match cmd {
            IOCTL_SUSPEND_SCHEDULER | IOCTL_TRY_SUSPEND_SCHEDULER | IOCTL_RESUME_SCHEDULER | IOCTL_SET_RTC_COUNTER => {
                ctx.mmio.phys_slice_mut(out, 32).fill(0);
            }
            IOCTL_REQUEST_GENERATED_USER_ID => {
                let Ok(config) = std::fs::read(&self.config) else {
                    return IPC_EINVAL;
                };
                if config.len() < 24 {
                    return IPC_EINVAL;
                }

                let status = if config[20..24] == 2u32.to_be_bytes() {
                    WC24_ID_REGISTERED
                } else {
                    WC24_ID_GENERATED
                };

                ctx.mmio.phys_slice_mut(out, 32).fill(0);
                ctx.mmio.phys_write_u32(out, status as u32);
                ctx.mmio.phys_slice_mut(out + 4, 8).copy_from_slice(&config[8..16]);
                ctx.mmio.phys_slice_mut(out + 12, 4).copy_from_slice(&config[20..24]);
            }
            _ => return IPC_EINVAL,
        }

        0
    }
}
