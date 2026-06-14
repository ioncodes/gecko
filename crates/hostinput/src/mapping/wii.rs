use crate::device::PortInput;
use crate::mapping::{self, PointerSource, WiiProfile};
use crate::motion;
use gecko::HostInput;
use gecko::hollywood::ipc::usb;

const STICK_DPAD_THRESHOLD: f32 = 0.5;

pub fn map(port: &PortInput, profile: &WiiProfile, source: PointerSource) -> HostInput {
    let state = &port.state;

    let mut buttons = 0u16;
    for (button, mask) in &profile.buttons {
        if state.pressed(*button) {
            buttons |= mask;
        }
    }

    if profile.stick_dpad && source != PointerSource::RightStick {
        let (rx, ry) = profile.stick_dpad_invert.apply(state.right);

        if ry >= STICK_DPAD_THRESHOLD {
            buttons |= usb::BTN_UP;
        }
        if ry <= -STICK_DPAD_THRESHOLD {
            buttons |= usb::BTN_DOWN;
        }
        if rx <= -STICK_DPAD_THRESHOLD {
            buttons |= usb::BTN_LEFT;
        }
        if rx >= STICK_DPAD_THRESHOLD {
            buttons |= usb::BTN_RIGHT;
        }
    }

    if profile.sideways {
        buttons = self::rotate_dpad(buttons);
    }

    let mut nunchuk_buttons = 0u8;
    if state.pressed(profile.nunchuk_c) {
        nunchuk_buttons |= usb::NUNCHUK_BTN_C;
    }
    if state.pressed(profile.nunchuk_z) {
        nunchuk_buttons |= usb::NUNCHUK_BTN_Z;
    }

    let (nx, ny) = mapping::radial(profile.nunchuk_invert.apply(state.left), profile.deadzone);

    let ir_pointer = match source {
        PointerSource::Gyro => port.pointer,
        PointerSource::RightStick => motion::stick_pointer(mapping::radial(
            profile.pointer_invert.apply(state.right),
            profile.deadzone,
        )),
        PointerSource::Touchpad => port.touch.map(|(x, y)| gecko::input::aim_to_ir(x, y)),
        _ => None,
    };

    let mut accel = port.caps.accel.then(|| motion::map_accel(state.accel));

    if profile.sideways {
        accel = accel.map(|a| [-a[2], a[1], a[0]]);
    }

    HostInput::Wii {
        wiimote_buttons: buttons,
        wiimote_shake: profile.shake.is_some_and(|button| state.pressed(button)) && accel.is_none(),
        nunchuk_buttons,
        nunchuk_stick_x: mapping::stick_byte(nx),
        nunchuk_stick_y: mapping::stick_byte(ny),
        ir_pointer,
        accel,
    }
}

fn rotate_dpad(buttons: u16) -> u16 {
    const DPAD: u16 = usb::BTN_UP | usb::BTN_DOWN | usb::BTN_LEFT | usb::BTN_RIGHT;

    let mut out = buttons & !DPAD;

    if buttons & usb::BTN_UP != 0 {
        out |= usb::BTN_RIGHT;
    }
    if buttons & usb::BTN_DOWN != 0 {
        out |= usb::BTN_LEFT;
    }
    if buttons & usb::BTN_LEFT != 0 {
        out |= usb::BTN_UP;
    }
    if buttons & usb::BTN_RIGHT != 0 {
        out |= usb::BTN_DOWN;
    }

    out
}
