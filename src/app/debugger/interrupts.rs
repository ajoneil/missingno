use iced::{
    Element, Length,
    widget::{checkbox, column, row, text},
};

use crate::{
    app::{
        Message,
        core::sizes::{m, s, xs},
    },
    game_boy::GameBoy,
};

pub fn interrupts(game_boy: &GameBoy) -> Element<'static, Message> {
    column![
        checkbox(game_boy.cpu().interrupts_enabled()).label("Interrupts"),
        enabled(game_boy),
        requested(game_boy),
    ]
    .spacing(m())
    .into()
}

fn enabled(game_boy: &GameBoy) -> Element<'static, Message> {
    column![
        text("Enabled"),
        row![
            checkbox(
                game_boy
                    .interrupts()
                    .enabled(crate::game_boy::interrupts::Interrupt::Joypad)
            )
            .label("Joypad")
            .width(Length::FillPortion(3)),
            checkbox(
                game_boy
                    .interrupts()
                    .enabled(crate::game_boy::interrupts::Interrupt::Serial)
            )
            .label("Serial")
            .width(Length::FillPortion(2)),
            checkbox(
                game_boy
                    .interrupts()
                    .enabled(crate::game_boy::interrupts::Interrupt::Timer)
            )
            .label("Timer")
            .width(Length::FillPortion(2)),
        ],
        row![
            checkbox(
                game_boy
                    .interrupts()
                    .enabled(crate::game_boy::interrupts::Interrupt::VideoStatus)
            )
            .label("Video Status")
            .width(Length::FillPortion(3)),
            checkbox(
                game_boy
                    .interrupts()
                    .enabled(crate::game_boy::interrupts::Interrupt::VideoBetweenFrames)
            )
            .label("Between Frames")
            .width(Length::FillPortion(4)),
        ],
    ]
    .spacing(s())
    .into()
}

fn requested(game_boy: &GameBoy) -> Element<'static, Message> {
    column![
        text("Requested"),
        row![
            checkbox(
                game_boy
                    .interrupts()
                    .requested(crate::game_boy::interrupts::Interrupt::Joypad)
            )
            .label("Joypad")
            .width(Length::FillPortion(3)),
            checkbox(
                game_boy
                    .interrupts()
                    .requested(crate::game_boy::interrupts::Interrupt::Serial)
            )
            .label("Serial")
            .width(Length::FillPortion(2)),
            checkbox(
                game_boy
                    .interrupts()
                    .requested(crate::game_boy::interrupts::Interrupt::Timer)
            )
            .label("Timer")
            .width(Length::FillPortion(2)),
        ],
        row![
            checkbox(
                game_boy
                    .interrupts()
                    .requested(crate::game_boy::interrupts::Interrupt::VideoStatus)
            )
            .label("Video Status")
            .width(Length::FillPortion(3)),
            checkbox(
                game_boy
                    .interrupts()
                    .requested(crate::game_boy::interrupts::Interrupt::VideoBetweenFrames)
            )
            .label("Between Frames")
            .width(Length::FillPortion(4)),
        ],
    ]
    .spacing(xs())
    .into()
}
