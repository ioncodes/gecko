use iced::keyboard::key::Code;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyboardConfig {
    pub gc: GcKeysConfig,
    pub wii: WiiKeysConfig,
    pub hotkeys: HotkeysConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GcKeysConfig {
    pub stick_up: Option<String>,
    pub stick_down: Option<String>,
    pub stick_left: Option<String>,
    pub stick_right: Option<String>,
    pub a: Option<String>,
    pub b: Option<String>,
    pub x: Option<String>,
    pub y: Option<String>,
    pub start: Option<String>,
    pub l: Option<String>,
    pub r: Option<String>,
    pub z: Option<String>,
    pub dpad_up: Option<String>,
    pub dpad_down: Option<String>,
    pub dpad_left: Option<String>,
    pub dpad_right: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WiiKeysConfig {
    pub one: Option<String>,
    pub two: Option<String>,
    pub plus: Option<String>,
    pub minus: Option<String>,
    pub home: Option<String>,
    pub dpad_up: Option<String>,
    pub dpad_down: Option<String>,
    pub dpad_left: Option<String>,
    pub dpad_right: Option<String>,
    pub shake: Option<String>,
    pub nunchuk_up: Option<String>,
    pub nunchuk_down: Option<String>,
    pub nunchuk_left: Option<String>,
    pub nunchuk_right: Option<String>,
    pub nunchuk_z: Option<String>,
    pub nunchuk_c: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeysConfig {
    pub pause: Option<String>,
    pub uncapped: Option<String>,
    pub fullscreen: Option<String>,
    pub overlay: Option<String>,
    pub screenshot: Option<String>,
    pub save_state: Option<String>,
    pub load_state: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct GcKeys {
    pub stick_up: Code,
    pub stick_down: Code,
    pub stick_left: Code,
    pub stick_right: Code,
    pub a: Code,
    pub b: Code,
    pub x: Code,
    pub y: Code,
    pub start: Code,
    pub l: Code,
    pub r: Code,
    pub z: Code,
    pub dpad_up: Code,
    pub dpad_down: Code,
    pub dpad_left: Code,
    pub dpad_right: Code,
}

impl Default for GcKeys {
    fn default() -> Self {
        Self {
            stick_up: Code::ArrowUp,
            stick_down: Code::ArrowDown,
            stick_left: Code::ArrowLeft,
            stick_right: Code::ArrowRight,
            a: Code::KeyX,
            b: Code::KeyZ,
            x: Code::KeyC,
            y: Code::KeyV,
            start: Code::Enter,
            l: Code::KeyA,
            r: Code::KeyS,
            z: Code::KeyD,
            dpad_up: Code::KeyI,
            dpad_down: Code::KeyK,
            dpad_left: Code::KeyJ,
            dpad_right: Code::KeyL,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WiiKeys {
    pub one: Code,
    pub two: Code,
    pub plus: Code,
    pub minus: Code,
    pub home: Code,
    pub dpad_up: Code,
    pub dpad_down: Code,
    pub dpad_left: Code,
    pub dpad_right: Code,
    pub shake: Code,
    pub nunchuk_up: Code,
    pub nunchuk_down: Code,
    pub nunchuk_left: Code,
    pub nunchuk_right: Code,
    pub nunchuk_z: Code,
    pub nunchuk_c: Code,
}

impl Default for WiiKeys {
    fn default() -> Self {
        Self {
            one: Code::Digit1,
            two: Code::Digit2,
            plus: Code::Equal,
            minus: Code::Minus,
            home: Code::Home,
            dpad_up: Code::ArrowUp,
            dpad_down: Code::ArrowDown,
            dpad_left: Code::ArrowLeft,
            dpad_right: Code::ArrowRight,
            shake: Code::ShiftLeft,
            nunchuk_up: Code::KeyW,
            nunchuk_down: Code::KeyS,
            nunchuk_left: Code::KeyA,
            nunchuk_right: Code::KeyD,
            nunchuk_z: Code::KeyQ,
            nunchuk_c: Code::KeyE,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Hotkeys {
    pub pause: Code,
    pub uncapped: Code,
    pub fullscreen: Code,
    pub overlay: Code,
    pub screenshot: Code,
    pub save_state: Code,
    pub load_state: Code,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            pause: Code::Space,
            uncapped: Code::Tab,
            fullscreen: Code::F10,
            overlay: Code::F11,
            screenshot: Code::F12,
            save_state: Code::F5,
            load_state: Code::F7,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Keymap {
    pub gc: GcKeys,
    pub wii: WiiKeys,
    pub hotkeys: Hotkeys,
}

#[derive(Debug, Clone, Copy)]
pub enum Hotkey {
    Pause,
    Uncapped,
    Fullscreen,
    Overlay,
    Screenshot,
    SaveState,
    LoadState,
}

impl Hotkeys {
    pub fn lookup(&self, key: Code) -> Option<Hotkey> {
        if key == self.pause {
            Some(Hotkey::Pause)
        } else if key == self.uncapped {
            Some(Hotkey::Uncapped)
        } else if key == self.fullscreen {
            Some(Hotkey::Fullscreen)
        } else if key == self.overlay {
            Some(Hotkey::Overlay)
        } else if key == self.screenshot {
            Some(Hotkey::Screenshot)
        } else if key == self.save_state {
            Some(Hotkey::SaveState)
        } else if key == self.load_state {
            Some(Hotkey::LoadState)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTarget {
    GcStickUp,
    GcStickDown,
    GcStickLeft,
    GcStickRight,
    GcA,
    GcB,
    GcX,
    GcY,
    GcStart,
    GcL,
    GcR,
    GcZ,
    GcDpadUp,
    GcDpadDown,
    GcDpadLeft,
    GcDpadRight,
    WiiOne,
    WiiTwo,
    WiiPlus,
    WiiMinus,
    WiiHome,
    WiiDpadUp,
    WiiDpadDown,
    WiiDpadLeft,
    WiiDpadRight,
    WiiShake,
    NunchukUp,
    NunchukDown,
    NunchukLeft,
    NunchukRight,
    NunchukZ,
    NunchukC,
    HotkeyPause,
    HotkeyUncapped,
    HotkeyFullscreen,
    HotkeyOverlay,
    HotkeyScreenshot,
    HotkeySaveState,
    HotkeyLoadState,
}

pub const GC_KEY_TARGETS: &[(KeyTarget, &str)] = &[
    (KeyTarget::GcStickUp, "Stick Up"),
    (KeyTarget::GcStickDown, "Stick Down"),
    (KeyTarget::GcStickLeft, "Stick Left"),
    (KeyTarget::GcStickRight, "Stick Right"),
    (KeyTarget::GcA, "A"),
    (KeyTarget::GcB, "B"),
    (KeyTarget::GcX, "X"),
    (KeyTarget::GcY, "Y"),
    (KeyTarget::GcStart, "Start"),
    (KeyTarget::GcL, "L"),
    (KeyTarget::GcR, "R"),
    (KeyTarget::GcZ, "Z"),
    (KeyTarget::GcDpadUp, "D-Pad Up"),
    (KeyTarget::GcDpadDown, "D-Pad Down"),
    (KeyTarget::GcDpadLeft, "D-Pad Left"),
    (KeyTarget::GcDpadRight, "D-Pad Right"),
];

pub const WII_KEY_TARGETS: &[(KeyTarget, &str)] = &[
    (KeyTarget::WiiDpadUp, "D-Pad Up"),
    (KeyTarget::WiiDpadDown, "D-Pad Down"),
    (KeyTarget::WiiDpadLeft, "D-Pad Left"),
    (KeyTarget::WiiDpadRight, "D-Pad Right"),
    (KeyTarget::WiiOne, "1"),
    (KeyTarget::WiiTwo, "2"),
    (KeyTarget::WiiPlus, "Plus"),
    (KeyTarget::WiiMinus, "Minus"),
    (KeyTarget::WiiHome, "Home"),
    (KeyTarget::WiiShake, "Shake"),
    (KeyTarget::NunchukUp, "Nunchuk Up"),
    (KeyTarget::NunchukDown, "Nunchuk Down"),
    (KeyTarget::NunchukLeft, "Nunchuk Left"),
    (KeyTarget::NunchukRight, "Nunchuk Right"),
    (KeyTarget::NunchukZ, "Nunchuk Z"),
    (KeyTarget::NunchukC, "Nunchuk C"),
];

pub const HOTKEY_TARGETS: &[(KeyTarget, &str)] = &[
    (KeyTarget::HotkeyPause, "Pause"),
    (KeyTarget::HotkeyUncapped, "Uncapped"),
    (KeyTarget::HotkeyFullscreen, "Fullscreen"),
    (KeyTarget::HotkeyOverlay, "Overlay"),
    (KeyTarget::HotkeyScreenshot, "Screenshot"),
    (KeyTarget::HotkeySaveState, "Save State"),
    (KeyTarget::HotkeyLoadState, "Load State"),
];

pub fn field(config: &mut KeyboardConfig, target: KeyTarget) -> &mut Option<String> {
    match target {
        KeyTarget::GcStickUp => &mut config.gc.stick_up,
        KeyTarget::GcStickDown => &mut config.gc.stick_down,
        KeyTarget::GcStickLeft => &mut config.gc.stick_left,
        KeyTarget::GcStickRight => &mut config.gc.stick_right,
        KeyTarget::GcA => &mut config.gc.a,
        KeyTarget::GcB => &mut config.gc.b,
        KeyTarget::GcX => &mut config.gc.x,
        KeyTarget::GcY => &mut config.gc.y,
        KeyTarget::GcStart => &mut config.gc.start,
        KeyTarget::GcL => &mut config.gc.l,
        KeyTarget::GcR => &mut config.gc.r,
        KeyTarget::GcZ => &mut config.gc.z,
        KeyTarget::GcDpadUp => &mut config.gc.dpad_up,
        KeyTarget::GcDpadDown => &mut config.gc.dpad_down,
        KeyTarget::GcDpadLeft => &mut config.gc.dpad_left,
        KeyTarget::GcDpadRight => &mut config.gc.dpad_right,
        KeyTarget::WiiOne => &mut config.wii.one,
        KeyTarget::WiiTwo => &mut config.wii.two,
        KeyTarget::WiiPlus => &mut config.wii.plus,
        KeyTarget::WiiMinus => &mut config.wii.minus,
        KeyTarget::WiiHome => &mut config.wii.home,
        KeyTarget::WiiDpadUp => &mut config.wii.dpad_up,
        KeyTarget::WiiDpadDown => &mut config.wii.dpad_down,
        KeyTarget::WiiDpadLeft => &mut config.wii.dpad_left,
        KeyTarget::WiiDpadRight => &mut config.wii.dpad_right,
        KeyTarget::WiiShake => &mut config.wii.shake,
        KeyTarget::NunchukUp => &mut config.wii.nunchuk_up,
        KeyTarget::NunchukDown => &mut config.wii.nunchuk_down,
        KeyTarget::NunchukLeft => &mut config.wii.nunchuk_left,
        KeyTarget::NunchukRight => &mut config.wii.nunchuk_right,
        KeyTarget::NunchukZ => &mut config.wii.nunchuk_z,
        KeyTarget::NunchukC => &mut config.wii.nunchuk_c,
        KeyTarget::HotkeyPause => &mut config.hotkeys.pause,
        KeyTarget::HotkeyUncapped => &mut config.hotkeys.uncapped,
        KeyTarget::HotkeyFullscreen => &mut config.hotkeys.fullscreen,
        KeyTarget::HotkeyOverlay => &mut config.hotkeys.overlay,
        KeyTarget::HotkeyScreenshot => &mut config.hotkeys.screenshot,
        KeyTarget::HotkeySaveState => &mut config.hotkeys.save_state,
        KeyTarget::HotkeyLoadState => &mut config.hotkeys.load_state,
    }
}

impl Keymap {
    pub fn code(&self, target: KeyTarget) -> Code {
        match target {
            KeyTarget::GcStickUp => self.gc.stick_up,
            KeyTarget::GcStickDown => self.gc.stick_down,
            KeyTarget::GcStickLeft => self.gc.stick_left,
            KeyTarget::GcStickRight => self.gc.stick_right,
            KeyTarget::GcA => self.gc.a,
            KeyTarget::GcB => self.gc.b,
            KeyTarget::GcX => self.gc.x,
            KeyTarget::GcY => self.gc.y,
            KeyTarget::GcStart => self.gc.start,
            KeyTarget::GcL => self.gc.l,
            KeyTarget::GcR => self.gc.r,
            KeyTarget::GcZ => self.gc.z,
            KeyTarget::GcDpadUp => self.gc.dpad_up,
            KeyTarget::GcDpadDown => self.gc.dpad_down,
            KeyTarget::GcDpadLeft => self.gc.dpad_left,
            KeyTarget::GcDpadRight => self.gc.dpad_right,
            KeyTarget::WiiOne => self.wii.one,
            KeyTarget::WiiTwo => self.wii.two,
            KeyTarget::WiiPlus => self.wii.plus,
            KeyTarget::WiiMinus => self.wii.minus,
            KeyTarget::WiiHome => self.wii.home,
            KeyTarget::WiiDpadUp => self.wii.dpad_up,
            KeyTarget::WiiDpadDown => self.wii.dpad_down,
            KeyTarget::WiiDpadLeft => self.wii.dpad_left,
            KeyTarget::WiiDpadRight => self.wii.dpad_right,
            KeyTarget::WiiShake => self.wii.shake,
            KeyTarget::NunchukUp => self.wii.nunchuk_up,
            KeyTarget::NunchukDown => self.wii.nunchuk_down,
            KeyTarget::NunchukLeft => self.wii.nunchuk_left,
            KeyTarget::NunchukRight => self.wii.nunchuk_right,
            KeyTarget::NunchukZ => self.wii.nunchuk_z,
            KeyTarget::NunchukC => self.wii.nunchuk_c,
            KeyTarget::HotkeyPause => self.hotkeys.pause,
            KeyTarget::HotkeyUncapped => self.hotkeys.uncapped,
            KeyTarget::HotkeyFullscreen => self.hotkeys.fullscreen,
            KeyTarget::HotkeyOverlay => self.hotkeys.overlay,
            KeyTarget::HotkeyScreenshot => self.hotkeys.screenshot,
            KeyTarget::HotkeySaveState => self.hotkeys.save_state,
            KeyTarget::HotkeyLoadState => self.hotkeys.load_state,
        }
    }
}

impl KeyboardConfig {
    pub fn resolve(&self) -> Keymap {
        let mut keymap = Keymap::default();

        self::set(&mut keymap.gc.stick_up, &self.gc.stick_up);
        self::set(&mut keymap.gc.stick_down, &self.gc.stick_down);
        self::set(&mut keymap.gc.stick_left, &self.gc.stick_left);
        self::set(&mut keymap.gc.stick_right, &self.gc.stick_right);
        self::set(&mut keymap.gc.a, &self.gc.a);
        self::set(&mut keymap.gc.b, &self.gc.b);
        self::set(&mut keymap.gc.x, &self.gc.x);
        self::set(&mut keymap.gc.y, &self.gc.y);
        self::set(&mut keymap.gc.start, &self.gc.start);
        self::set(&mut keymap.gc.l, &self.gc.l);
        self::set(&mut keymap.gc.r, &self.gc.r);
        self::set(&mut keymap.gc.z, &self.gc.z);
        self::set(&mut keymap.gc.dpad_up, &self.gc.dpad_up);
        self::set(&mut keymap.gc.dpad_down, &self.gc.dpad_down);
        self::set(&mut keymap.gc.dpad_left, &self.gc.dpad_left);
        self::set(&mut keymap.gc.dpad_right, &self.gc.dpad_right);

        self::set(&mut keymap.wii.one, &self.wii.one);
        self::set(&mut keymap.wii.two, &self.wii.two);
        self::set(&mut keymap.wii.plus, &self.wii.plus);
        self::set(&mut keymap.wii.minus, &self.wii.minus);
        self::set(&mut keymap.wii.home, &self.wii.home);
        self::set(&mut keymap.wii.dpad_up, &self.wii.dpad_up);
        self::set(&mut keymap.wii.dpad_down, &self.wii.dpad_down);
        self::set(&mut keymap.wii.dpad_left, &self.wii.dpad_left);
        self::set(&mut keymap.wii.dpad_right, &self.wii.dpad_right);
        self::set(&mut keymap.wii.shake, &self.wii.shake);
        self::set(&mut keymap.wii.nunchuk_up, &self.wii.nunchuk_up);
        self::set(&mut keymap.wii.nunchuk_down, &self.wii.nunchuk_down);
        self::set(&mut keymap.wii.nunchuk_left, &self.wii.nunchuk_left);
        self::set(&mut keymap.wii.nunchuk_right, &self.wii.nunchuk_right);
        self::set(&mut keymap.wii.nunchuk_z, &self.wii.nunchuk_z);
        self::set(&mut keymap.wii.nunchuk_c, &self.wii.nunchuk_c);

        self::set(&mut keymap.hotkeys.pause, &self.hotkeys.pause);
        self::set(&mut keymap.hotkeys.uncapped, &self.hotkeys.uncapped);
        self::set(&mut keymap.hotkeys.fullscreen, &self.hotkeys.fullscreen);
        self::set(&mut keymap.hotkeys.overlay, &self.hotkeys.overlay);
        self::set(&mut keymap.hotkeys.screenshot, &self.hotkeys.screenshot);
        self::set(&mut keymap.hotkeys.save_state, &self.hotkeys.save_state);
        self::set(&mut keymap.hotkeys.load_state, &self.hotkeys.load_state);

        keymap
    }
}

fn set(slot: &mut Code, name: &Option<String>) {
    let Some(name) = name.as_deref() else {
        return;
    };

    match self::parse_key(name) {
        Some(code) => *slot = code,
        None => tracing::warn!(key = name, "unknown key name in config, keeping default"),
    }
}

pub fn key_label(code: Code) -> String {
    match self::key_name(code) {
        Some(name) => self::title_case(name),
        None => "Unsupported".to_owned(),
    }
}

fn title_case(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

macro_rules! key_names {
    ($($variant:ident => $name:literal),* $(,)?) => {
        pub fn key_name(code: Code) -> Option<&'static str> {
            match code {
                $( Code::$variant => Some($name), )*
                _ => None,
            }
        }

        pub fn parse_key(name: &str) -> Option<Code> {
            match name {
                $( $name => Some(Code::$variant), )*
                _ => None,
            }
        }
    };
}

key_names! {
    KeyA => "a", KeyB => "b", KeyC => "c", KeyD => "d", KeyE => "e", KeyF => "f",
    KeyG => "g", KeyH => "h", KeyI => "i", KeyJ => "j", KeyK => "k", KeyL => "l",
    KeyM => "m", KeyN => "n", KeyO => "o", KeyP => "p", KeyQ => "q", KeyR => "r",
    KeyS => "s", KeyT => "t", KeyU => "u", KeyV => "v", KeyW => "w", KeyX => "x",
    KeyY => "y", KeyZ => "z",
    Digit0 => "0", Digit1 => "1", Digit2 => "2", Digit3 => "3", Digit4 => "4",
    Digit5 => "5", Digit6 => "6", Digit7 => "7", Digit8 => "8", Digit9 => "9",
    ArrowUp => "up", ArrowDown => "down", ArrowLeft => "left", ArrowRight => "right",
    Space => "space", Enter => "enter", Tab => "tab", Escape => "escape",
    Backspace => "backspace", Delete => "delete", Insert => "insert",
    Home => "home", End => "end", PageUp => "page_up", PageDown => "page_down",
    CapsLock => "caps_lock",
    ShiftLeft => "shift_left", ShiftRight => "shift_right",
    ControlLeft => "ctrl_left", ControlRight => "ctrl_right",
    AltLeft => "alt_left", AltRight => "alt_right",
    SuperLeft => "super_left", SuperRight => "super_right",
    Minus => "minus", Equal => "equal", Comma => "comma", Period => "period",
    Slash => "slash", Semicolon => "semicolon", Quote => "quote",
    Backquote => "backquote", Backslash => "backslash",
    BracketLeft => "bracket_left", BracketRight => "bracket_right",
    F1 => "f1", F2 => "f2", F3 => "f3", F4 => "f4", F5 => "f5", F6 => "f6",
    F7 => "f7", F8 => "f8", F9 => "f9", F10 => "f10", F11 => "f11", F12 => "f12",
    Numpad0 => "numpad_0", Numpad1 => "numpad_1", Numpad2 => "numpad_2",
    Numpad3 => "numpad_3", Numpad4 => "numpad_4", Numpad5 => "numpad_5",
    Numpad6 => "numpad_6", Numpad7 => "numpad_7", Numpad8 => "numpad_8",
    Numpad9 => "numpad_9", NumpadEnter => "numpad_enter", NumpadAdd => "numpad_add",
    NumpadSubtract => "numpad_subtract", NumpadMultiply => "numpad_multiply",
    NumpadDivide => "numpad_divide", NumpadDecimal => "numpad_decimal",
}
