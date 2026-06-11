pub mod gc;
pub mod wii;

use crate::device::Button;
use gecko::flipper::si::pad;
use gecko::hollywood::ipc::usb;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PointerSource {
    Auto,
    Gyro,
    RightStick,
    Touchpad,
    Mouse,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Invert {
    pub x: bool,
    pub y: bool,
}

impl Invert {
    pub fn apply(self, v: (f32, f32)) -> (f32, f32) {
        (if self.x { -v.0 } else { v.0 }, if self.y { -v.1 } else { v.1 })
    }
}

pub struct GcProfile {
    pub buttons: Vec<(Button, u16)>,
    pub stick_invert: Invert,
    pub substick_invert: Invert,
    pub deadzone: f32,
}

impl Default for GcProfile {
    fn default() -> Self {
        Self {
            buttons: vec![
                (Button::South, pad::A),
                (Button::East, pad::X),
                (Button::West, pad::B),
                (Button::North, pad::Y),
                (Button::Start, pad::START),
                (Button::R1, pad::Z),
                (Button::DpadUp, pad::DPAD_UP),
                (Button::DpadDown, pad::DPAD_DOWN),
                (Button::DpadLeft, pad::DPAD_LEFT),
                (Button::DpadRight, pad::DPAD_RIGHT),
            ],
            stick_invert: Invert::default(),
            substick_invert: Invert::default(),
            deadzone: 0.12,
        }
    }
}

pub struct WiiProfile {
    pub buttons: Vec<(Button, u16)>,
    pub nunchuk_c: Button,
    pub nunchuk_z: Button,
    pub shake: Option<Button>,
    pub recenter: Button,
    pub pointer: PointerSource,
    pub sensitivity: f32,
    pub sideways: bool,
    pub stick_dpad: bool,
    pub nunchuk_invert: Invert,
    pub stick_dpad_invert: Invert,
    pub pointer_invert: Invert,
    pub deadzone: f32,
}

impl Default for WiiProfile {
    fn default() -> Self {
        Self {
            buttons: vec![
                (Button::South, usb::BTN_A),
                (Button::R2, usb::BTN_B),
                (Button::West, usb::BTN_ONE),
                (Button::North, usb::BTN_TWO),
                (Button::Start, usb::BTN_PLUS),
                (Button::Select, usb::BTN_MINUS),
                (Button::Guide, usb::BTN_HOME),
                (Button::DpadUp, usb::BTN_UP),
                (Button::DpadDown, usb::BTN_DOWN),
                (Button::DpadLeft, usb::BTN_LEFT),
                (Button::DpadRight, usb::BTN_RIGHT),
            ],
            nunchuk_c: Button::L1,
            nunchuk_z: Button::L2,
            shake: Some(Button::R1),
            recenter: Button::R3,
            pointer: PointerSource::Auto,
            sensitivity: 1.0,
            sideways: false,
            stick_dpad: true,
            nunchuk_invert: Invert::default(),
            stick_dpad_invert: Invert::default(),
            pointer_invert: Invert::default(),
            deadzone: 0.12,
        }
    }
}

pub fn radial(v: (f32, f32), deadzone: f32) -> (f32, f32) {
    let mag = (v.0 * v.0 + v.1 * v.1).sqrt();

    if mag <= deadzone {
        return (0.0, 0.0);
    }

    let scale = ((mag - deadzone) / (1.0 - deadzone)).min(1.0) / mag;

    (v.0 * scale, v.1 * scale)
}

pub fn stick_byte(v: f32) -> u8 {
    (v * 127.0 + 128.0).clamp(0.0, 255.0) as u8
}

pub fn trigger_byte(v: f32) -> u8 {
    (v * 255.0).clamp(0.0, 255.0) as u8
}
