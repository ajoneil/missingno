use crate::{
    emulator::video::{
        Video,
        palette::{Palette, PaletteIndex},
        tiles::{Tile, TileBlock, TileBlockId},
    },
    ui::Message,
};
use iced::{
    Element,
    widget::{Column, Row, column, container, text},
};

use super::iced_color;

pub fn tile_blocks(video: &Video) -> Element<'static, Message> {
    column![
        tile_block(video, TileBlockId(0)),
        tile_block(video, TileBlockId(1))
    ]
    .spacing(10)
    .into()
}

fn tile_block(video: &Video, block: TileBlockId) -> Element<'static, Message> {
    column![text(block.to_string()), tiles(video.tile_block(block))].into()
}

fn tiles(block: &TileBlock) -> Element<'static, Message> {
    Column::from_iter((0..8).map(|row: u8| row_of_tiles(block, row))).into()
}

fn row_of_tiles(block: &TileBlock, row: u8) -> Element<'static, Message> {
    Row::from_iter((0..16).map(|col| tile(&block.tile(row * 8 + col)))).into()
}

fn tile(tile: &Tile) -> Element<'static, Message> {
    Column::from_iter(
        tile.rows()
            .iter()
            .map(|tile_row_pixels| tile_row(tile_row_pixels)),
    )
    .into()
}

fn tile_row(tile_row_pixels: &[PaletteIndex; 8]) -> Element<'static, Message> {
    Row::from_iter(tile_row_pixels.iter().map(|color| pixel(*color))).into()
}

fn pixel(color: PaletteIndex) -> Element<'static, Message> {
    let c = iced_color(Palette::MONOCHROME_GREEN.color(color));
    container("")
        .style(move |_| container::background(c))
        .width(1)
        .height(1)
        .into()
}
