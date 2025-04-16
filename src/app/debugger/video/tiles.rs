use iced::{
    Element,
    widget::{Column, Row, column, row, text},
};

use crate::{
    app::{Message, core::sizes::m, debugger::video::tile_widget::tile},
    emulator::video::{
        Video,
        tiles::{TileBlock, TileBlockId, TileIndex},
    },
};

pub fn tile_blocks(video: &Video) -> Element<'static, Message> {
    row![
        tile_block(video, TileBlockId(0)),
        tile_block(video, TileBlockId(1)),
        tile_block(video, TileBlockId(2))
    ]
    .spacing(m())
    .wrap()
    .into()
}

fn tile_block(video: &Video, block: TileBlockId) -> Element<'static, Message> {
    column![text(block.to_string()), tiles(video.tile_block(block))].into()
}

fn tiles(block: &TileBlock) -> Element<'static, Message> {
    Column::from_iter((0..8).map(|row: u8| row_of_tiles(block, row))).into()
}

fn row_of_tiles(block: &TileBlock, row: u8) -> Element<'static, Message> {
    Row::from_iter((0..16).map(|col| tile(block.tile(TileIndex(row * 8 + col))))).into()
}
