use iced::{
    Element,
    widget::{radio, row, text},
};

use crate::{
    emulator::video::{control::Control, tile::TileAddressMode, tile_map::TileMapRegion},
    ui::Message,
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
    .spacing(10)
    .into()
}

pub fn tile_map_region(label: &str, region: TileMapRegion) -> Element<'_, Message> {
    row![
        text(label),
        radio(
            TileMapRegion::Map9800.to_string(),
            TileMapRegion::Map9800,
            Some(region),
            |_| -> Message { Message::None }
        ),
        radio(
            TileMapRegion::Map9c00.to_string(),
            TileMapRegion::Map9c00,
            Some(region),
            |_| -> Message { Message::None }
        )
    ]
    .spacing(10)
    .into()
}
