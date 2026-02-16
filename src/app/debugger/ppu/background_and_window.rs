use iced::{
    Element,
    widget::{checkbox, column, row, text},
};

use crate::app::{
    Message,
    core::sizes::{m, s},
    debugger::ppu::{palette::palette4, tile_maps::tile_map_choice},
};
use missingno_core::game_boy::ppu::{Ppu, palette::Palette};

pub fn background_and_window(ppu: &Ppu, palette: &Palette) -> Element<'static, Message> {
    column![
        row![
            checkbox(ppu.control().background_and_window_enabled()).label("Background & Window"),
            checkbox(ppu.control().window_enabled()).label("Window"),
        ]
        .spacing(m()),
        row![
            text("Palette"),
            palette4(&ppu.palettes().background, palette)
        ]
        .spacing(m()),
        tile_map_choice("Background Tile Map", ppu.control().background_tile_map()),
        tile_map_choice("Window Tile Map", ppu.control().window_tile_map()),
    ]
    .spacing(s())
    .into()
}
