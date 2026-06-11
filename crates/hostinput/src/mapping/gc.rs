use crate::device::DeviceState;
use crate::mapping::{self, GcProfile};
use gecko::flipper::si::pad::{self, PadStatus};

pub const TRIGGER_THRESHOLD: f32 = 0.85;

pub fn map(state: &DeviceState, profile: &GcProfile) -> PadStatus {
    let mut buttons = 0u16;
    for (button, mask) in &profile.buttons {
        if state.pressed(*button) {
            buttons |= mask;
        }
    }

    if state.l2 >= TRIGGER_THRESHOLD {
        buttons |= pad::L;
    }
    if state.r2 >= TRIGGER_THRESHOLD {
        buttons |= pad::R;
    }

    let (lx, ly) = mapping::radial(profile.stick_invert.apply(state.left), profile.deadzone);
    let (rx, ry) = mapping::radial(profile.substick_invert.apply(state.right), profile.deadzone);

    PadStatus {
        buttons,
        stick_x: mapping::stick_byte(lx),
        stick_y: mapping::stick_byte(ly),
        substick_x: mapping::stick_byte(rx),
        substick_y: mapping::stick_byte(ry),
        trigger_left: mapping::trigger_byte(state.l2),
        trigger_right: mapping::trigger_byte(state.r2),
        connected: true,
    }
}
