use iced::{
    Element,
    Length::Fill,
    widget::{column, pane_grid, row, scrollable, text},
};

use crate::app::{
    Message,
    core::sizes::m,
    debugger::{
        panes::{DebuggerPane, pane, title_bar},
        video::tile_atlas::tile_block_atlas,
    },
};
use missingno_core::game_boy::video::{memory::Vram, palette::Palette, tiles::TileBlockId};

pub struct TilesPane;

impl TilesPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(&self, vram: &Vram, palette: &Palette) -> pane_grid::Content<'_, Message> {
        pane(
            title_bar("Tiles", DebuggerPane::Tiles),
            scrollable(
                row![
                    tile_block(vram, TileBlockId(0), palette),
                    tile_block(vram, TileBlockId(1), palette),
                    tile_block(vram, TileBlockId(2), palette)
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

fn tile_block(vram: &Vram, block: TileBlockId, palette: &Palette) -> Element<'static, Message> {
    column![
        text(block.to_string()),
        tile_block_atlas(vram.tile_block(block), palette)
    ]
    .into()
}
