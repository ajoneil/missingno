use iced::{
    Element,
    widget::{radio, row, text},
};

use crate::{
    app::{Message, core::sizes::m},
    emulator::video::{control::Control, tile_maps::TileMapId, tiles::TileAddressMode},
};

pub fn tile_address_mode(control: Control) -> Element<'static, Message> {
    row![
        text("Tiles"),
        radio(
            TileAddressMode::Block0Block1.to_string(),
            TileAddressMode::Block0Block1,
            Some(control.tile_address_mode()),
            |_| -> Message { Message::None }
        ),
        radio(
            TileAddressMode::Block2Block1.to_string(),
            TileAddressMode::Block2Block1,
            Some(control.tile_address_mode()),
            |_| -> Message { Message::None }
        )
    ]
    .spacing(m())
    .into()
}

pub fn tile_map(label: &str, tile_map: TileMapId) -> Element<'_, Message> {
    row![
        text(label),
        radio(
            TileMapId(0).to_string(),
            TileMapId(0),
            Some(tile_map),
            |_| -> Message { Message::None }
        ),
        radio(
            TileMapId(1).to_string(),
            TileMapId(1),
            Some(tile_map),
            |_| -> Message { Message::None }
        )
    ]
    .spacing(m())
    .into()
}
