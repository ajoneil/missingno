use iced::widget::{Text, text::IntoFragment};

use crate::app::core::{fonts, text};

pub fn m<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text::m(content).font(fonts::emoji()).into()
}

pub fn xl<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text::xl(content).font(fonts::emoji()).into()
}
