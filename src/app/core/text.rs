use iced::widget::{Text, text, text::IntoFragment};

use crate::app::core::sizes;

pub fn l<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::l()).into()
}
