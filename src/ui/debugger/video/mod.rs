mod background_and_window;
mod palette;
mod sprites;
mod tile_map;

use crate::{
    emulator::video::{Video, ppu::Mode},
    ui::Message,
};

use iced::{
    Element, Length,
    widget::{checkbox, column, horizontal_rule, radio, row},
};
use sprites::sprites;
use tile_map::tile_address_mode;

pub fn video(video: &Video) -> Element<'_, Message> {
    column![
        row![checkbox("Video", video.control().video_enabled()),].spacing(10),
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
        background_and_window::background_and_window(video),
        horizontal_rule(1),
        sprites(video)
    ]
    .spacing(10)
    .into()
}

fn mode_radio(current_mode: Mode, mode: Mode) -> Element<'static, Message> {
    radio(mode.to_string(), mode, Some(current_mode), |_| -> Message {
        Message::None
    })
    .width(Length::Fill)
    .into()
}
