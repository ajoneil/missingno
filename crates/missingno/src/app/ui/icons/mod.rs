use iced::advanced::svg::Handle;
use iced::widget::Svg;
use iced::widget::svg::Style;
use iced::{Theme, widget::svg};

use crate::app::ui::text;

#[allow(dead_code)]
pub enum Icon {
    Back,
    Camera,
    CircuitBoard,
    Close,
    Debug,
    Download,
    Expand,
    FolderOpen,
    Front,
    Gamepad,
    GameBoy,
    Gear,
    GitHub,
    Info,
    Globe,
    Menu,
    Monitor,
    Play,
    Sliders,
    Warning,
    Wifi,
}

fn icon_data(icon: Icon) -> Handle {
    match icon {
        Icon::Back => Handle::from_memory(include_bytes!("pixelarticons/chevron-left.svg")),
        Icon::Camera => Handle::from_memory(include_bytes!("pixelarticons/camera.svg")),
        Icon::CircuitBoard => {
            Handle::from_memory(include_bytes!("pixelarticons/circuit-board.svg"))
        }
        Icon::Close => Handle::from_memory(include_bytes!("pixelarticons/close.svg")),
        Icon::Debug => Handle::from_memory(include_bytes!("pixelarticons/debug.svg")),
        Icon::Download => Handle::from_memory(include_bytes!("pixelarticons/download.svg")),
        Icon::Expand => Handle::from_memory(include_bytes!("pixelarticons/expand.svg")),
        Icon::FolderOpen => Handle::from_memory(include_bytes!("pixelarticons/folder.svg")),
        Icon::Front => Handle::from_memory(include_bytes!("pixelarticons/chevron-right.svg")),
        Icon::Gamepad => Handle::from_memory(include_bytes!("pixelarticons/gamepad.svg")),
        Icon::GameBoy => Handle::from_memory(include_bytes!("missingno.svg")),
        Icon::Gear => Handle::from_memory(include_bytes!("pixelarticons/settings-cog.svg")),
        Icon::GitHub => Handle::from_memory(include_bytes!("bootstrap/github.svg")),
        Icon::Globe => Handle::from_memory(include_bytes!("pixelarticons/globe.svg")),
        Icon::Menu => Handle::from_memory(include_bytes!("pixelarticons/menu.svg")),
        Icon::Info => Handle::from_memory(include_bytes!("pixelarticons/info-box.svg")),
        Icon::Monitor => Handle::from_memory(include_bytes!("pixelarticons/monitor.svg")),
        Icon::Play => Handle::from_memory(include_bytes!("pixelarticons/play.svg")),
        Icon::Sliders => Handle::from_memory(include_bytes!("pixelarticons/sliders.svg")),
        Icon::Warning => Handle::from_memory(include_bytes!("pixelarticons/warning-diamond.svg")),
        Icon::Wifi => Handle::from_memory(include_bytes!("pixelarticons/wifi.svg")),
    }
}

pub const ICON_SIZE: f32 = 24.0;

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
