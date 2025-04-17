use iced::{
    Alignment, Element,
    Length::{self, Fill},
    alignment::Vertical,
    widget::{
        button, checkbox, column, container, horizontal_rule, pane_grid, row, text, text_input,
    },
};

use crate::{
    app::{
        self,
        core::{
            fonts,
            sizes::{m, s},
        },
        debugger::{
            interrupts::interrupts,
            panes::{DebuggerPane, checkbox_title_bar, pane},
        },
    },
    debugger::Debugger,
    emulator::cpu::{
        Cpu,
        flags::Flags,
        registers::{Register8, Register16},
    },
};

pub struct CpuPane;

impl CpuPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(&self, debugger: &Debugger) -> pane_grid::Content<'_, app::Message> {
        pane(
            checkbox_title_bar(
                "CPU",
                !debugger.game_boy().cpu().halted,
                Some(DebuggerPane::Cpu),
            ),
            column![
                self.cpu(debugger.game_boy().cpu()),
                horizontal_rule(1),
                interrupts(debugger.game_boy()),
            ]
            .spacing(s())
            .into(),
        )
    }

    fn cpu(&self, cpu: &Cpu) -> Element<'_, app::Message> {
        column![
            row![
                text("Program Counter")
                    .align_x(Alignment::End)
                    .width(Length::FillPortion(2)),
                text_input("Program Counter", &format!("{:04x}", cpu.program_counter))
                    .font(fonts::monospace())
                    .width(Length::FillPortion(1))
            ]
            .align_y(Vertical::Center)
            .spacing(s()),
            row![
                text("Stack Pointer")
                    .align_x(Alignment::End)
                    .width(Length::FillPortion(2)),
                text_input("Stack Pointer", &format!("{:04x}", cpu.stack_pointer))
                    .font(fonts::monospace())
                    .width(Length::FillPortion(1))
            ]
            .align_y(Vertical::Center)
            .spacing(s()),
            self.controls(),
            horizontal_rule(1),
            container(self.flags(cpu.flags)),
            row![
                container(self.register8(cpu, Register8::A)).width(Length::FillPortion(1)),
                container("").width(Length::FillPortion(1)),
                container(self.register16(cpu, Register16::Af, Register16::Af.to_string()))
                    .width(Length::FillPortion(2))
            ]
            .spacing(m()),
            row![self.register_pair(cpu, Register8::B, Register8::C, Register16::Bc)],
            row![self.register_pair(cpu, Register8::D, Register8::E, Register16::De)],
            row![self.register_pair(cpu, Register8::H, Register8::L, Register16::Hl)],
        ]
        .spacing(s())
        .into()
    }

    fn register_pair(
        &self,
        cpu: &Cpu,
        reg1: Register8,
        reg2: Register8,
        pair: Register16,
    ) -> Element<'_, app::Message> {
        row![
            container(self.register8(cpu, reg1)).width(Length::FillPortion(1)),
            container(self.register8(cpu, reg2)).width(Length::FillPortion(1)),
            container(self.register16(cpu, pair, pair.to_string())).width(Length::FillPortion(2))
        ]
        .spacing(m())
        .into()
    }

    fn register8(&self, cpu: &Cpu, register: Register8) -> Element<'_, app::Message> {
        row![
            text(register.to_string())
                .align_x(Alignment::End)
                .font(fonts::monospace()),
            text_input(
                &register.to_string(),
                &cpu.get_register8(register).to_string()
            )
            .font(fonts::monospace())
        ]
        .align_y(Vertical::Center)
        .spacing(s())
        .into()
    }

    fn register16(
        &self,
        cpu: &Cpu,
        register: Register16,
        label: String,
    ) -> Element<'_, app::Message> {
        row![
            text(label).align_x(Alignment::End).font(fonts::monospace()),
            text_input(
                &register.to_string(),
                &format!("{:04x}", &cpu.get_register16(register))
            )
            .font(fonts::monospace())
        ]
        .align_y(Vertical::Center)
        .spacing(s())
        .into()
    }

    fn flags(&self, flags: Flags) -> Element<'_, app::Message> {
        container(
            row![
                column![
                    self.flag("zero", flags, Flags::ZERO),
                    self.flag("carry", flags, Flags::CARRY)
                ],
                column![
                    self.flag("negative", flags, Flags::NEGATIVE),
                    self.flag("half-carry", flags, Flags::HALF_CARRY),
                ]
            ]
            .spacing(s()),
        )
        .align_right(Length::Fill)
        .into()
    }

    fn flag(&self, label: &str, flags: Flags, flag: Flags) -> Element<'_, app::Message> {
        container(checkbox(label, flags.contains(flag))).into()
    }

    fn controls(&self) -> Element<'_, app::Message> {
        container(
            row![
                button("Step").on_press(super::Message::Step.into()),
                button("Step Over").on_press(super::Message::StepOver.into()),
            ]
            .spacing(s())
            .wrap(),
        )
        .align_right(Fill)
        .into()
    }
}
