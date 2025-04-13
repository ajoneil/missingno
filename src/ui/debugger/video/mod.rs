mod control;

use crate::{
    emulator::video::{Video, ppu::Mode},
    ui::Message,
};

use iced::{
    Element, Length,
    widget::{column, horizontal_rule, radio, row},
};

pub fn video(video: &Video) -> Element<'_, Message> {
    column![
        row![
            mode_radio(video.mode(), Mode::BetweenFrames),
            mode_radio(video.mode(), Mode::PreparingScanline)
        ],
        row![
            mode_radio(video.mode(), Mode::DrawingPixels),
            mode_radio(video.mode(), Mode::FinishingScanline),
        ],
        horizontal_rule(1),
        control::control(video.control())
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
