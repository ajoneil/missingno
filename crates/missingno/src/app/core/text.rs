use iced::widget::{Text, text, text::IntoFragment};

use crate::app::core::fonts;

pub mod sizes {
    use crate::app::core::sizes;

    pub fn m() -> f32 {
        sizes::m()
    }

    pub fn xl() -> f32 {
        m() * 2.0
    }
}

pub fn m<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::m()).into()
}

pub fn xl<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::xl()).font(fonts::heading()).into()
}
