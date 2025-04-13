use crate::{
    emulator::cpu::{self, Cpu, Register8, Register16},
    ui::Message,
};

use iced::{
    Alignment, Element, Font, Length, Padding,
    alignment::Vertical,
    widget::{checkbox, column, container, horizontal_rule, row, text, text_input},
};

pub fn cpu(cpu: &Cpu) -> Element<'_, Message> {
    column![
        register16("Program Counter", cpu.program_counter),
        register16("Stack Pointer", cpu.stack_pointer),
        container(column![checkbox("Halted", cpu.halted),]).align_right(Length::Fill),
        horizontal_rule(1),
        row![register_pair(
            cpu,
            Register8::A,
            Register8::F,
            Register16::Af
        )],
        row![register_pair(
            cpu,
            Register8::B,
            Register8::C,
            Register16::Bc
        )],
        row![register_pair(
            cpu,
            Register8::D,
            Register8::E,
            Register16::De
        )],
        row![register_pair(
            cpu,
            Register8::H,
            Register8::L,
            Register16::Hl
        )],
        horizontal_rule(1),
        row![
            column![
                flag("zero", cpu.flags, cpu::Flags::ZERO),
                flag("carry", cpu.flags, cpu::Flags::CARRY)
            ],
            column![
                flag("negative", cpu.flags, cpu::Flags::NEGATIVE),
                flag("half-carry", cpu.flags, cpu::Flags::HALF_CARRY),
            ]
        ]
        .spacing(10),
    ]
    .spacing(5)
    .into()
}

fn register_pair(
    cpu: &Cpu,
    reg1: Register8,
    reg2: Register8,
    pair: Register16,
) -> Element<'_, Message> {
    row![
        row![
            text(reg1.to_string())
                .align_x(Alignment::End)
                .font(Font::MONOSPACE),
            text_input(&reg1.to_string(), &cpu.get_register8(reg1).to_string())
                .font(Font::MONOSPACE)
        ]
        .align_y(Vertical::Center)
        .width(Length::FillPortion(1))
        .spacing(5),
        row![
            text(reg2.to_string())
                .align_x(Alignment::End)
                .font(Font::MONOSPACE),
            text_input(&reg2.to_string(), &cpu.get_register8(reg2).to_string())
                .font(Font::MONOSPACE)
        ]
        .align_y(Vertical::Center)
        .width(Length::FillPortion(1))
        .spacing(5),
        row![
            text(pair.to_string())
                .align_x(Alignment::End)
                .font(Font::MONOSPACE),
            text_input(
                &pair.to_string(),
                &format!("{:04x}", &cpu.get_register16(pair))
            )
            .font(Font::MONOSPACE)
        ]
        .align_y(Vertical::Center)
        .width(Length::FillPortion(2))
        .spacing(5)
    ]
    .spacing(10)
    .into()
}

fn register16(label: &str, value: u16) -> Element<'_, Message> {
    row![
        container(label)
            .align_right(Length::Fill)
            .padding(Padding::from([5.0, 10.0])),
        text_input(label, &format!("{:04x}", value)).font(Font::MONOSPACE)
    ]
    .align_y(Vertical::Center)
    .into()
}

fn flag(label: &str, flags: cpu::Flags, flag: cpu::Flags) -> Element<'_, Message> {
    container(checkbox(label, flags.contains(flag))).into()
}
