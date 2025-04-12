use crate::{
    debugger::Debugger,
    emulation::Cartridge,
    ui::{self, cpu::cpu, instructions::instructions},
};
use iced::{
    Element, Length, Task,
    widget::{button, column, container, row, text},
};

#[derive(Debug, Clone)]
pub enum Message {
    Step,
    Run,
    SetBreakpoint(u16),
    ClearBreakpoint(u16),
}

impl Into<super::Message> for Message {
    fn into(self) -> super::Message {
        super::Message::Debugger(self)
    }
}

pub fn update(debugger: &mut Debugger, message: Message) -> Task<ui::Message> {
    match message {
        Message::Step => debugger.step(),
        Message::Run => debugger.run(),
        Message::SetBreakpoint(address) => debugger.set_breakpoint(address),
        Message::ClearBreakpoint(address) => debugger.clear_breakpoint(address),
    }

    Task::none()
}

pub fn debugger(debugger: &Debugger) -> Element<'_, ui::Message> {
    row![
        container(instructions(
            debugger.game_boy().cartridge(),
            debugger.game_boy().cpu().program_counter,
            debugger.breakpoints()
        ))
        .width(Length::FillPortion(2)),
        column![
            cartridge(debugger.game_boy().cartridge()),
            cpu(debugger.game_boy().cpu()),
            controls()
        ]
        .width(Length::FillPortion(1))
        .spacing(10)
    ]
    .height(Length::Fill)
    .spacing(20)
    .padding(10)
    .into()
}

fn cartridge(cartridge: &Cartridge) -> Element<'_, ui::Message> {
    text(cartridge.title()).into()
}

fn controls() -> Element<'static, ui::Message> {
    row![
        button("Step").on_press(Message::Step.into()),
        button("Run").on_press(Message::Run.into())
    ]
    .spacing(10)
    .into()
}
