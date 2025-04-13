use iced::{
    Element,
    widget::{checkbox, column, row, text},
};

use super::{palette::palette4, tile_map::tile_map_region};
use crate::{
    emulator::video::{Video, palette::Palette},
    ui::Message,
};

pub fn background_and_window(video: &Video) -> Element<'static, Message> {
    column![
        row![
            checkbox(
                "Background & Window Enabled",
                video.control().background_and_window_enabled()
            ),
            checkbox("Window Enabled", video.control().window_enabled()),
        ]
        .spacing(10),
        row![
            text("Palette"),
            palette4(&video.palettes().background, &Palette::MONOCHROME_GREEN)
        ]
        .spacing(10),
        tile_map_region(
            "Background Tile Map",
            video.control().background_tile_map_region()
        ),
        tile_map_region("Window Tile Map", video.control().window_tile_map_region()),
    ]
    .spacing(5)
    .into()
}
