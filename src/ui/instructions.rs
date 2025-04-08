use crate::{
    emulation::{Cartridge, Instruction},
    ui::Message,
};
use iced::{Element, widget::text};

pub fn instructions(cartridge: &Cartridge, pc: u16) -> Element<'_, Message> {
    instruction(Instruction::decode(
        cartridge.rom()[pc.into()..].iter().copied(),
    ))
}

pub fn instruction(instruction: Instruction) -> Element<'static, Message> {
    text(instruction.to_string()).into()
}
