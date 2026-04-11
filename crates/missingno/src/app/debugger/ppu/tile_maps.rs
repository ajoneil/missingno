use iced::{
    Length::Fill,
    widget::{container, pane_grid, scrollable},
};

use crate::app::{
    Message,
    debugger::panes::{pane, title_bar},
};
use missingno_gb::ppu::{
    Ppu, memory::Vram, types::palette::Palette,
    types::tile_maps::TileMapId,
};

use super::tile_atlas::tile_map_atlas;

pub struct TileMapPane {
    tile_map: TileMapId,
    title: String,
}

impl TileMapPane {
    pub fn new(tile_map: TileMapId) -> Self {
        Self {
            tile_map,
            title: tile_map.to_string(),
        }
    }

    pub fn content(
        &self,
        ppu: &Ppu,
        vram: &Vram,
        palette: &Palette,
    ) -> pane_grid::Content<'_, Message> {
        pane(
            title_bar(&self.title),
            scrollable(
                container(tile_map_atlas(
                    vram.tile_map(self.tile_map),
                    ppu.control(),
                    vram,
                    palette,
                ))
                .center_x(Fill),
            )
            .into(),
        )
    }
}
