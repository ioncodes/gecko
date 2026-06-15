use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER, STICK_MAX, STICK_MIN, TRIGGER_MAX, TRIGGER_MIN};
use gecko::hollywood::ipc::usb as wiimote;
use iced::keyboard::key::Code;

use crate::keybinds::{GcKeys, WiiKeys};

#[inline(always)]
fn set_bit<T: std::ops::BitOrAssign + std::ops::BitAndAssign + std::ops::Not<Output = T> + Copy>(
    bits: &mut T,
    mask: T,
    on: bool,
) {
    if on {
        *bits |= mask;
    } else {
        *bits &= !mask;
    }
}

pub fn update_pad(pad: &mut PadStatus, keys: &GcKeys, key: Code, pressed: bool) {
    if key == keys.stick_up {
        pad.stick_y = if pressed { STICK_MAX } else { STICK_CENTER };
    } else if key == keys.stick_down {
        pad.stick_y = if pressed { STICK_MIN } else { STICK_CENTER };
    } else if key == keys.stick_left {
        pad.stick_x = if pressed { STICK_MIN } else { STICK_CENTER };
    } else if key == keys.stick_right {
        pad.stick_x = if pressed { STICK_MAX } else { STICK_CENTER };
    } else if key == keys.a {
        self::set_bit(&mut pad.buttons, pad::A, pressed);
    } else if key == keys.b {
        self::set_bit(&mut pad.buttons, pad::B, pressed);
    } else if key == keys.x {
        self::set_bit(&mut pad.buttons, pad::X, pressed);
    } else if key == keys.y {
        self::set_bit(&mut pad.buttons, pad::Y, pressed);
    } else if key == keys.start {
        self::set_bit(&mut pad.buttons, pad::START, pressed);
    } else if key == keys.l {
        self::set_bit(&mut pad.buttons, pad::L, pressed);
        pad.trigger_left = if pressed { TRIGGER_MAX } else { TRIGGER_MIN };
    } else if key == keys.r {
        self::set_bit(&mut pad.buttons, pad::R, pressed);
        pad.trigger_right = if pressed { TRIGGER_MAX } else { TRIGGER_MIN };
    } else if key == keys.z {
        self::set_bit(&mut pad.buttons, pad::Z, pressed);
    } else if key == keys.dpad_up {
        self::set_bit(&mut pad.buttons, pad::DPAD_UP, pressed);
    } else if key == keys.dpad_down {
        self::set_bit(&mut pad.buttons, pad::DPAD_DOWN, pressed);
    } else if key == keys.dpad_left {
        self::set_bit(&mut pad.buttons, pad::DPAD_LEFT, pressed);
    } else if key == keys.dpad_right {
        self::set_bit(&mut pad.buttons, pad::DPAD_RIGHT, pressed);
    }
}

pub fn update_wiimote_keys(buttons: &mut u16, keys: &WiiKeys, key: Code, pressed: bool) {
    let mask = if key == keys.one {
        wiimote::BTN_ONE
    } else if key == keys.two {
        wiimote::BTN_TWO
    } else if key == keys.home {
        wiimote::BTN_HOME
    } else if key == keys.minus {
        wiimote::BTN_MINUS
    } else if key == keys.plus {
        wiimote::BTN_PLUS
    } else if key == keys.dpad_up {
        wiimote::BTN_UP
    } else if key == keys.dpad_down {
        wiimote::BTN_DOWN
    } else if key == keys.dpad_left {
        wiimote::BTN_LEFT
    } else if key == keys.dpad_right {
        wiimote::BTN_RIGHT
    } else {
        return;
    };

    self::set_bit(buttons, mask, pressed);
}

pub fn update_wiimote_motion_keys(shake: &mut bool, keys: &WiiKeys, key: Code, pressed: bool) {
    if key == keys.shake {
        *shake = pressed;
    }
}

pub fn update_nunchuk_keys(
    buttons: &mut u8,
    stick_x: &mut u8,
    stick_y: &mut u8,
    keys: &WiiKeys,
    key: Code,
    pressed: bool,
) {
    use wiimote::{NUNCHUK_STICK_CENTER as C, NUNCHUK_STICK_MAX as MAX, NUNCHUK_STICK_MIN as MIN};

    if key == keys.nunchuk_up {
        *stick_y = if pressed { MAX } else { C };
    } else if key == keys.nunchuk_down {
        *stick_y = if pressed { MIN } else { C };
    } else if key == keys.nunchuk_left {
        *stick_x = if pressed { MIN } else { C };
    } else if key == keys.nunchuk_right {
        *stick_x = if pressed { MAX } else { C };
    } else if key == keys.nunchuk_z {
        self::set_bit(buttons, wiimote::NUNCHUK_BTN_Z, pressed);
    } else if key == keys.nunchuk_c {
        self::set_bit(buttons, wiimote::NUNCHUK_BTN_C, pressed);
    }
}
