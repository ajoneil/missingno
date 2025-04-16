use iced::widget::{Text, text, text::IntoFragment};

use crate::app::core::sizes;

pub fn m<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::m()).into()
}

// pub fn l<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
//     text(content).size(sizes::l()).into()
// }

pub fn xl<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::xl()).into()
}

pub fn xxl<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::xxl()).into()
}
