use iced::{Element, Task, widget::container};

use crate::{
    app::{self, core::sizes::m},
    emulator::GameBoy,
};
use panes::Panes;

mod audio;
mod breakpoints;
mod cpu;
mod instructions;
mod interrupts;
pub mod panes;
mod video;

#[derive(Debug, Clone)]
pub enum Message {
    Step,
    StepOver,
    StepFrame,
    Run,
    Pause,
    Reset,

    SetBreakpoint(u16),
    ClearBreakpoint(u16),

    Pane(panes::Message),
}

impl Into<super::Message> for Message {
    fn into(self) -> super::Message {
        super::Message::Debugger(self)
    }
}

pub struct Debugger {
    debugger: crate::debugger::Debugger,
    panes: Panes,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes: Panes::new(),
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }

    pub fn panes(&self) -> &Panes {
        &self.panes
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::Step => self.debugger.step(),
            Message::StepOver => self.debugger.step_over(),
            Message::StepFrame => self.debugger.step_frame(),
            Message::Run => self.debugger.run(),
            Message::Pause => todo!(),
            Message::Reset => self.debugger.reset(),

            Message::SetBreakpoint(address) => self.debugger.set_breakpoint(address),
            Message::ClearBreakpoint(address) => self.debugger.clear_breakpoint(address),

            Message::Pane(message) => self.panes.update(message, &mut self.debugger),
        }

        Task::none()
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        container(self.panes.view(&self.debugger))
            .padding(m())
            .into()
    }
}
