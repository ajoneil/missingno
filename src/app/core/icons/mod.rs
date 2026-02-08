use iced::advanced::svg::Handle;
use iced::widget::Svg;
use iced::widget::svg::Style;
use iced::{Theme, widget::svg};

use crate::app::core::text;

pub enum Icon {
    Close,
    Front,
    Back,
    Fullscreen,
    GameBoy,
    Globe,
    GitHub,
    BreakpointEnabled,
    // BreakpointDisabled,
}

fn icon_data(icon: Icon) -> Handle {
    match icon {
        Icon::Close => Handle::from_memory(include_bytes!("bootstrap/x-square-fill.svg")),

        Icon::Front => Handle::from_memory(include_bytes!("bootstrap/front.svg")),
        Icon::Back => Handle::from_memory(include_bytes!("bootstrap/back.svg")),
        Icon::Fullscreen => Handle::from_memory(include_bytes!("bootstrap/fullscreen.svg")),
        Icon::GameBoy => Handle::from_memory(include_bytes!("missingno.svg")),
        Icon::Globe => Handle::from_memory(include_bytes!("bootstrap/globe.svg")),
        Icon::GitHub => Handle::from_memory(include_bytes!("bootstrap/github.svg")),
        Icon::BreakpointEnabled => {
            Handle::from_memory(include_bytes!("bootstrap/exclamation-octagon-fill.svg"))
        } // Icon::BreakpointDisabled => {
          //     Handle::from_memory(include_bytes!("bootstrap/exclamation-octagon.svg"))
          // }
    }
}

pub fn m<'a>(icon: Icon) -> Svg<'a, Theme> {
    svg(icon_data(icon))
        .width(text::sizes::m())
        .height(text::sizes::m())
        .style(|theme: &Theme, _state| Style {
            color: Some(theme.palette().text),
        })
}

// pub fn l<'a>(icon: Icon) -> Svg<'a, Theme> {
//     svg(icon_data(icon))
//         .width(text::sizes::l())
//         .height(text::sizes::l())
//         .style(|theme: &Theme, _state| Style {
//             color: Some(theme.palette().text),
//         })
// }

pub fn xl<'a>(icon: Icon) -> Svg<'a, Theme> {
    svg(icon_data(icon))
        .width(text::sizes::xl())
        .height(text::sizes::xl())
        .style(|theme: &Theme, _state| Style {
            color: Some(theme.palette().text),
        })
}

pub fn breakpoint_enabled() -> Svg<'static, Theme> {
    m(Icon::BreakpointEnabled).style(|theme: &Theme, _state| Style {
        color: Some(theme.extended_palette().danger.strong.color),
    })
}
