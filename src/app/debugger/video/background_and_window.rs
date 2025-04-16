use iced::{
    Element,
    widget::{checkbox, column, row, text},
};

use crate::{
    app::{
        Message,
        core::sizes::{m, s},
        debugger::video::{palette::palette4, tile_map::tile_map},
    },
    emulator::video::{Video, palette::Palette},
};

pub fn background_and_window(video: &Video) -> Element<'static, Message> {
    column![
        row![
            checkbox(
                "Background & Window",
                video.control().background_and_window_enabled()
            ),
            checkbox("Window", video.control().window_enabled()),
        ]
        .spacing(m()),
        row![
            text("Palette"),
            palette4(&video.palettes().background, &Palette::MONOCHROME_GREEN)
        ]
        .spacing(m()),
        tile_map("Background Tile Map", video.control().background_tile_map()),
        tile_map("Window Tile Map", video.control().window_tile_map()),
    ]
    .spacing(s())
    .into()
}
