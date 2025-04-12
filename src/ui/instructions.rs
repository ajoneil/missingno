use std::collections::HashSet;

use crate::{
    emulation::{Cartridge, Instruction},
    ui::Message,
};
use iced::{
    Element, Font, Length,
    alignment::Vertical,
    widget::{Column, button, row, text},
};

use super::debugger;

struct InstructionsIterator<'a> {
    address: u16,
    rom: &'a [u8],
}

impl<'a> Iterator for InstructionsIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.rom[self.address as usize];

        self.address += 1;
        // Skip over header as it's data and not opcodes
        if (0x104..0x14f).contains(&self.address) {
            self.address = 0x150;
        }

        Some(value)
    }
}

pub fn instructions<'a>(
    cartridge: &'a Cartridge,
    pc: u16,
    breakpoints: &HashSet<u16>,
) -> Element<'a, Message> {
    let mut iterator = InstructionsIterator {
        address: pc,
        rom: cartridge.rom(),
    };

    let mut instructions = Vec::new();

    for _ in 0..20 {
        let address = iterator.address;
        if let Some(decoded) = Instruction::decode(&mut iterator) {
            instructions.push(instruction(
                address,
                decoded,
                breakpoints.contains(&address),
            ));
        } else {
            break;
        }
    }

    Column::from_vec(instructions).into()
}

pub fn instruction(
    address: u16,
    instruction: Instruction,
    is_breakpoint: bool,
) -> Element<'static, Message> {
    row![
        breakpoint(address, is_breakpoint),
        text(format!("{:04x}", address)).font(Font::MONOSPACE),
        text(instruction.to_string()).font(Font::MONOSPACE)
    ]
    .align_y(Vertical::Center)
    .spacing(10)
    .into()
}

fn breakpoint(address: u16, breakpoint: bool) -> Element<'static, Message> {
    button(text(if breakpoint { "🔴" } else { "" }).font(Font::with_name("Noto Color Emoji")))
        .style(button::text)
        .width(Length::Fixed(20.0))
        .padding(3)
        .on_press(
            if breakpoint {
                debugger::Message::ClearBreakpoint(address)
            } else {
                debugger::Message::SetBreakpoint(address)
            }
            .into(),
        )
        .into()
}
