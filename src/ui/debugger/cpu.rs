use crate::{
    emulator::cpu::{
        Cpu,
        flags::Flags,
        registers::{Register8, Register16},
    },
    ui::Message,
};

use iced::{
    Alignment, Element, Font, Length,
    alignment::Vertical,
    widget::{checkbox, column, container, horizontal_rule, row, text, text_input},
};

pub fn cpu(cpu: &Cpu) -> Element<'_, Message> {
    column![
        container(checkbox("Halted", cpu.halted))
            .width(Length::Fill)
            .align_x(Alignment::End),
        row![
            text("Program Counter")
                .align_x(Alignment::End)
                .width(Length::FillPortion(2)),
            text_input("Program Counter", &format!("{:04x}", cpu.program_counter))
                .font(Font::MONOSPACE)
                .width(Length::FillPortion(1))
        ]
        .align_y(Vertical::Center)
        .spacing(5),
        row![
            text("Stack Pointer")
                .align_x(Alignment::End)
                .width(Length::FillPortion(2)),
            text_input("Stack Pointer", &format!("{:04x}", cpu.stack_pointer))
                .font(Font::MONOSPACE)
                .width(Length::FillPortion(1))
        ]
        .align_y(Vertical::Center)
        .spacing(5),
        horizontal_rule(1),
        container(flags(cpu.flags)),
        row![
            container(register8(cpu, Register8::A)).width(Length::FillPortion(1)),
            container("").width(Length::FillPortion(1)),
            container(register16(cpu, Register16::Af, Register16::Af.to_string()))
                .width(Length::FillPortion(2))
        ]
        .spacing(10),
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
        container(register8(cpu, reg1)).width(Length::FillPortion(1)),
        container(register8(cpu, reg2)).width(Length::FillPortion(1)),
        container(register16(cpu, pair, pair.to_string())).width(Length::FillPortion(2))
    ]
    .spacing(10)
    .into()
}

fn register8(cpu: &Cpu, register: Register8) -> Element<'static, Message> {
    row![
        text(register.to_string())
            .align_x(Alignment::End)
            .font(Font::MONOSPACE),
        text_input(
            &register.to_string(),
            &cpu.get_register8(register).to_string()
        )
        .font(Font::MONOSPACE)
    ]
    .align_y(Vertical::Center)
    .spacing(5)
    .into()
}

fn register16(cpu: &Cpu, register: Register16, label: String) -> Element<'_, Message> {
    row![
        text(label).align_x(Alignment::End).font(Font::MONOSPACE),
        text_input(
            &register.to_string(),
            &format!("{:04x}", &cpu.get_register16(register))
        )
        .font(Font::MONOSPACE)
    ]
    .align_y(Vertical::Center)
    .spacing(5)
    .into()
}

fn flags(flags: Flags) -> Element<'static, Message> {
    container(
        row![
            column![
                flag("zero", flags, Flags::ZERO),
                flag("carry", flags, Flags::CARRY)
            ],
            column![
                flag("negative", flags, Flags::NEGATIVE),
                flag("half-carry", flags, Flags::HALF_CARRY),
            ]
        ]
        .spacing(10),
    )
    .align_right(Length::Fill)
    .into()
}

fn flag(label: &str, flags: Flags, flag: Flags) -> Element<'_, Message> {
    container(checkbox(label, flags.contains(flag))).into()
}
