pub mod gpio;
pub mod ipc;
pub mod irq;
pub mod regs;

pub struct Hollywood {
    pub gpio: gpio::Gpio,
    pub ipc: ipc::Ipc,
    pub irq: irq::Irq,
}

impl Hollywood {
    pub fn new() -> Self {
        Hollywood {
            gpio: gpio::Gpio::new(),
            ipc: ipc::Ipc::new(),
            irq: irq::Irq::new(),
        }
    }
}
