use iced::{
    Element, Length,
    widget::{column, horizontal_rule, pane_grid, radio, row, scrollable},
};

use crate::{
    app::{
        Message,
        core::sizes::m,
        debugger::panes::{DebuggerPane, checkbox_title_bar, pane},
    },
    emulator::video::{Video, ppu::Mode},
};

use background_and_window::background_and_window;
use sprites::sprites;
use tile_map::{tile_address_mode, tile_maps};
use tiles::tile_blocks;

mod background_and_window;
mod palette;
mod sprites;
mod tile_map;
mod tile_widget;
mod tiles;

pub struct VideoPane;

impl VideoPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(&self, video: &Video) -> pane_grid::Content<'_, Message> {
        pane(
            checkbox_title_bar(
                "Video",
                video.control().video_enabled(),
                Some(DebuggerPane::Video),
            ),
            scrollable(
                column![
                    row![
                        self.mode_radio(video.mode(), Mode::BetweenFrames),
                        self.mode_radio(video.mode(), Mode::PreparingScanline)
                    ],
                    row![
                        self.mode_radio(video.mode(), Mode::DrawingPixels),
                        self.mode_radio(video.mode(), Mode::BetweenLines),
                    ],
                    tile_address_mode(video.control()),
                    horizontal_rule(1),
                    background_and_window(video),
                    horizontal_rule(1),
                    sprites(video),
                    horizontal_rule(1),
                    tile_blocks(video),
                    horizontal_rule(1),
                    tile_maps(video),
                ]
                .spacing(m()),
            )
            .into(),
        )
    }

    fn mode_radio(&self, current_mode: Mode, mode: Mode) -> Element<'_, Message> {
        radio(mode.to_string(), mode, Some(current_mode), |_| -> Message {
            Message::None
        })
        .width(Length::Fill)
        .into()
    }
}
