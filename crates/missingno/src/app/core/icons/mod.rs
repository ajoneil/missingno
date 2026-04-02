use iced::advanced::svg::Handle;
use iced::widget::Svg;
use iced::widget::svg::Style;
use iced::{Theme, widget::svg};

use crate::app::core::text;

pub enum Icon {
    Back,
    Close,
    Expand,
    Front,
    Gamepad,
    GameBoy,
    Gear,
    GitHub,
    Globe,
    Monitor,
    Sliders,
    Warning,
    Wifi,
}

fn icon_data(icon: Icon) -> Handle {
    match icon {
        Icon::Back => Handle::from_memory(include_bytes!("pixelarticons/chevron-left.svg")),
        Icon::Close => Handle::from_memory(include_bytes!("pixelarticons/close.svg")),
        Icon::Expand => Handle::from_memory(include_bytes!("pixelarticons/expand.svg")),
        Icon::Front => Handle::from_memory(include_bytes!("pixelarticons/chevron-right.svg")),
        Icon::Gamepad => Handle::from_memory(include_bytes!("pixelarticons/gamepad.svg")),
        Icon::GameBoy => Handle::from_memory(include_bytes!("missingno.svg")),
        Icon::Gear => Handle::from_memory(include_bytes!("pixelarticons/settings-cog.svg")),
        Icon::GitHub => Handle::from_memory(include_bytes!("bootstrap/github.svg")),
        Icon::Globe => Handle::from_memory(include_bytes!("pixelarticons/globe.svg")),
        Icon::Monitor => Handle::from_memory(include_bytes!("pixelarticons/monitor.svg")),
        Icon::Sliders => Handle::from_memory(include_bytes!("pixelarticons/sliders.svg")),
        Icon::Warning => Handle::from_memory(include_bytes!("pixelarticons/warning-diamond.svg")),
        Icon::Wifi => Handle::from_memory(include_bytes!("pixelarticons/wifi.svg")),
    }
}

const ICON_SIZE: f32 = 24.0;

pub fn m<'a>(icon: Icon) -> Svg<'a, Theme> {
    svg(icon_data(icon))
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        .style(|theme: &Theme, _state| Style {
            color: Some(theme.palette().text),
        })
}

pub fn xl<'a>(icon: Icon) -> Svg<'a, Theme> {
    svg(icon_data(icon))
        .width(text::sizes::xl())
        .height(text::sizes::xl())
        .style(|theme: &Theme, _state| Style {
            color: Some(theme.palette().text),
        })
}

pub fn breakpoint_enabled() -> Svg<'static, Theme> {
    m(Icon::Warning).style(|theme: &Theme, _state| Style {
        color: Some(theme.extended_palette().danger.strong.color),
    })
}
