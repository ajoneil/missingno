use crate::{
    debugger::instructions::InstructionsIterator,
    emulator::{MemoryMapped, cpu::instructions::Instruction},
    ui::{
        Message,
        debugger::{
            self,
            panes::{pane, title_bar},
        },
        styles::{fonts, spacing},
    },
};
use iced::{
    Element, Length,
    alignment::Vertical,
    widget::{Column, button, pane_grid, row, text},
};
use std::collections::BTreeSet;

pub fn instructions_pane<'a>(
    memory: &'a MemoryMapped,
    pc: u16,
    breakpoints: &'a BTreeSet<u16>,
) -> pane_grid::Content<'a, Message> {
    pane(
        title_bar("Instructions"),
        instructions(memory, pc, breakpoints),
    )
}

pub fn instructions<'a>(
    memory: &'a MemoryMapped,
    pc: u16,
    breakpoints: &BTreeSet<u16>,
) -> Element<'a, Message> {
    let mut iterator = InstructionsIterator::new(pc, memory);

    let mut instructions = Vec::new();

    for _ in 0..50 {
        if let Some(address) = iterator.address {
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
    .spacing(spacing::s())
    .into()
}

fn breakpoint(address: u16, breakpoint: bool) -> Element<'static, Message> {
    button(text(if breakpoint { "🔴" } else { "" }).font(fonts::EMOJI))
        .style(button::text)
        .width(Length::Fixed(20.0))
        .padding(spacing::xs())
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
