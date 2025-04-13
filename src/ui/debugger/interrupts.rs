use crate::{emulator::GameBoy, ui::Message};

use iced::{
    Element, Length,
    widget::{checkbox, column, row, text},
};

pub fn interrupts(game_boy: &GameBoy) -> Element<'_, Message> {
    column![
        checkbox(
            "Interrupts Master Enable",
            game_boy.cpu().interrupts_enabled()
        ),
        enabled(game_boy),
        requested(game_boy),
    ]
    .spacing(10)
    .into()
}

fn enabled(game_boy: &GameBoy) -> Element<'_, Message> {
    column![
        text("Enabled"),
        row![
            checkbox(
                "Joypad",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::Joypad)
            )
            .width(Length::Fill),
            checkbox(
                "Serial",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::Serial)
            )
            .width(Length::Fill),
            checkbox(
                "Timer",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::Timer)
            )
            .width(Length::Fill),
        ],
        row![
            checkbox(
                "Video Status",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::VideoStatus)
            )
            .width(Length::FillPortion(2)),
            checkbox(
                "Between Frames",
                game_boy
                    .interrupts()
                    .enabled(crate::emulator::interrupts::Interrupt::VideoBetweenFrames)
            )
            .width(Length::FillPortion(2)),
        ],
    ]
    .spacing(3)
    .into()
}

fn requested(game_boy: &GameBoy) -> Element<'_, Message> {
    column![
        text("Requested"),
        row![
            checkbox(
                "Joypad",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::Joypad)
            )
            .width(Length::Fill),
            checkbox(
                "Serial",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::Serial)
            )
            .width(Length::Fill),
            checkbox(
                "Timer",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::Timer)
            )
            .width(Length::Fill),
        ],
        row![
            checkbox(
                "Video Status",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::VideoStatus)
            )
            .width(Length::FillPortion(2)),
            checkbox(
                "Between Frames",
                game_boy
                    .interrupts()
                    .requested(crate::emulator::interrupts::Interrupt::VideoBetweenFrames)
            )
            .width(Length::FillPortion(2)),
        ],
    ]
    .spacing(3)
    .into()
}
