use iced::{
    Element,
    widget::{Button, button},
};

use crate::app::Message;

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
