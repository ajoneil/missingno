use crate::{
    emulation::{Cartridge, Instruction},
    ui::Message,
};
use iced::{
    Element, Font,
    widget::{Column, row, text},
};

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

pub fn instructions(cartridge: &Cartridge, pc: u16) -> Element<'_, Message> {
    let mut iterator = InstructionsIterator {
        address: pc,
        rom: cartridge.rom(),
    };

    let mut instructions = Vec::new();

    for _ in 0..20 {
        let address = iterator.address;
        if let Some(decoded) = Instruction::decode(&mut iterator) {
            instructions.push(instruction(address, decoded));
        } else {
            break;
        }
    }

    Column::from_vec(instructions).into()
}

pub fn instruction(address: u16, instruction: Instruction) -> Element<'static, Message> {
    row![
        text(format!("{:04x}", address)).font(Font::MONOSPACE),
        text(instruction.to_string()).font(Font::MONOSPACE)
    ]
    .spacing(10)
    .into()
}
