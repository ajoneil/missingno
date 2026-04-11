use iced::advanced::svg::Handle;
use iced::widget::Svg;
use iced::widget::svg::Style;
use iced::{Theme, widget::svg};

use crate::app::ui::text;

#[allow(dead_code)]
pub enum Icon {
    Back,
    Brush,
    Camera,
    CircuitBoard,
    Clock,
    Close,
    ColorsSwatch,
    Debug,
    Download,
    Expand,
    Eye,
    FolderOpen,
    Front,
    Gamepad,
    GameBoy,
    Gear,
    GitHub,
    Grid,
    Human,
    Image,
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
        Icon::Brush => Handle::from_memory(include_bytes!("pixelarticons/brush.svg")),
        Icon::Camera => Handle::from_memory(include_bytes!("pixelarticons/camera.svg")),
        Icon::CircuitBoard => {
            Handle::from_memory(include_bytes!("pixelarticons/circuit-board.svg"))
        }
        Icon::Clock => Handle::from_memory(include_bytes!("pixelarticons/clock.svg")),
        Icon::Close => Handle::from_memory(include_bytes!("pixelarticons/close.svg")),
        Icon::ColorsSwatch => {
            Handle::from_memory(include_bytes!("pixelarticons/colors-swatch.svg"))
        }
        Icon::Debug => Handle::from_memory(include_bytes!("pixelarticons/debug.svg")),
        Icon::Download => Handle::from_memory(include_bytes!("pixelarticons/download.svg")),
        Icon::Expand => Handle::from_memory(include_bytes!("pixelarticons/expand.svg")),
        Icon::Eye => Handle::from_memory(include_bytes!("pixelarticons/eye.svg")),
        Icon::FolderOpen => Handle::from_memory(include_bytes!("pixelarticons/folder.svg")),
        Icon::Front => Handle::from_memory(include_bytes!("pixelarticons/chevron-right.svg")),
        Icon::Gamepad => Handle::from_memory(include_bytes!("pixelarticons/gamepad.svg")),
        Icon::GameBoy => Handle::from_memory(include_bytes!("missingno.svg")),
        Icon::Gear => Handle::from_memory(include_bytes!("pixelarticons/settings-cog.svg")),
        Icon::GitHub => Handle::from_memory(include_bytes!("bootstrap/github.svg")),
        Icon::Grid => Handle::from_memory(include_bytes!("pixelarticons/grid.svg")),
        Icon::Human => Handle::from_memory(include_bytes!("pixelarticons/human.svg")),
        Icon::Image => Handle::from_memory(include_bytes!("pixelarticons/image.svg")),
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

pub fn m_colored<'a>(icon: Icon, color: iced::Color) -> Svg<'a, Theme> {
    svg(icon_data(icon))
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        .style(move |_: &Theme, _state| Style {
            color: Some(color),
        })
}

pub fn m_muted<'a>(icon: Icon) -> Svg<'a, Theme> {
    svg(icon_data(icon))
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        .style(|_: &Theme, _state| Style {
            color: Some(super::palette::MUTED),
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
