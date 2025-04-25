use iced::{
    Alignment::Center,
    Element,
    widget::{Button, button, row, svg},
};

use crate::app::{
    Message,
    core::{
        icons::{self, Icon},
        sizes::s,
    },
};

pub fn standard<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).into()
}

pub fn success<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(button::success).into()
}

pub fn danger<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(button::danger).into()
}

pub fn text<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(button::text).into()
}

pub fn icon_label<'a>(icon: Icon, label: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        icons::m(icon).style(|theme, _status| svg::Style {
            color: Some(theme.extended_palette().primary.base.text)
        }),
        label.into()
    ]
    .spacing(s())
    .align_y(Center)
    .into()
}
