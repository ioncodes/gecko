use gecko::HostInput;
use gecko::flipper::si::pad::{PadStatus, STICK_CENTER};

pub fn gc(pad: PadStatus, kb: PadStatus) -> PadStatus {
    PadStatus {
        buttons: pad.buttons | kb.buttons,
        stick_x: self::stick(pad.stick_x, kb.stick_x),
        stick_y: self::stick(pad.stick_y, kb.stick_y),
        substick_x: self::stick(pad.substick_x, kb.substick_x),
        substick_y: self::stick(pad.substick_y, kb.substick_y),
        trigger_left: pad.trigger_left.max(kb.trigger_left),
        trigger_right: pad.trigger_right.max(kb.trigger_right),
        connected: true,
    }
}

fn stick(pad: u8, kb: u8) -> u8 {
    if pad != STICK_CENTER { pad } else { kb }
}

pub fn wii(pad: HostInput, kb: HostInput) -> HostInput {
    let HostInput::Wii {
        wiimote_buttons,
        wiimote_shake,
        nunchuk_buttons,
        nunchuk_stick_x,
        nunchuk_stick_y,
        ir_pointer,
        accel,
    } = pad
    else {
        return kb;
    };

    let HostInput::Wii {
        wiimote_buttons: kb_buttons,
        wiimote_shake: kb_shake,
        nunchuk_buttons: kb_nunchuk,
        nunchuk_stick_x: kb_stick_x,
        nunchuk_stick_y: kb_stick_y,
        ir_pointer: kb_pointer,
        accel: kb_accel,
    } = kb
    else {
        return kb;
    };

    HostInput::Wii {
        wiimote_buttons: wiimote_buttons | kb_buttons,
        wiimote_shake: wiimote_shake || kb_shake,
        nunchuk_buttons: nunchuk_buttons | kb_nunchuk,
        nunchuk_stick_x: self::stick(nunchuk_stick_x, kb_stick_x),
        nunchuk_stick_y: self::stick(nunchuk_stick_y, kb_stick_y),
        ir_pointer: ir_pointer.or(kb_pointer),
        accel: accel.or(kb_accel),
    }
}
