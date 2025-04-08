use crate::{
    emulation::{Cartridge, GameBoy},
    ui::{Message, cpu::cpu, instructions::instructions},
};
use iced::{
    Element, Length,
    widget::{column, container, row, text},
};

pub fn emulator(game_boy: &GameBoy) -> Element<'_, Message> {
    row![
        container(instructions(game_boy.cartridge(), game_boy.cpu().pc)).width(Length::Fill),
        column![cartridge(game_boy.cartridge()), cpu(game_boy.cpu())]
    ]
    .height(Length::Fill)
    .spacing(20)
    .padding(10)
    .into()
}

fn cartridge(cartridge: &Cartridge) -> Element<'_, Message> {
    text(cartridge.title()).into()
}
