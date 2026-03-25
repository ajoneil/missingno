use iced::widget::{Text, text, text::IntoFragment};

pub mod sizes {
    use crate::app::core::sizes;

    // pub fn s() -> f32 {
    //     m() * 0.875
    // }

    pub fn m() -> f32 {
        sizes::m()
    }

    // pub fn l() -> f32 {
    //     m() * 1.5
    // }

    pub fn xl() -> f32 {
        m() * 2.0
    }
}

// pub fn s<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
//     text(content).size(sizes::s() * 0.875).into()
// }

pub fn m<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::m()).into()
}

// pub fn l<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
//     text(content).size(sizes::l()).into()
// }

pub fn xl<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    text(content).size(sizes::xl()).into()
}
