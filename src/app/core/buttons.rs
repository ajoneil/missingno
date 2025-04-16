use iced::{
    Element,
    widget::{Button, button},
};

use crate::app::Message;

pub fn success<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    button(content).style(button::success).into()
}
