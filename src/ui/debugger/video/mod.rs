mod background_and_window;
mod palette;
mod sprites;
mod tile_map;
mod tile_widget;
mod tiles;

use super::panes::{checkbox_title_bar, pane};
use crate::{
    emulator::video::{Video, ppu::Mode},
    ui::{Message, styles::spacing},
};
use background_and_window::background_and_window;
use iced::{
    Color, Element, Length,
    widget::{column, horizontal_rule, pane_grid, radio, row},
};
use rgb::RGB8;
use sprites::sprites;
use tile_map::tile_address_mode;
use tiles::tile_blocks;

pub fn video_pane(video: &Video) -> pane_grid::Content<'_, Message> {
    pane(
        checkbox_title_bar("Video", video.control().video_enabled()),
        column![
            row![
                mode_radio(video.mode(), Mode::BetweenFrames),
                mode_radio(video.mode(), Mode::PreparingScanline)
            ],
            row![
                mode_radio(video.mode(), Mode::DrawingPixels),
                mode_radio(video.mode(), Mode::FinishingScanline),
            ],
            tile_address_mode(video.control()),
            horizontal_rule(1),
            background_and_window(video),
            horizontal_rule(1),
            sprites(video),
            horizontal_rule(1),
            tile_blocks(video)
        ]
        .spacing(spacing::m())
        .into(),
    )
}

fn mode_radio(current_mode: Mode, mode: Mode) -> Element<'static, Message> {
    radio(mode.to_string(), mode, Some(current_mode), |_| -> Message {
        Message::None
    })
    .width(Length::Fill)
    .into()
}

fn iced_color(color: RGB8) -> Color {
    Color::from_rgb8(color.r, color.g, color.b)
}
