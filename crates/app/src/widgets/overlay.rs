use iced::widget::{container, mouse_area, stack, text};
use iced::{Background, Border, Color, Element, Length};

use crate::app::Message;
use crate::theme::Palette;

pub fn modal(
    palette: &Palette,
    width: f32,
    padding: f32,
    on_close: Message,
    content: Element<'static, Message>,
) -> Element<'static, Message> {
    let bg = palette.bg;
    let border_2 = palette.border_2;
    let backdrop_color = Color {
        a: 0.45,
        ..(if palette.is_dark { Color::BLACK } else { palette.text })
    };

    let backdrop: Element<'static, Message> = mouse_area(
        container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(backdrop_color)),
                ..container::Style::default()
            }),
    )
    .on_press(on_close)
    .into();

    let card = container(content)
        .width(Length::Fixed(width))
        .padding(padding)
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                color: border_2,
                width: 1.0,
                radius: 12.0.into(),
            },
            ..container::Style::default()
        });

    let centered = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}
