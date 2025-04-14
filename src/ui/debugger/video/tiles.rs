use super::tile_widget::tile;
use crate::{
    emulator::video::{
        Video,
        tiles::{TileBlock, TileBlockId},
    },
    ui::Message,
};
use iced::{
    Element,
    widget::{Column, Row, column, text},
};

pub fn tile_blocks(video: &Video) -> Element<'_, Message> {
    column![
        tile_block(video, TileBlockId(0)),
        tile_block(video, TileBlockId(1)),
        tile_block(video, TileBlockId(2))
    ]
    .spacing(10)
    .into()
}

fn tile_block(video: &Video, block: TileBlockId) -> Element<'_, Message> {
    column![text(block.to_string()), tiles(video.tile_block(block))].into()
}

fn tiles(block: &TileBlock) -> Element<'_, Message> {
    Column::from_iter((0..8).map(|row: u8| row_of_tiles(block, row))).into()
}

fn row_of_tiles(block: &TileBlock, row: u8) -> Element<'_, Message> {
    Row::from_iter((0..16).map(|col| tile(block.tile(row * 8 + col)))).into()
}
