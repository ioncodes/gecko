use gecko::flipper::si::pad;
use gecko::hollywood::ipc::usb;
use hostinput::mapping::{GcProfile, WiiProfile};
use hostinput::{Button, InputConfig};
use iced::widget::{button, column, container, pick_list, row, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding};

use crate::app::Message;
use crate::keybinds::{self, KeyTarget, KeyboardConfig, Keymap};
use crate::theme::Palette;
use crate::widgets::overlay;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTab {
    Gc,
    Wii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardTab {
    Gc,
    Wii,
    Hotkeys,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindTarget {
    GcA,
    GcB,
    GcX,
    GcY,
    GcStart,
    GcZ,
    WiiA,
    WiiB,
    WiiOne,
    WiiTwo,
    WiiPlus,
    WiiMinus,
    WiiHome,
    NunchukC,
    NunchukZ,
    Shake,
    Recenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvertTarget {
    GcStickX,
    GcStickY,
    GcCstickX,
    GcCstickY,
    NunchukX,
    NunchukY,
    StickDpadX,
    StickDpadY,
    PointerX,
    PointerY,
}

const GC_TARGETS: [(BindTarget, &str); 6] = [
    (BindTarget::GcA, "A"),
    (BindTarget::GcB, "B"),
    (BindTarget::GcX, "X"),
    (BindTarget::GcY, "Y"),
    (BindTarget::GcStart, "Start"),
    (BindTarget::GcZ, "Z"),
];

const WII_TARGETS: [(BindTarget, &str); 11] = [
    (BindTarget::WiiA, "A"),
    (BindTarget::WiiB, "B"),
    (BindTarget::WiiOne, "1"),
    (BindTarget::WiiTwo, "2"),
    (BindTarget::WiiPlus, "Plus"),
    (BindTarget::WiiMinus, "Minus"),
    (BindTarget::WiiHome, "Home"),
    (BindTarget::NunchukC, "Nunchuk C"),
    (BindTarget::NunchukZ, "Nunchuk Z"),
    (BindTarget::Shake, "Shake"),
    (BindTarget::Recenter, "Recenter"),
];

const POINTER_SOURCES: [&str; 5] = ["auto", "gyro", "stick", "touchpad", "mouse"];

pub fn field(config: &mut InputConfig, target: BindTarget) -> &mut Option<String> {
    match target {
        BindTarget::GcA => &mut config.gc.a,
        BindTarget::GcB => &mut config.gc.b,
        BindTarget::GcX => &mut config.gc.x,
        BindTarget::GcY => &mut config.gc.y,
        BindTarget::GcStart => &mut config.gc.start,
        BindTarget::GcZ => &mut config.gc.z,
        BindTarget::WiiA => &mut config.wii.a,
        BindTarget::WiiB => &mut config.wii.b,
        BindTarget::WiiOne => &mut config.wii.one,
        BindTarget::WiiTwo => &mut config.wii.two,
        BindTarget::WiiPlus => &mut config.wii.plus,
        BindTarget::WiiMinus => &mut config.wii.minus,
        BindTarget::WiiHome => &mut config.wii.home,
        BindTarget::NunchukC => &mut config.wii.nunchuk_c,
        BindTarget::NunchukZ => &mut config.wii.nunchuk_z,
        BindTarget::Shake => &mut config.wii.shake,
        BindTarget::Recenter => &mut config.wii.recenter,
    }
}

pub fn invert_field(config: &mut InputConfig, target: InvertTarget) -> &mut Option<bool> {
    match target {
        InvertTarget::GcStickX => &mut config.gc.invert_stick_x,
        InvertTarget::GcStickY => &mut config.gc.invert_stick_y,
        InvertTarget::GcCstickX => &mut config.gc.invert_cstick_x,
        InvertTarget::GcCstickY => &mut config.gc.invert_cstick_y,
        InvertTarget::NunchukX => &mut config.wii.invert_nunchuk_x,
        InvertTarget::NunchukY => &mut config.wii.invert_nunchuk_y,
        InvertTarget::StickDpadX => &mut config.wii.invert_stick_dpad_x,
        InvertTarget::StickDpadY => &mut config.wii.invert_stick_dpad_y,
        InvertTarget::PointerX => &mut config.wii.invert_pointer_x,
        InvertTarget::PointerY => &mut config.wii.invert_pointer_y,
    }
}

fn invert_value(config: &InputConfig, target: InvertTarget) -> bool {
    let field = match target {
        InvertTarget::GcStickX => &config.gc.invert_stick_x,
        InvertTarget::GcStickY => &config.gc.invert_stick_y,
        InvertTarget::GcCstickX => &config.gc.invert_cstick_x,
        InvertTarget::GcCstickY => &config.gc.invert_cstick_y,
        InvertTarget::NunchukX => &config.wii.invert_nunchuk_x,
        InvertTarget::NunchukY => &config.wii.invert_nunchuk_y,
        InvertTarget::StickDpadX => &config.wii.invert_stick_dpad_x,
        InvertTarget::StickDpadY => &config.wii.invert_stick_dpad_y,
        InvertTarget::PointerX => &config.wii.invert_pointer_x,
        InvertTarget::PointerY => &config.wii.invert_pointer_y,
    };

    field.unwrap_or(false)
}

fn display_name(button: Button) -> &'static str {
    match button {
        Button::South => "South",
        Button::East => "East",
        Button::West => "West",
        Button::North => "North",
        Button::L1 => "L1",
        Button::R1 => "R1",
        Button::L2 => "L2",
        Button::R2 => "R2",
        Button::L3 => "L3",
        Button::R3 => "R3",
        Button::Start => "Start",
        Button::Select => "Select",
        Button::Guide => "Guide",
        Button::DpadUp => "D-Pad Up",
        Button::DpadDown => "D-Pad Down",
        Button::DpadLeft => "D-Pad Left",
        Button::DpadRight => "D-Pad Right",
        Button::TouchpadClick => "Touchpad",
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();

    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn bound(buttons: &[(Button, u16)], mask: u16) -> String {
    buttons
        .iter()
        .find(|(_, m)| *m == mask)
        .map(|(b, _)| self::display_name(*b).to_owned())
        .unwrap_or_else(|| "Unbound".to_owned())
}

fn effective(gc: &GcProfile, wii: &WiiProfile, target: BindTarget) -> String {
    match target {
        BindTarget::GcA => self::bound(&gc.buttons, pad::A),
        BindTarget::GcB => self::bound(&gc.buttons, pad::B),
        BindTarget::GcX => self::bound(&gc.buttons, pad::X),
        BindTarget::GcY => self::bound(&gc.buttons, pad::Y),
        BindTarget::GcStart => self::bound(&gc.buttons, pad::START),
        BindTarget::GcZ => self::bound(&gc.buttons, pad::Z),
        BindTarget::WiiA => self::bound(&wii.buttons, usb::BTN_A),
        BindTarget::WiiB => self::bound(&wii.buttons, usb::BTN_B),
        BindTarget::WiiOne => self::bound(&wii.buttons, usb::BTN_ONE),
        BindTarget::WiiTwo => self::bound(&wii.buttons, usb::BTN_TWO),
        BindTarget::WiiPlus => self::bound(&wii.buttons, usb::BTN_PLUS),
        BindTarget::WiiMinus => self::bound(&wii.buttons, usb::BTN_MINUS),
        BindTarget::WiiHome => self::bound(&wii.buttons, usb::BTN_HOME),
        BindTarget::NunchukC => self::display_name(wii.nunchuk_c).to_owned(),
        BindTarget::NunchukZ => self::display_name(wii.nunchuk_z).to_owned(),
        BindTarget::Shake => wii
            .shake
            .map(|b| self::display_name(b).to_owned())
            .unwrap_or_else(|| "None".to_owned()),
        BindTarget::Recenter => self::display_name(wii.recenter).to_owned(),
    }
}

pub fn pressed_button(fresh: u32) -> Option<Button> {
    Button::ALL.iter().copied().find(|b| fresh & b.mask() != 0)
}

pub fn overlay(
    palette: &Palette,
    config: &InputConfig,
    tab: InputTab,
    capture: Option<BindTarget>,
    pad: Option<&str>,
) -> Element<'static, Message> {
    let gc_profile = config.gc_profile();
    let wii_profile = config.wii_profile();

    let status = match pad {
        Some(name) => text(name.to_owned()).size(11).color(palette.accent),
        None => text("No gamepad detected").size(11).color(palette.text_mute),
    };

    let header = column![text("Controller Bindings").size(16).color(palette.text), status,].spacing(2);

    let tabs = row![
        self::chip(
            palette,
            "GameCube",
            Message::InputTab(InputTab::Gc),
            tab == InputTab::Gc
        ),
        self::chip(palette, "Wii", Message::InputTab(InputTab::Wii), tab == InputTab::Wii),
    ]
    .spacing(6);

    let mut body = column![].spacing(4);

    let targets: &[(BindTarget, &str)] = match tab {
        InputTab::Gc => &GC_TARGETS,
        InputTab::Wii => &WII_TARGETS,
    };

    for (target, label) in targets {
        let value = if capture == Some(*target) {
            "press a button".to_owned()
        } else {
            self::effective(&gc_profile, &wii_profile, *target)
        };

        body = body.push(self::binding_row(
            palette,
            label,
            value,
            *target,
            capture == Some(*target),
        ));
    }

    if tab == InputTab::Gc {
        body = body
            .push(self::separator(palette))
            .push(self::invert_row(
                palette,
                config,
                "Invert Stick",
                InvertTarget::GcStickX,
                InvertTarget::GcStickY,
            ))
            .push(self::invert_row(
                palette,
                config,
                "Invert C-Stick",
                InvertTarget::GcCstickX,
                InvertTarget::GcCstickY,
            ));
    }

    if tab == InputTab::Wii {
        let options: Vec<String> = POINTER_SOURCES.iter().map(|s| s.to_string()).collect();
        let selected = config.wii.pointer.as_deref().unwrap_or("auto").to_string();

        let surface = palette.surface;
        let surface_2 = palette.surface_2;
        let border_2 = palette.border_2;
        let text_dim = palette.text_dim;
        let accent = palette.accent;
        let pointer_row = row![
            container(text("Pointer").size(13).color(palette.text_dim)).width(Length::Fixed(130.0)),
            pick_list(Some(selected), options, |source: &String| self::capitalize(source))
                .on_select(Message::InputPointer)
                .text_size(12)
                .padding(Padding::from([4, 10]))
                .width(Length::Fixed(140.0))
                .style(move |_, status| {
                    let bg = match status {
                        iced::widget::pick_list::Status::Hovered | iced::widget::pick_list::Status::Opened { .. } => {
                            surface_2
                        }
                        _ => surface,
                    };
                    iced::widget::pick_list::Style {
                        text_color: text_dim,
                        placeholder_color: text_dim,
                        handle_color: text_dim,
                        background: Background::Color(bg),
                        border: Border {
                            color: border_2,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                    }
                })
                .menu_style(move |_| iced::widget::overlay::menu::Style {
                    background: Background::Color(surface),
                    border: Border {
                        color: border_2,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    text_color: text_dim,
                    selected_text_color: accent,
                    selected_background: Background::Color(surface_2),
                    shadow: iced::Shadow::default(),
                }),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let sensitivity = wii_profile.sensitivity;
        let sensitivity_row = row![
            container(text("Sensitivity").size(13).color(palette.text_dim)).width(Length::Fixed(130.0)),
            self::chip(palette, "-", Message::InputSensitivity(-0.25), false),
            container(text(format!("{sensitivity:.2}")).size(13).color(palette.text))
                .width(Length::Fixed(48.0))
                .align_x(Alignment::Center),
            self::chip(palette, "+", Message::InputSensitivity(0.25), false),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let toggles = row![
            container(text("Options").size(13).color(palette.text_dim)).width(Length::Fixed(130.0)),
            self::chip(palette, "Sideways", Message::InputToggleSideways, wii_profile.sideways),
            self::chip(
                palette,
                "Stick D-Pad",
                Message::InputToggleStickDpad,
                wii_profile.stick_dpad
            ),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        body = body
            .push(self::separator(palette))
            .push(pointer_row)
            .push(sensitivity_row)
            .push(toggles)
            .push(self::invert_row(
                palette,
                config,
                "Invert Nunchuk",
                InvertTarget::NunchukX,
                InvertTarget::NunchukY,
            ))
            .push(self::invert_row(
                palette,
                config,
                "Invert Stick D-Pad",
                InvertTarget::StickDpadX,
                InvertTarget::StickDpadY,
            ))
            .push(self::invert_row(
                palette,
                config,
                "Invert Pointer",
                InvertTarget::PointerX,
                InvertTarget::PointerY,
            ));
    }

    let footer = row![
        self::chip(palette, "Reset to Defaults", Message::InputReset, false),
        container(text("")).width(Length::Fill),
        self::chip(palette, "Close", Message::InputClose, false),
    ]
    .align_y(Alignment::Center);

    let card_col = column![header, tabs, body, footer].spacing(14);

    overlay::modal(palette, 360.0, 20.0, Message::InputClose, card_col.into())
}

pub fn keyboard_overlay(
    palette: &Palette,
    keyboard: &KeyboardConfig,
    tab: KeyboardTab,
    capture: Option<KeyTarget>,
) -> Element<'static, Message> {
    let status = text("Click a binding, then press a key")
        .size(11)
        .color(palette.text_mute);

    let header = column![text("Keyboard Bindings").size(16).color(palette.text), status,].spacing(2);

    let tabs = row![
        self::chip(
            palette,
            "GameCube",
            Message::KeyboardTab(KeyboardTab::Gc),
            tab == KeyboardTab::Gc
        ),
        self::chip(
            palette,
            "Wii",
            Message::KeyboardTab(KeyboardTab::Wii),
            tab == KeyboardTab::Wii
        ),
        self::chip(
            palette,
            "Hotkeys",
            Message::KeyboardTab(KeyboardTab::Hotkeys),
            tab == KeyboardTab::Hotkeys
        ),
    ]
    .spacing(6);

    let keymap = keyboard.resolve();

    let targets: &[(KeyTarget, &str)] = match tab {
        KeyboardTab::Gc => keybinds::GC_KEY_TARGETS,
        KeyboardTab::Wii => keybinds::WII_KEY_TARGETS,
        KeyboardTab::Hotkeys => keybinds::HOTKEY_TARGETS,
    };

    let mut body = column![].spacing(4);
    for (target, label) in targets {
        body = body.push(self::key_binding_row(palette, label, *target, &keymap, capture));
    }

    let footer = row![
        self::chip(palette, "Reset to Defaults", Message::KeyboardReset, false),
        container(text("")).width(Length::Fill),
        self::chip(palette, "Close", Message::KeyboardClose, false),
    ]
    .align_y(Alignment::Center);

    let card_col = column![header, tabs, body, footer].spacing(14);

    overlay::modal(palette, 360.0, 20.0, Message::KeyboardClose, card_col.into())
}

fn key_binding_row(
    palette: &Palette,
    label: &'static str,
    target: KeyTarget,
    keymap: &Keymap,
    capture: Option<KeyTarget>,
) -> Element<'static, Message> {
    let capturing = capture == Some(target);
    let value = if capturing {
        "press a key".to_owned()
    } else {
        keybinds::key_label(keymap.code(target))
    };

    self::bind_row(palette, label, value, Message::KeyboardCapture(target), capturing)
}

fn control_style(
    surface: Color,
    surface_2: Color,
    border: Color,
    text_color: Color,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => surface_2,
            _ => surface,
        };

        button::Style {
            background: Some(Background::Color(bg)),
            text_color,
            border: Border {
                color: border,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        }
    }
}

fn binding_row(
    palette: &Palette,
    label: &'static str,
    value: String,
    target: BindTarget,
    capturing: bool,
) -> Element<'static, Message> {
    self::bind_row(palette, label, value, Message::InputCapture(target), capturing)
}

fn bind_row(
    palette: &Palette,
    label: &str,
    value: String,
    on_press: Message,
    capturing: bool,
) -> Element<'static, Message> {
    let value_color = if capturing { palette.accent } else { palette.text_dim };
    let border = if capturing { palette.accent } else { palette.border_2 };

    let bind = button(
        container(text(value).size(12).color(value_color))
            .width(Length::Fill)
            .align_x(Alignment::Center),
    )
    .width(Length::Fixed(180.0))
    .padding(Padding::from([4, 10]))
    .on_press(on_press)
    .style(self::control_style(
        palette.surface,
        palette.surface_2,
        border,
        value_color,
    ));

    row![
        container(text(label.to_owned()).size(13).color(palette.text)).width(Length::Fixed(130.0)),
        bind,
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn separator(palette: &Palette) -> Element<'static, Message> {
    let color = palette.border;

    container(
        container(text(""))
            .width(Length::Fill)
            .height(1)
            .style(move |_| container::Style {
                background: Some(Background::Color(color)),
                ..container::Style::default()
            }),
    )
    .padding(Padding::from([6, 0]))
    .into()
}

fn invert_row(
    palette: &Palette,
    config: &InputConfig,
    label: &'static str,
    x: InvertTarget,
    y: InvertTarget,
) -> Element<'static, Message> {
    row![
        container(text(label).size(13).color(palette.text_dim)).width(Length::Fixed(130.0)),
        self::chip(
            palette,
            "X",
            Message::InputToggleInvert(x),
            self::invert_value(config, x)
        ),
        self::chip(
            palette,
            "Y",
            Message::InputToggleInvert(y),
            self::invert_value(config, y)
        ),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn chip(palette: &Palette, label: &'static str, msg: Message, selected: bool) -> Element<'static, Message> {
    let text_color = if selected { palette.accent } else { palette.text_dim };
    let border = if selected { palette.accent } else { palette.border_2 };

    button(text(label).size(12).color(text_color))
        .padding(Padding::from([4, 10]))
        .on_press(msg)
        .style(self::control_style(
            palette.surface,
            palette.surface_2,
            border,
            text_color,
        ))
        .into()
}
