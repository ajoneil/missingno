use crate::{
    emulation::{Cpu, CpuFlags},
    ui::Message,
};
use iced::{
    Alignment, Element, Length, Padding,
    alignment::Vertical,
    widget::{checkbox, column, container, row, text_input},
};

pub fn cpu_view(cpu: &Cpu) -> Element<'_, Message> {
    column![
        row![register16("pc", cpu.pc), register16("sp", cpu.sp)],
        row![
            container(checkbox("halted", cpu.halted))
                .align_left(Length::Fixed(100.0))
                .padding(5.0),
            container(checkbox("ime", cpu.ime))
                .align_left(Length::Fixed(100.0))
                .padding(5.0)
        ],
        row![
            register8("a", cpu.a),
            register8("b", cpu.b),
            register8("c", cpu.c),
            register8("d", cpu.d)
        ],
        row![
            register8("e", cpu.e),
            register8("h", cpu.h),
            register8("l", cpu.l)
        ],
        row![
            register8("f", cpu.f.bits()),
            flag("z", cpu.f, CpuFlags::Z),
            flag("n", cpu.f, CpuFlags::N),
            flag("h", cpu.f, CpuFlags::H),
            flag("c", cpu.f, CpuFlags::C)
        ]
        .align_y(Vertical::Center),
    ]
    .spacing(3.0)
    .into()
}

fn register8(label: &str, value: u8) -> Element<'_, Message> {
    row![
        container(label)
            .align_right(Length::Fixed(20.0))
            .padding(5.0),
        text_input(label, &format!("{:02x}", value))
            .width(Length::Fixed(40.0))
            .align_x(Alignment::Start)
    ]
    .align_y(Vertical::Center)
    .into()
}

fn register16(label: &str, value: u16) -> Element<'_, Message> {
    row![
        container(label)
            .align_right(Length::Fixed(40.0))
            .padding(Padding::from([5.0, 10.0])),
        text_input(label, &format!("{:04x}", value))
            .width(Length::Fixed(80.0))
            .align_x(Alignment::Start)
    ]
    .align_y(Vertical::Center)
    .into()
}

fn flag(label: &str, flags: CpuFlags, flag: CpuFlags) -> Element<'_, Message> {
    container(checkbox(label, flags.contains(flag)))
        .align_right(Length::Fixed(45.0))
        .into()
}
