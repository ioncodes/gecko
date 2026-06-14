#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    South,
    East,
    West,
    North,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Start,
    Select,
    Guide,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    TouchpadClick,
}

impl Button {
    pub const ALL: [Button; 18] = [
        Button::South,
        Button::East,
        Button::West,
        Button::North,
        Button::L1,
        Button::R1,
        Button::L2,
        Button::R2,
        Button::L3,
        Button::R3,
        Button::Start,
        Button::Select,
        Button::Guide,
        Button::DpadUp,
        Button::DpadDown,
        Button::DpadLeft,
        Button::DpadRight,
        Button::TouchpadClick,
    ];

    pub const fn mask(self) -> u32 {
        1 << self as u32
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct DeviceState {
    pub buttons: u32,
    pub left: (f32, f32),
    pub right: (f32, f32),
    pub l2: f32,
    pub r2: f32,
    pub accel: [f32; 3],
}

impl DeviceState {
    pub fn pressed(&self, button: Button) -> bool {
        self.buttons & button.mask() != 0
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Capabilities {
    pub analog_sticks: bool,
    pub gyro: bool,
    pub accel: bool,
    pub rumble: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PortInput {
    pub state: DeviceState,
    pub caps: Capabilities,
    pub pointer: Option<(u16, u16)>,
    pub touch: Option<(f32, f32)>,
}
