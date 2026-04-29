pub struct Stm;

impl super::IosDevice for Stm {
    fn open(&mut self, _ctx: &mut super::DeviceContext<'_>, mode: u32) -> i32 {
        tracing::error!(mode, "IOS_Open: unimplemented");
        0
    }

    fn close(&mut self, _ctx: &mut super::DeviceContext<'_>) -> i32 {
        tracing::error!("IOS_Close: unimplemented");
        0
    }

    fn read(&mut self, _ctx: &mut super::DeviceContext<'_>, buf: u32, len: u32) -> i32 {
        tracing::error!(buf, len, "IOS_Read: unimplemented");
        super::IPC_EINVAL
    }

    fn write(&mut self, _ctx: &mut super::DeviceContext<'_>, buf: u32, len: u32) -> i32 {
        tracing::error!(buf, len, "IOS_Write: unimplemented");
        super::IPC_EINVAL
    }

    fn seek(&mut self, _ctx: &mut super::DeviceContext<'_>, where_: i32, whence: i32) -> i32 {
        tracing::error!(where_, whence, "IOS_Seek: unimplemented");
        0
    }

    fn ioctl(
        &mut self,
        _ctx: &mut super::DeviceContext<'_>,
        cmd: u32,
        in_buf: u32,
        in_len: u32,
        out_buf: u32,
        out_len: u32,
    ) -> i32 {
        tracing::error!(cmd, in_buf, in_len, out_buf, out_len, "IOS_Ioctl: unimplemented");
        super::IPC_EINVAL
    }

    fn ioctlv(&mut self, _ctx: &mut super::DeviceContext<'_>, cmd: u32, argcin: u32, argcio: u32, vec: u32) -> i32 {
        tracing::error!(cmd, argcin, argcio, vec, "IOS_Ioctlv: unimplemented");
        super::IPC_EINVAL
    }
}
