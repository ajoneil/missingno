use crate::{
    emulation::{Cartridge, GameBoy},
    ui::{Message, cpu::cpu_view},
};
use iced::{
    Element,
    widget::{column, text},
};

pub fn emulator_view(game_boy: &GameBoy) -> Element<'_, Message> {
    column![
        cartridge_view(game_boy.cartridge()),
        cpu_view(game_boy.cpu())
    ]
    .into()
}

fn cartridge_view(cartridge: &Cartridge) -> Element<'_, Message> {
    text(cartridge.title()).into()
}
