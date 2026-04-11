use iced::Element;

use crate::app::Message;

pub mod buttons;
pub mod containers;
pub mod fonts;
pub mod icons;
pub mod palette;
pub mod sizes;
pub mod text;

pub fn horizontal_rule() -> Element<'static, Message> {
    iced::widget::rule::horizontal(1).into()
}

pub fn menu_divider() -> Element<'static, Message> {
    iced::widget::rule::horizontal(1)
        .style(|_: &iced::Theme| iced::widget::rule::Style {
            color: iced::Color::from_rgba(1.0, 1.0, 1.0, 0.15),
            radius: Default::default(),
            fill_mode: iced::widget::rule::FillMode::Full,
            snap: true,
        })
        .into()
}
