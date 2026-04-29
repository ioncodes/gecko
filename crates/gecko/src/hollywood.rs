pub mod ipc;
pub mod irq;
pub mod regs;

pub struct Hollywood {
    pub ipc: ipc::Ipc,
    pub irq: irq::Irq,
}

impl Hollywood {
    pub fn new() -> Self {
        Hollywood {
            ipc: ipc::Ipc::new(),
            irq: irq::Irq::new(),
        }
    }
}
