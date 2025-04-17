use iced::{
    Element,
    widget::{Column, Row, column, radio, row, text},
};

use crate::{
    app::{Message, core::sizes::m},
    emulator::video::{
        Video,
        control::Control,
        tile_maps::{TileMap, TileMapId},
        tiles::TileAddressMode,
    },
};

use super::tile_widget::tile;

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

pub fn tile_map_choice(label: &str, tile_map: TileMapId) -> Element<'_, Message> {
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

pub fn tile_maps(video: &Video) -> Element<'static, Message> {
    row![tile_map(video, TileMapId(0)), tile_map(video, TileMapId(1))]
        .spacing(m())
        .wrap()
        .into()
}

fn tile_map(video: &Video, map: TileMapId) -> Element<'static, Message> {
    column![text(map.to_string()), tiles(video, video.tile_map(map))].into()
}

fn tiles(video: &Video, map: &TileMap) -> Element<'static, Message> {
    Column::from_iter((0..32).map(|row: u8| row_of_tiles(video, map, row))).into()
}

fn row_of_tiles(video: &Video, map: &TileMap, row: u8) -> Element<'static, Message> {
    Row::from_iter((0..32).map(|col| {
        let map_tile_index = map.get_tile(col, row);
        let (block, mapped_index) = video.control().tile_address_mode().tile(map_tile_index);
        tile(video.tile_block(block).tile(mapped_index))
    }))
    .into()
}
