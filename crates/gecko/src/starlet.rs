use crate::system::SystemId;
use crate::{
    System,
    hollywood::ipc::{DeviceContext, IosDevice},
};
use std::collections::{HashMap, VecDeque};

// As per zayd
const FINALIZE_DELAY_CYCLES: u64 = 10_000;
const ACK_TO_NEXT_DELAY_CYCLES: u64 = 500;

pub struct PendingResponse {
    pub cmd_paddr: u32,
    pub result: i32,
}

pub struct Starlet {
    devices: HashMap<String, Box<dyn IosDevice>>,
    handles: HashMap<i32, String>,
    next_fd: i32,
    pub pending: VecDeque<PendingResponse>,
}

impl Starlet {
    pub fn new() -> Self {
        Starlet {
            devices: HashMap::new(),
            handles: HashMap::new(),
            next_fd: 1,
            pending: VecDeque::new(),
        }
    }

    /// Install a device at `path`. Replaces any existing registration.
    pub fn register(&mut self, path: &str, dev: Box<dyn IosDevice>) {
        self.devices.insert(path.to_owned(), dev);
    }

    /// Allocate a fresh fd and bind it to `path`.
    pub fn allocate_fd(&mut self, path: &str) -> i32 {
        let fd = self.next_fd;
        self.next_fd = self.next_fd.checked_add(1).expect("Starlet fd overflow");
        self.handles.insert(fd, path.to_owned());
        fd
    }

    /// Drop the fd to path mapping. Returns the path the fd was bound to.
    pub fn release_fd(&mut self, fd: i32) -> Option<String> {
        self.handles.remove(&fd)
    }

    pub fn device_for_path(&mut self, path: &str) -> Option<&mut Box<dyn IosDevice>> {
        self.devices.get_mut(path)
    }

    pub fn device_for_fd(&mut self, fd: i32) -> Option<&mut Box<dyn IosDevice>> {
        let path = self.handles.get(&fd)?.clone();
        self.devices.get_mut(&path)
    }
}

impl System<{ crate::WII }> {
    pub fn initialize_starlet_devices(&mut self) {
        use crate::hollywood::ipc::{self, stm};

        self.starlet.register("/dev/stm/immediate", Box::new(stm::Immediate));
        self.starlet.register("/dev/stm/eventhook", Box::new(stm::EventHook));
        self.starlet.register("/dev/fs", Box::new(ipc::fs::FileSystem));
        self.starlet.register("/dev/di", Box::new(ipc::di::DiskInterface));
    }

    pub fn create_device_context(&mut self) -> (&mut Starlet, DeviceContext<'_>) {
        (
            &mut self.starlet,
            DeviceContext {
                mmio: &mut self.mmio,
                scheduler: &mut self.scheduler,
            },
        )
    }
}

pub fn dispatch_command<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, cmd_paddr: u32) {
    let result = self::process_command(sys, cmd_paddr);

    sys.starlet.pending.push_back(PendingResponse { cmd_paddr, result });
    sys.scheduler
        .schedule_in(FINALIZE_DELAY_CYCLES, self::deliver_pending::<SYSTEM>);
}

pub fn schedule_drain<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.scheduler
        .schedule_in(ACK_TO_NEXT_DELAY_CYCLES, self::deliver_pending::<SYSTEM>);
}

fn process_command<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, cmd_paddr: u32) -> i32 {
    use crate::hollywood::ipc::{IPC_EINVAL, IPC_ENOENT};

    const IOS_OPEN: u32 = 1;
    const IOS_CLOSE: u32 = 2;
    const IOS_READ: u32 = 3;
    const IOS_WRITE: u32 = 4;
    const IOS_SEEK: u32 = 5;
    const IOS_IOCTL: u32 = 6;
    const IOS_IOCTLV: u32 = 7;

    assert!(SYSTEM == crate::WII, "Starlet dispatch reached on non-Wii system");

    let wii: &mut crate::Wii = unsafe { ::core::mem::transmute(sys) };
    let cmd = wii.mmio.phys_read_u32(cmd_paddr);
    let fd = wii.mmio.phys_read_u32(cmd_paddr + 0x08) as i32;

    let (starlet, mut ctx) = wii.create_device_context();

    match cmd {
        IOS_OPEN => {
            let path_ptr = ctx.mmio.phys_read_u32(cmd_paddr + 0x0C);
            let mode = ctx.mmio.phys_read_u32(cmd_paddr + 0x10);
            let path = self::read_c_string(&mut ctx, path_ptr);
            tracing::info!(%path, mode, "IOS_Open");

            let Some(dev) = starlet.device_for_path(&path) else {
                tracing::error!(%path, "IOS_Open: no device registered");
                return IPC_ENOENT;
            };
            let rc = dev.open(&mut ctx, mode);
            if rc >= 0 { starlet.allocate_fd(&path) } else { rc }
        }
        IOS_CLOSE => {
            tracing::info!(fd, "IOS_Close");

            let Some(path) = starlet.release_fd(fd) else {
                return IPC_EINVAL;
            };
            let Some(dev) = starlet.device_for_path(&path) else {
                return IPC_EINVAL;
            };

            dev.close(&mut ctx)
        }
        IOS_READ => {
            let buf = ctx.mmio.phys_read_u32(cmd_paddr + 0x0C);
            let len = ctx.mmio.phys_read_u32(cmd_paddr + 0x10);

            tracing::info!(fd, buf = format!("{buf:#010X}"), len, "IOS_Read");

            match starlet.device_for_fd(fd) {
                Some(dev) => dev.read(&mut ctx, buf, len),
                None => IPC_EINVAL,
            }
        }
        IOS_WRITE => {
            let buf = ctx.mmio.phys_read_u32(cmd_paddr + 0x0C);
            let len = ctx.mmio.phys_read_u32(cmd_paddr + 0x10);

            tracing::info!(fd, buf = format!("{buf:#010X}"), len, "IOS_Write");

            match starlet.device_for_fd(fd) {
                Some(dev) => dev.write(&mut ctx, buf, len),
                None => IPC_EINVAL,
            }
        }
        IOS_SEEK => {
            let where_ = ctx.mmio.phys_read_u32(cmd_paddr + 0x0C) as i32;
            let whence = ctx.mmio.phys_read_u32(cmd_paddr + 0x10) as i32;

            tracing::info!(fd, where_, whence, "IOS_Seek");

            match starlet.device_for_fd(fd) {
                Some(dev) => dev.seek(&mut ctx, where_, whence),
                None => IPC_EINVAL,
            }
        }
        IOS_IOCTL => {
            let ioctl_cmd = ctx.mmio.phys_read_u32(cmd_paddr + 0x0C);
            let in_buf = ctx.mmio.phys_read_u32(cmd_paddr + 0x10);
            let in_len = ctx.mmio.phys_read_u32(cmd_paddr + 0x14);
            let out_buf = ctx.mmio.phys_read_u32(cmd_paddr + 0x18);
            let out_len = ctx.mmio.phys_read_u32(cmd_paddr + 0x1C);

            tracing::info!(fd, ioctl_cmd, "IOS_Ioctl");

            match starlet.device_for_fd(fd) {
                Some(dev) => dev.ioctl(&mut ctx, ioctl_cmd, in_buf, in_len, out_buf, out_len),
                None => IPC_EINVAL,
            }
        }
        IOS_IOCTLV => {
            let ioctl_cmd = ctx.mmio.phys_read_u32(cmd_paddr + 0x0C);
            let argcin = ctx.mmio.phys_read_u32(cmd_paddr + 0x10);
            let argcio = ctx.mmio.phys_read_u32(cmd_paddr + 0x14);
            let vec = ctx.mmio.phys_read_u32(cmd_paddr + 0x18);

            tracing::info!(fd, ioctl_cmd, "IOS_Ioctlv");

            match starlet.device_for_fd(fd) {
                Some(dev) => dev.ioctlv(&mut ctx, ioctl_cmd, argcin, argcio, vec),
                None => IPC_EINVAL,
            }
        }
        other => {
            tracing::error!(cmd = other, "unimplemented IOS command");
            IPC_EINVAL
        }
    }
}

fn read_c_string(ctx: &mut DeviceContext<'_>, paddr: u32) -> String {
    let mut bytes = Vec::with_capacity(64);

    for i in 0..64 {
        let b = ctx.mmio.phys_read_u8(paddr + i);
        if b == 0 {
            break;
        }

        bytes.push(b);
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

fn deliver_pending<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if sys.hollywood.ipc.ppcctrl.arm_response() {
        // PPC slot still occupied. Wait for the ack.
        return;
    }

    let Some(p) = sys.starlet.pending.pop_front() else {
        return;
    };
    crate::hollywood::ipc::deliver_response(sys, p.cmd_paddr, p.result);
}
