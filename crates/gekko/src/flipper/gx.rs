use super::pi::InterruptFlag;
use crate::gekko::Gekko;

pub struct Gx {
    pub raise_interrupt: bool,
}

impl Gx {
    pub fn new() -> Self {
        Gx {
            raise_interrupt: false,
        }
    }

    pub fn mmio_write_u8(&mut self, val: u8) {
        tracing::debug!(value = format!("{val:02X}"), "FIFO");
    }

    pub fn mmio_write_u16(&mut self, _val: u16) {
        tracing::debug!(value = format!("{_val:04X}"), "FIFO");
    }

    pub fn mmio_write_u32(&mut self, val: u32) {
        tracing::debug!(value = format!("{val:08X}"), "FIFO");

        if val == 0x45000002 {
            self.raise_interrupt = true;
            tracing::debug!("detected finish command?");
        }
    }
}

impl Gekko {
    /// Check if the GX stub detected a finish command and assert the PI interrupt
    pub fn check_gx_pe_finish(&mut self) {
        if self.gx.raise_interrupt {
            self.gx.raise_interrupt = false;
            self.pi.assert_interrupt(InterruptFlag::PeFinish);
        }
    }
}
