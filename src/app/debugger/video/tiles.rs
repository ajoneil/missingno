use iced::{
    Element,
    Length::Fill,
    widget::{column, pane_grid, row, scrollable, text},
};

use crate::{
    app::{
        Message,
        core::sizes::m,
        debugger::{
            panes::{DebuggerPane, pane, title_bar},
            video::tile_atlas::tile_block_atlas,
        },
    },
    game_boy::video::{Video, tiles::TileBlockId},
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
    column![
        text(block.to_string()),
        tile_block_atlas(video.tile_block(block))
    ]
    .into()
}
