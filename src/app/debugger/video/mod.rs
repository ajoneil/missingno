use iced::{
    Element, Length,
    widget::{column, pane_grid, radio, row, rule, scrollable},
};

use crate::{
    app::{
        Message,
        core::sizes::m,
        debugger::panes::{DebuggerPane, checkbox_title_bar, pane},
    },
    game_boy::video::{Video, ppu::Mode},
};

use background_and_window::background_and_window;
use tile_maps::tile_address_mode;

mod background_and_window;
mod palette;
pub mod sprites;
mod tile_atlas;
pub mod tile_maps;
mod tile_widget;
pub mod tiles;

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
                DebuggerPane::Video,
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
                    rule::horizontal(1),
                    background_and_window(video),
                ]
                .spacing(m())
                .padding(m()),
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
