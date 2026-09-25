use iced::widget::{Space, button, container, row, text};
use iced::{Background, Border, Color, Element, Length, Padding};
use iced_aw::menu::{self, Item, Menu, MenuBar};

use crate::app::Message;
use crate::game::{AspectMode, CpuMode, Platform, ThemePreference};
use crate::theme::Palette;

const COMMUNITY_COMPAT_URL: &str = "https://gecko.layle.dev/compat";
const WII_DB_URL: &str = "https://emu.layle.dev/gecko-wii";
const GC_DB_URL: &str = "https://emu.layle.dev/gecko-gc";

pub fn menubar(
    palette: &Palette,
    cpu: CpuMode,
    theme_pref: ThemePreference,
    ipl_hle: bool,
    skip_ipl: bool,
    upscale: u32,
    texture_packs: bool,
    aspect: AspectMode,
    memcard_enabled: bool,
    sram_enabled: bool,
) -> Element<'static, Message> {
    let bar_bg = palette.bg_2;
    let border = palette.border;
    let border_2 = palette.border_2;
    let surface = palette.surface;
    let bar = MenuBar::new(vec![
        Item::with_menu(self::top_label(palette, "File"), self::file_menu(palette)),
        Item::with_menu(
            self::top_label(palette, "Settings"),
            self::settings_menu(
                palette,
                cpu,
                theme_pref,
                ipl_hle,
                skip_ipl,
                upscale,
                texture_packs,
                aspect,
                memcard_enabled,
                sram_enabled,
            ),
        ),
        Item::with_menu(
            self::top_label(palette, "Compatibility"),
            self::compatibility_menu(palette),
        ),
        Item::with_menu(self::top_label(palette, "About"), self::about_menu(palette)),
    ])
    .draw_path(menu::DrawPath::Backdrop)
    .style(move |_: &iced::Theme, _status| menu::Style {
        bar_background: Background::Color(bar_bg),
        bar_border: Border {
            color: border,
            width: 0.0,
            radius: 0.0.into(),
        },
        bar_shadow: iced::Shadow::default(),
        menu_background: Background::Color(surface),
        menu_border: Border {
            color: border_2,
            width: 1.0,
            radius: 8.0.into(),
        },
        menu_shadow: iced::Shadow::default(),
        path: Background::Color(surface),
        path_border: Border {
            radius: 0.0.into(),
            ..Border::default()
        },
    });

    container(bar)
        .width(Length::Fill)
        .height(30)
        .padding(Padding {
            top: 0.0,
            right: 6.0,
            bottom: 0.0,
            left: 6.0,
        })
        .style(move |_| container::Style {
            background: Some(Background::Color(bar_bg)),
            border: Border {
                color: border,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn top_label(palette: &Palette, label: &'static str) -> Element<'static, Message> {
    let dim = palette.text_dim;
    let text_color = palette.text;
    let hover = palette.surface;
    button(text(label).size(13).color(dim))
        .padding(Padding {
            top: 6.0,
            right: 11.0,
            bottom: 6.0,
            left: 11.0,
        })
        .on_press(Message::Noop)
        .style(move |_, status| self::top_button_style(status, hover, text_color))
        .into()
}

fn top_button_style(status: button::Status, hover: Color, text_color: Color) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => hover,
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    }
}

fn menu_item(palette: &Palette, label: &'static str, msg: Message, checked: Option<bool>) -> Element<'static, Message> {
    let text_color = palette.text;
    let hover = palette.surface_2;
    let (content, left_pad): (iced::widget::Row<'static, Message>, f32) = match checked {
        Some(is_on) => {
            let mark = if is_on { "✓" } else { " " };
            let r = row![
                text(mark).size(13).color(palette.accent).width(14),
                text(label).size(13).color(text_color),
            ]
            .spacing(4);
            (r, 10.0)
        }
        None => (row![text(label).size(13).color(text_color)], 14.0),
    };

    button(content.width(Length::Fill).align_y(iced::Alignment::Center))
        .width(Length::Fill)
        .padding(Padding {
            top: 6.0,
            right: 14.0,
            bottom: 6.0,
            left: left_pad,
        })
        .on_press(msg)
        .style(move |_, status| self::menu_item_style(status, hover, text_color))
        .into()
}

fn menu_item_style(status: button::Status, hover: Color, text_color: Color) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => hover,
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    }
}

fn separator(palette: &Palette) -> Element<'static, Message> {
    let color = palette.border;
    container(text(""))
        .width(Length::Fill)
        .height(1)
        .padding(Padding::from([4, 0]))
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            ..container::Style::default()
        })
        .into()
}

fn section_header(palette: &Palette, label: &'static str) -> Element<'static, Message> {
    container(text(label).size(10).color(palette.text_mute))
        .width(Length::Fill)
        .padding(Padding {
            top: 6.0,
            right: 14.0,
            bottom: 2.0,
            left: 14.0,
        })
        .into()
}

fn file_menu(palette: &Palette) -> Menu<'static, Message, iced::Theme, iced::Renderer> {
    Menu::new(vec![
        Item::new(self::menu_item(palette, "Open Game", Message::MenuOpenGame, None)),
        Item::new(self::menu_item(
            palette,
            "Open GameCube DOL",
            Message::MenuOpenDol(Platform::Gcn),
            None,
        )),
        Item::new(self::menu_item(
            palette,
            "Open Wii DOL",
            Message::MenuOpenDol(Platform::Wii),
            None,
        )),
        Item::new(self::separator(palette)),
        Item::new(self::menu_item(
            palette,
            "Set GameCube Folder",
            Message::MenuChooseLibrary(Platform::Gcn),
            None,
        )),
        Item::new(self::menu_item(
            palette,
            "Set Wii Folder",
            Message::MenuChooseLibrary(Platform::Wii),
            None,
        )),
        Item::new(self::menu_item(palette, "Rescan", Message::MenuRescan, None)),
        Item::new(self::separator(palette)),
        Item::new(self::menu_item(palette, "Quit", Message::MenuQuit, None)),
    ])
    .max_width(240.0)
    .offset(4.0)
    .spacing(2.0)
}

fn settings_menu(
    palette: &Palette,
    cpu: CpuMode,
    theme_pref: ThemePreference,
    ipl_hle: bool,
    skip_ipl: bool,
    upscale: u32,
    texture_packs: bool,
    aspect: AspectMode,
    memcard_enabled: bool,
    sram_enabled: bool,
) -> Menu<'static, Message, iced::Theme, iced::Renderer> {
    let check = |label: &'static str, msg: Message, on: bool| Item::new(self::menu_item(palette, label, msg, Some(on)));

    let mut boot_items = vec![check("IPL HLE (GameCube)", Message::MenuToggleIplHle, ipl_hle)];
    if !ipl_hle {
        boot_items.push(check("Skip IPL (GameCube)", Message::MenuToggleSkipIpl, skip_ipl));
    }
    boot_items.extend([
        check("Memory Card (Slot A)", Message::MenuToggleMemoryCard, memcard_enabled),
        check("Persist SRAM", Message::MenuToggleSram, sram_enabled),
    ]);

    Menu::new(vec![
        Item::new(self::menu_item(palette, "Guided Setup...", Message::MenuSetup, None)),
        Item::new(self::separator(palette)),
        self::submenu(
            palette,
            "Execution Engine",
            vec![
                check(
                    "JIT (Recompiler)",
                    Message::MenuToggleCpu(CpuMode::Jit),
                    cpu == CpuMode::Jit,
                ),
                check(
                    "Interpreter",
                    Message::MenuToggleCpu(CpuMode::Interpreter),
                    cpu == CpuMode::Interpreter,
                ),
            ],
        ),
        self::submenu(palette, "Boot", boot_items),
        self::submenu(
            palette,
            "Graphics",
            vec![
                check(
                    "Texture Packs (restart game)",
                    Message::MenuToggleTexturePacks,
                    texture_packs,
                ),
                Item::new(self::separator(palette)),
                Item::new(self::section_header(palette, "Internal Resolution")),
                check("1x (Native)", Message::MenuSetUpscale(1), upscale == 1),
                check("2x", Message::MenuSetUpscale(2), upscale == 2),
                check("3x", Message::MenuSetUpscale(3), upscale == 3),
                check("4x", Message::MenuSetUpscale(4), upscale == 4),
                Item::new(self::separator(palette)),
                Item::new(self::section_header(palette, "Aspect Ratio")),
                check(
                    "Auto (16:9 Wii, 4:3 GC)",
                    Message::MenuSetAspect(AspectMode::Auto),
                    aspect == AspectMode::Auto,
                ),
                check(
                    "4:3",
                    Message::MenuSetAspect(AspectMode::Standard),
                    aspect == AspectMode::Standard,
                ),
                check(
                    "16:9",
                    Message::MenuSetAspect(AspectMode::Widescreen),
                    aspect == AspectMode::Widescreen,
                ),
                check(
                    "Stretch",
                    Message::MenuSetAspect(AspectMode::Stretch),
                    aspect == AspectMode::Stretch,
                ),
            ],
        ),
        self::submenu(
            palette,
            "Theme",
            vec![
                check(
                    "System",
                    Message::MenuSetTheme(ThemePreference::System),
                    theme_pref == ThemePreference::System,
                ),
                check(
                    "Light",
                    Message::MenuSetTheme(ThemePreference::Light),
                    theme_pref == ThemePreference::Light,
                ),
                check(
                    "Dark",
                    Message::MenuSetTheme(ThemePreference::Dark),
                    theme_pref == ThemePreference::Dark,
                ),
            ],
        ),
        Item::new(self::separator(palette)),
        Item::new(self::menu_item(
            palette,
            "Controller Bindings",
            Message::MenuInputSettings,
            None,
        )),
        Item::new(self::menu_item(
            palette,
            "Keyboard Bindings",
            Message::MenuKeyboardSettings,
            None,
        )),
    ])
    .max_width(240.0)
    .offset(4.0)
    .spacing(2.0)
}

fn submenu(
    palette: &Palette,
    label: &'static str,
    items: Vec<Item<'static, Message, iced::Theme, iced::Renderer>>,
) -> Item<'static, Message, iced::Theme, iced::Renderer> {
    let text_color = palette.text;
    let hover = palette.surface_2;
    let content = row![
        text(label).size(13).color(text_color),
        Space::new().width(Length::Fill),
        crate::widgets::icons::chevron_right(palette.text_dim),
    ]
    .align_y(iced::Alignment::Center);

    let header = button(content)
        .width(Length::Fill)
        .padding(Padding {
            top: 6.0,
            right: 10.0,
            bottom: 6.0,
            left: 14.0,
        })
        .on_press(Message::Noop)
        .style(move |_, status| self::menu_item_style(status, hover, text_color));

    Item::with_menu(header, Menu::new(items).max_width(220.0).offset(6.0).spacing(2.0))
}

fn compatibility_menu(palette: &Palette) -> Menu<'static, Message, iced::Theme, iced::Renderer> {
    Menu::new(vec![
        Item::new(self::menu_item(
            palette,
            "Community Compatibility List",
            Message::OpenUrl(COMMUNITY_COMPAT_URL.to_owned()),
            None,
        )),
        Item::new(self::separator(palette)),
        Item::new(self::menu_item(
            palette,
            "Wii Screenshots Database",
            Message::OpenUrl(WII_DB_URL.to_owned()),
            None,
        )),
        Item::new(self::menu_item(
            palette,
            "GameCube Screenshots Database",
            Message::OpenUrl(GC_DB_URL.to_owned()),
            None,
        )),
    ])
    .max_width(220.0)
    .offset(4.0)
    .spacing(2.0)
}

fn about_menu(palette: &Palette) -> Menu<'static, Message, iced::Theme, iced::Renderer> {
    Menu::new(vec![
        Item::new(self::menu_item(palette, "About Gecko", Message::MenuAbout, None)),
        Item::new(self::menu_item(
            palette,
            "Check for Updates",
            Message::CheckForUpdates,
            None,
        )),
        Item::new(self::separator(palette)),
        Item::new(self::menu_item(
            palette,
            "Report an Issue",
            Message::OpenUrl(crate::app::ISSUE_URL.to_owned()),
            None,
        )),
    ])
    .max_width(200.0)
    .offset(4.0)
    .spacing(2.0)
}
