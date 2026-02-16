use iced::{
    Element,
    Length::Fill,
    widget::{container, pane_grid, radio, row, scrollable, text},
};

use crate::app::{
    Message,
    core::sizes::m,
    debugger::panes::{DebuggerPane, pane, title_bar},
};
use missingno_core::game_boy::video::{
    Video, control::Control, memory::Vram, palette::Palette, tile_maps::TileMapId,
    tiles::TileAddressMode,
};

use super::tile_atlas::tile_map_atlas;

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
        video: &Video,
        vram: &Vram,
        palette: &Palette,
    ) -> pane_grid::Content<'_, Message> {
        pane(
            title_bar(&self.title, DebuggerPane::TileMap(self.tile_map)),
            scrollable(
                container(tile_map_atlas(
                    vram.tile_map(self.tile_map),
                    video.control(),
                    vram,
                    palette,
                ))
                .center_x(Fill),
            )
            .into(),
        )
    }
}
