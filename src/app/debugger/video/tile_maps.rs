use iced::{
    Element,
    Length::Fill,
    widget::{Column, Row, container, pane_grid, radio, row, scrollable, text},
};

use crate::{
    app::{
        Message,
        core::sizes::m,
        debugger::panes::{DebuggerPane, pane, title_bar},
    },
    game_boy::video::{
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

    pub fn content(&self, video: &Video) -> pane_grid::Content<'_, Message> {
        pane(
            title_bar(&self.title, DebuggerPane::TileMap(self.tile_map)),
            scrollable(container(self.tiles(video, video.tile_map(self.tile_map))).center_x(Fill))
                .into(),
        )
    }

    fn tiles(&self, video: &Video, map: &TileMap) -> Element<'_, Message> {
        Column::from_iter((0..32).map(|row: u8| self.row_of_tiles(video, map, row))).into()
    }

    fn row_of_tiles(&self, video: &Video, map: &TileMap, row: u8) -> Element<'_, Message> {
        Row::from_iter((0..32).map(|col| {
            let map_tile_index = map.get_tile(col, row);
            let (block, mapped_index) = video.control().tile_address_mode().tile(map_tile_index);
            tile(video.tile_block(block).tile(mapped_index)).into()
        }))
        .into()
    }
}
