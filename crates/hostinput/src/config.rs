use crate::device::Button;
use crate::mapping::{GcProfile, PointerSource, WiiProfile};
use gecko::flipper::si::pad;
use gecko::hollywood::ipc::usb;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    pub gamepads: bool,
    pub gc: GcConfig,
    pub wii: WiiConfig,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            gamepads: true,
            gc: GcConfig::default(),
            wii: WiiConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GcConfig {
    pub a: Option<String>,
    pub b: Option<String>,
    pub x: Option<String>,
    pub y: Option<String>,
    pub start: Option<String>,
    pub z: Option<String>,
    pub invert_stick_x: Option<bool>,
    pub invert_stick_y: Option<bool>,
    pub invert_cstick_x: Option<bool>,
    pub invert_cstick_y: Option<bool>,
    pub deadzone: Option<f32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WiiConfig {
    pub a: Option<String>,
    pub b: Option<String>,
    pub one: Option<String>,
    pub two: Option<String>,
    pub plus: Option<String>,
    pub minus: Option<String>,
    pub home: Option<String>,
    pub nunchuk_c: Option<String>,
    pub nunchuk_z: Option<String>,
    pub shake: Option<String>,
    pub recenter: Option<String>,
    pub pointer: Option<String>,
    pub pointer_sensitivity: Option<f32>,
    pub sideways: Option<bool>,
    pub stick_dpad: Option<bool>,
    pub invert_nunchuk_x: Option<bool>,
    pub invert_nunchuk_y: Option<bool>,
    pub invert_stick_dpad_x: Option<bool>,
    pub invert_stick_dpad_y: Option<bool>,
    pub invert_pointer_x: Option<bool>,
    pub invert_pointer_y: Option<bool>,
    pub deadzone: Option<f32>,
}

pub fn button_name(button: Button) -> &'static str {
    match button {
        Button::South => "south",
        Button::East => "east",
        Button::West => "west",
        Button::North => "north",
        Button::L1 => "l1",
        Button::R1 => "r1",
        Button::L2 => "l2",
        Button::R2 => "r2",
        Button::L3 => "l3",
        Button::R3 => "r3",
        Button::Start => "start",
        Button::Select => "select",
        Button::Guide => "guide",
        Button::DpadUp => "dpad_up",
        Button::DpadDown => "dpad_down",
        Button::DpadLeft => "dpad_left",
        Button::DpadRight => "dpad_right",
        Button::TouchpadClick => "touchpad",
    }
}

pub fn parse_button(name: &str) -> Option<Button> {
    Some(match name {
        "south" => Button::South,
        "east" => Button::East,
        "west" => Button::West,
        "north" => Button::North,
        "l1" => Button::L1,
        "r1" => Button::R1,
        "l2" => Button::L2,
        "r2" => Button::R2,
        "l3" => Button::L3,
        "r3" => Button::R3,
        "start" => Button::Start,
        "select" => Button::Select,
        "guide" => Button::Guide,
        "dpad_up" => Button::DpadUp,
        "dpad_down" => Button::DpadDown,
        "dpad_left" => Button::DpadLeft,
        "dpad_right" => Button::DpadRight,
        "touchpad" => Button::TouchpadClick,
        _ => return None,
    })
}

fn warn_parse(name: &str) -> Option<Button> {
    let button = self::parse_button(name);

    if button.is_none() {
        tracing::warn!(name, "unknown input button name, keeping default");
    }

    button
}

fn rebind(buttons: &mut Vec<(Button, u16)>, mask: u16, name: &Option<String>) {
    let Some(button) = name.as_deref().and_then(self::warn_parse) else {
        return;
    };

    buttons.retain(|(_, m)| *m != mask);
    buttons.push((button, mask));
}

fn button_or(name: &Option<String>, default: Button) -> Button {
    name.as_deref().and_then(self::warn_parse).unwrap_or(default)
}

impl InputConfig {
    pub fn gc_profile(&self) -> GcProfile {
        let mut profile = GcProfile::default();

        self::rebind(&mut profile.buttons, pad::A, &self.gc.a);
        self::rebind(&mut profile.buttons, pad::B, &self.gc.b);
        self::rebind(&mut profile.buttons, pad::X, &self.gc.x);
        self::rebind(&mut profile.buttons, pad::Y, &self.gc.y);
        self::rebind(&mut profile.buttons, pad::START, &self.gc.start);
        self::rebind(&mut profile.buttons, pad::Z, &self.gc.z);

        profile.stick_invert.x = self.gc.invert_stick_x.unwrap_or(profile.stick_invert.x);
        profile.stick_invert.y = self.gc.invert_stick_y.unwrap_or(profile.stick_invert.y);
        profile.substick_invert.x = self.gc.invert_cstick_x.unwrap_or(profile.substick_invert.x);
        profile.substick_invert.y = self.gc.invert_cstick_y.unwrap_or(profile.substick_invert.y);

        if let Some(deadzone) = self.gc.deadzone {
            profile.deadzone = deadzone.clamp(0.0, 0.9);
        }

        profile
    }

    pub fn wii_profile(&self) -> WiiProfile {
        let mut profile = WiiProfile::default();

        self::rebind(&mut profile.buttons, usb::BTN_A, &self.wii.a);
        self::rebind(&mut profile.buttons, usb::BTN_B, &self.wii.b);
        self::rebind(&mut profile.buttons, usb::BTN_ONE, &self.wii.one);
        self::rebind(&mut profile.buttons, usb::BTN_TWO, &self.wii.two);
        self::rebind(&mut profile.buttons, usb::BTN_PLUS, &self.wii.plus);
        self::rebind(&mut profile.buttons, usb::BTN_MINUS, &self.wii.minus);
        self::rebind(&mut profile.buttons, usb::BTN_HOME, &self.wii.home);

        profile.nunchuk_c = self::button_or(&self.wii.nunchuk_c, profile.nunchuk_c);
        profile.nunchuk_z = self::button_or(&self.wii.nunchuk_z, profile.nunchuk_z);
        profile.recenter = self::button_or(&self.wii.recenter, profile.recenter);

        profile.shake = match self.wii.shake.as_deref() {
            Some("none") => None,
            Some(name) => self::warn_parse(name).or(profile.shake),
            None => profile.shake,
        };

        profile.pointer = match self.wii.pointer.as_deref() {
            Some("auto") | None => PointerSource::Auto,
            Some("gyro") => PointerSource::Gyro,
            Some("stick") => PointerSource::RightStick,
            Some("touchpad") => PointerSource::Touchpad,
            Some("mouse") => PointerSource::Mouse,
            Some(other) => {
                tracing::warn!(other, "unknown pointer source, using auto");
                PointerSource::Auto
            }
        };

        if let Some(sensitivity) = self.wii.pointer_sensitivity {
            profile.sensitivity = sensitivity.clamp(0.1, 10.0);
        }
        if let Some(sideways) = self.wii.sideways {
            profile.sideways = sideways;
        }
        if let Some(stick_dpad) = self.wii.stick_dpad {
            profile.stick_dpad = stick_dpad;
        }

        profile.nunchuk_invert.x = self.wii.invert_nunchuk_x.unwrap_or(profile.nunchuk_invert.x);
        profile.nunchuk_invert.y = self.wii.invert_nunchuk_y.unwrap_or(profile.nunchuk_invert.y);
        profile.stick_dpad_invert.x = self.wii.invert_stick_dpad_x.unwrap_or(profile.stick_dpad_invert.x);
        profile.stick_dpad_invert.y = self.wii.invert_stick_dpad_y.unwrap_or(profile.stick_dpad_invert.y);
        profile.pointer_invert.x = self.wii.invert_pointer_x.unwrap_or(profile.pointer_invert.x);
        profile.pointer_invert.y = self.wii.invert_pointer_y.unwrap_or(profile.pointer_invert.y);
        if let Some(deadzone) = self.wii.deadzone {
            profile.deadzone = deadzone.clamp(0.0, 0.9);
        }

        profile
    }
}
