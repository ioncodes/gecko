use crate::flipper::si::pad;
use crate::hollywood::ipc::usb;
use crate::{GC, SystemId, WII};
use std::sync::{Arc, Mutex};

/// Live host input the emulator pulls from. The Wiimote report tick samples
/// this every report interval, so implementations should return the freshest
/// state available.
pub trait InputSink: Send {
    fn sample(&mut self) -> HostInput;

    fn set_rumble(&mut self, _port: usize, _on: bool) {}
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
        accel: Option<[f32; 3]>,
    },
}

pub fn aim_to_ir(aim_x: f32, aim_y: f32) -> (u16, u16) {
    const POINTER_SCALE_X: f64 = 0.44;
    const POINTER_SCALE_Y: f64 = 0.66;
    const POINTER_Y_OFFSET: f64 = 120.0;

    let aim_x = (aim_x as f64).clamp(0.0, 1.0);
    let aim_y = (aim_y as f64).clamp(0.0, 1.0);

    let span_x = usb::IR_CAMERA_WIDTH as f64 * POINTER_SCALE_X;
    let span_y = usb::IR_CAMERA_HEIGHT as f64 * POINTER_SCALE_Y;
    let base_x = (usb::IR_CAMERA_WIDTH as f64 - span_x) / 2.0;
    let base_y = (usb::IR_CAMERA_HEIGHT as f64 - span_y) / 2.0 + POINTER_Y_OFFSET;

    let ir_x = (base_x + (1.0 - aim_x) * span_x) as u16;
    let ir_y = (base_y + aim_y * span_y) as u16;

    (ir_x, ir_y)
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
            accel: None,
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
