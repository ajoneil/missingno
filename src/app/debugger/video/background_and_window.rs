use iced::{
    Element,
    widget::{checkbox, column, row, text},
};

use crate::{
    app::{
        Message,
        core::sizes::{m, s},
        debugger::video::{palette::palette4, tile_maps::tile_map_choice},
    },
    game_boy::video::{Video, palette::Palette},
};

pub fn background_and_window(video: &Video) -> Element<'static, Message> {
    column![
        row![
            checkbox(video.control().background_and_window_enabled()).label("Background & Window"),
            checkbox(video.control().window_enabled()).label("Window"),
        ]
        .spacing(m()),
        row![
            text("Palette"),
            palette4(&video.palettes().background, &Palette::MONOCHROME_GREEN)
        ]
        .spacing(m()),
        tile_map_choice("Background Tile Map", video.control().background_tile_map()),
        tile_map_choice("Window Tile Map", video.control().window_tile_map()),
    ]
    .spacing(s())
    .into()
}
