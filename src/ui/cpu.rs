use crate::{
    emulation::{Cpu, CpuFlags},
    ui::Message,
};
use iced::{
    Element, Font, Length, Padding,
    alignment::Vertical,
    widget::{checkbox, column, container, horizontal_rule, row, text, text_input},
};

pub fn cpu(cpu: &Cpu) -> Element<'_, Message> {
    column![
        register16("Program Counter", cpu.program_counter),
        register16("Stack Pointer", cpu.stack_pointer),
        container(column![
            checkbox("Halted", cpu.halted),
            checkbox("Interrupt master enable", cpu.interrupt_master_enable),
        ])
        .align_right(Length::Fill),
        horizontal_rule(1),
        row![
            register8("a", cpu.a),
            register8("b", cpu.b),
            register8("c", cpu.c),
            column![].width(Length::Fill),
        ],
        row![
            register8("d", cpu.d),
            register8("e", cpu.e),
            register8("h", cpu.h),
            register8("l", cpu.l)
        ],
        horizontal_rule(1),
        row![
            column![].width(Length::Fill),
            column![
                text("Flags"),
                text_input("Flags", &format!("{:02x}", cpu.flags)).font(Font::MONOSPACE),
            ]
            .spacing(5),
            column![
                flag("zero", cpu.flags, CpuFlags::ZERO),
                flag("negative", cpu.flags, CpuFlags::NEGATIVE),
                flag("half-carry", cpu.flags, CpuFlags::HALF_CARRY),
                flag("carry", cpu.flags, CpuFlags::CARRY)
            ]
        ]
        .spacing(10),
    ]
    .spacing(5)
    .into()
}

fn register8(label: &str, value: u8) -> Element<'_, Message> {
    row![
        container(label).align_right(Length::Fill),
        text_input(label, &format!("{:02x}", value)).font(Font::MONOSPACE)
    ]
    .align_y(Vertical::Center)
    .spacing(5)
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

fn flag(label: &str, flags: CpuFlags, flag: CpuFlags) -> Element<'_, Message> {
    container(checkbox(label, flags.contains(flag))).into()
}
