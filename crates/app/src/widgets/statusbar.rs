use iced::widget::{Space, button, container, row, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding};

use crate::app::{Message, UpdateState};
use crate::game::CpuMode;
use crate::theme::Palette;

pub fn statusbar(
    palette: &Palette,
    cpu: CpuMode,
    count: usize,
    scanning: bool,
    update: &UpdateState,
) -> Element<'static, Message> {
    let led_color = match cpu {
        CpuMode::Jit => palette.accent,
        CpuMode::Interpreter => palette.text_mute,
    };

    let led = container(text("")).width(6).height(6).style(move |_| container::Style {
        background: Some(Background::Color(led_color)),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });

    let left = row![led, text(cpu.label()).size(11).color(palette.text_dim)]
        .spacing(8)
        .align_y(Alignment::Center);

    let count_label = if count == 1 {
        "1 game".to_owned()
    } else {
        format!("{count} games")
    };
    let count_text = if scanning {
        format!("Scanning...  ·  {count_label}")
    } else {
        count_label
    };
    let count_color = if scanning { palette.accent } else { palette.text_dim };
    let right = text(count_text).size(11).color(count_color);

    let content = row![
        left,
        Space::new().width(Length::Fill),
        self::update_center(palette, update),
        Space::new().width(Length::Fill),
        right,
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .height(Length::Fill);

    let bg = palette.bg_2;
    let border = palette.border;
    let text_color = palette.text_dim;

    container(content)
        .width(Length::Fill)
        .height(28)
        .padding(Padding::from([0, 12]))
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                color: border,
                width: 1.0,
                radius: 0.0.into(),
            },
            text_color: Some(text_color),
            ..container::Style::default()
        })
        .into()
}

fn update_center(palette: &Palette, update: &UpdateState) -> Element<'static, Message> {
    let muted =
        |label: &'static str| -> Element<'static, Message> { text(label).size(11).color(palette.text_mute).into() };

    match update {
        UpdateState::Idle => Space::new().into(),
        UpdateState::Checking => muted("Checking for updates..."),
        UpdateState::UpToDate => muted("Up to date"),
        UpdateState::Unpublished => muted("Development build"),
        UpdateState::Failed => muted("Update check failed"),
        UpdateState::Available(url) => {
            let accent = palette.accent;
            button(text("Update available").size(11).color(accent))
                .padding(Padding::from([2, 8]))
                .on_press(Message::OpenUrl(url.clone()))
                .style(move |_, status| {
                    let bg = match status {
                        button::Status::Hovered | button::Status::Pressed => Color { a: 0.10, ..accent },
                        _ => Color::TRANSPARENT,
                    };
                    button::Style {
                        background: Some(Background::Color(bg)),
                        text_color: accent,
                        border: Border {
                            radius: 4.0.into(),
                            ..Border::default()
                        },
                        ..button::Style::default()
                    }
                })
                .into()
        }
    }
}
