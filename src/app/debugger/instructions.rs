use std::collections::BTreeSet;

use iced::{
    Element, Length,
    alignment::Vertical,
    widget::{Column, button, pane_grid, row, text},
};

use crate::{
    app::{
        Message,
        core::{
            emoji, fonts,
            sizes::{s, xs},
        },
        debugger::{
            self,
            panes::{AvailablePanes, pane, title_bar},
        },
    },
    debugger::instructions::InstructionsIterator,
    emulator::{MemoryMapped, cpu::instructions::Instruction},
};

pub fn instructions_pane<'a>(
    memory: &MemoryMapped,
    pc: u16,
    breakpoints: &BTreeSet<u16>,
) -> pane_grid::Content<'a, Message> {
    pane(
        title_bar("Instructions", Some(AvailablePanes::Instructions)),
        instructions(memory, pc, breakpoints),
    )
}

pub fn instructions<'a>(
    memory: &MemoryMapped,
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
        text(format!("{:04x}", address)).font(fonts::monospace()),
        text(instruction.to_string()).font(fonts::monospace())
    ]
    .align_y(Vertical::Center)
    .spacing(s())
    .into()
}

fn breakpoint(address: u16, breakpoint: bool) -> Element<'static, Message> {
    button(emoji::m(if breakpoint { "🔴" } else { "" }))
        .style(button::text)
        .width(Length::Fixed(20.0))
        .padding(xs())
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
