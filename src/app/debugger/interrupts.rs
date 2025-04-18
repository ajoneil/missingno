use iced::{
    Element, Length,
    widget::{checkbox, column, row, text},
};

use crate::{
    app::{
        Message,
        core::sizes::{m, s, xs},
    },
    emulator::GameBoy,
};

pub fn interrupts(game_boy: &GameBoy) -> Element<'static, Message> {
    column![
        checkbox("Interrupts", game_boy.cpu().interrupts_enabled()),
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
                "Joypad",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::Joypad)
            )
            .width(Length::FillPortion(3)),
            checkbox(
                "Serial",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::Serial)
            )
            .width(Length::FillPortion(2)),
            checkbox(
                "Timer",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::Timer)
            )
            .width(Length::FillPortion(2)),
        ],
        row![
            checkbox(
                "Video Status",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::VideoStatus)
            )
            .width(Length::FillPortion(3)),
            checkbox(
                "Between Frames",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::VideoBetweenFrames)
            )
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
                "Joypad",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::Joypad)
            )
            .width(Length::FillPortion(3)),
            checkbox(
                "Serial",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::Serial)
            )
            .width(Length::FillPortion(2)),
            checkbox(
                "Timer",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::Timer)
            )
            .width(Length::FillPortion(2)),
        ],
        row![
            checkbox(
                "Video Status",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::VideoStatus)
            )
            .width(Length::FillPortion(3)),
            checkbox(
                "Between Frames",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::VideoBetweenFrames)
            )
            .width(Length::FillPortion(4)),
        ],
    ]
    .spacing(xs())
    .into()
}
