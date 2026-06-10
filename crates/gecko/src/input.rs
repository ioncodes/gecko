use crate::flipper::si::pad;
use crate::hollywood::ipc::usb;
use crate::{GC, SystemId, WII};
use std::sync::{Arc, Mutex};

/// Live host input the emulator pulls from. The Wiimote report tick samples
/// this every report interval, so implementations should return the freshest
/// state available.
pub trait InputSink: Send {
    fn sample(&mut self) -> HostInput;
}

impl InputSink for Arc<Mutex<HostInput>> {
    fn sample(&mut self) -> HostInput {
        *self.lock().unwrap()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum HostInput {
    Gc(pad::PadStatus),
    Wii {
        wiimote_buttons: u16,
        wiimote_shake: bool,
        nunchuk_buttons: u8,
        nunchuk_stick_x: u8,
        nunchuk_stick_y: u8,
        ir_pointer: Option<(u16, u16)>,
    },
}

impl HostInput {
    pub fn gc_connected() -> Self {
        Self::Gc(pad::PadStatus {
            connected: true,
            ..pad::PadStatus::default()
        })
    }

    pub fn wii_neutral() -> Self {
        Self::Wii {
            wiimote_buttons: 0,
            wiimote_shake: false,
            nunchuk_buttons: 0,
            nunchuk_stick_x: usb::NUNCHUK_STICK_CENTER,
            nunchuk_stick_y: usb::NUNCHUK_STICK_CENTER,
            ir_pointer: None,
        }
    }

    pub fn neutral_for(system: SystemId) -> Self {
        match system {
            WII => Self::wii_neutral(),
            GC => Self::gc_connected(),
            _ => unreachable!(),
        }
    }
}
