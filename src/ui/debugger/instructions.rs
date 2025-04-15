use crate::{
    debugger::instructions::InstructionsIterator,
    emulator::{cartridge::Cartridge, cpu::instructions::Instruction},
    ui::{
        Message,
        debugger::{
            self,
            panes::{pane, title_bar},
        },
        styles::fonts,
    },
};
use iced::{
    Element, Length,
    alignment::Vertical,
    widget::{Column, button, pane_grid, row, text},
};
use std::collections::HashSet;

pub fn instructions_pane<'a>(
    cartridge: &'a Cartridge,
    pc: u16,
    breakpoints: &'a HashSet<u16>,
) -> pane_grid::Content<'a, Message> {
    pane(
        title_bar("Instructions"),
        instructions(cartridge, pc, breakpoints),
    )
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

    for _ in 0..50 {
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
        text(format!("{:04x}", address)).font(fonts::MONOSPACE),
        text(instruction.to_string()).font(fonts::MONOSPACE)
    ]
    .align_y(Vertical::Center)
    .spacing(10)
    .into()
}

fn breakpoint(address: u16, breakpoint: bool) -> Element<'static, Message> {
    button(text(if breakpoint { "🔴" } else { "" }).font(fonts::EMOJI))
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
