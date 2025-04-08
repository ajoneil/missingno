use crate::{
    emulation::{Cartridge, GameBoy},
    ui::{self, cpu::cpu, instructions::instructions},
};
use iced::{
    Element, Length, Task,
    widget::{button, column, container, row, text},
};

#[derive(Debug, Clone)]
pub enum Message {
    Step,
}

pub fn update(game_boy: &mut GameBoy, message: Message) -> Task<ui::Message> {
    match message {
        Message::Step => {
            game_boy.step();
            Task::none()
        }
    }
}

pub fn emulator(game_boy: &GameBoy) -> Element<'_, ui::Message> {
    row![
        container(instructions(game_boy.cartridge(), game_boy.cpu().pc)).width(Length::Fill),
        column![
            cartridge(game_boy.cartridge()),
            cpu(game_boy.cpu()),
            controls()
        ]
        .spacing(10)
    ]
    .height(Length::Fill)
    .spacing(20)
    .padding(10)
    .into()
}

fn cartridge(cartridge: &Cartridge) -> Element<'_, ui::Message> {
    text(cartridge.title()).into()
}

fn controls() -> Element<'static, ui::Message> {
    button("Step")
        .on_press(ui::Message::Emulator(Message::Step))
        .into()
}
