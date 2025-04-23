use iced::advanced::svg::Handle;
use iced::widget::Svg;
use iced::widget::svg::Style;
use iced::{Theme, widget::svg};

use crate::app::core::sizes::m;

pub enum Icon {
    Close,
    Front,
    Back,
}

fn icon_data(icon: Icon) -> Handle {
    match icon {
        Icon::Close => Handle::from_memory(include_bytes!("bootstrap/x-square-fill.svg")),
        Icon::Front => Handle::from_memory(include_bytes!("bootstrap/front.svg")),
        Icon::Back => Handle::from_memory(include_bytes!("bootstrap/back.svg")),
    }
}

pub fn icon<'a>(icon: Icon) -> Svg<'a, Theme> {
    svg(icon_data(icon))
        .width(m())
        .height(m())
        .style(|theme: &Theme, _state| Style {
            color: Some(theme.palette().text),
        })
}
