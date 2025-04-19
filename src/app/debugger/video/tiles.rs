use iced::{
    Element,
    Length::Fill,
    widget::{Column, Row, column, pane_grid, row, scrollable, text},
};

use crate::{
    app::{
        Message,
        core::sizes::m,
        debugger::{
            panes::{DebuggerPane, pane, title_bar},
            video::tile_widget::tile,
        },
    },
    emulator::video::{
        Video,
        tiles::{TileBlock, TileBlockId, TileIndex},
    },
};

pub struct TilesPane;

impl TilesPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(&self, video: &Video) -> pane_grid::Content<'_, Message> {
        pane(
            title_bar("Tiles", DebuggerPane::Tiles),
            scrollable(
                row![
                    tile_block(video, TileBlockId(0)),
                    tile_block(video, TileBlockId(1)),
                    tile_block(video, TileBlockId(2))
                ]
                .spacing(m())
                .padding(m())
                .width(Fill)
                .wrap(),
            )
            .into(),
        )
    }
}

fn tile_block(video: &Video, block: TileBlockId) -> Element<'static, Message> {
    column![text(block.to_string()), tiles(video.tile_block(block))].into()
}

fn tiles(block: &TileBlock) -> Element<'static, Message> {
    Column::from_iter((0..8).map(|row: u8| row_of_tiles(block, row))).into()
}

fn row_of_tiles(block: &TileBlock, row: u8) -> Element<'static, Message> {
    Row::from_iter((0..16).map(|col| tile(block.tile(TileIndex(row * 16 + col))).into())).into()
}
