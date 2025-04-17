use iced::{Element, Task, widget::container};

use crate::{
    app::{self, core::sizes::m},
    emulator::GameBoy,
};
use panes::DebuggerPanes;

mod audio;
mod breakpoints;
mod cpu;
mod instructions;
mod interrupts;
pub mod panes;
mod screen;
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
    panes: DebuggerPanes,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes: DebuggerPanes::new(),
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }

    pub fn panes(&self) -> &DebuggerPanes {
        &self.panes
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::Step => {
                if let Some(screen) = self.debugger.step() {
                    return Task::done(screen::Message::Update(screen).into());
                }
            }
            Message::StepOver => {
                if let Some(screen) = self.debugger.step_over() {
                    return Task::done(screen::Message::Update(screen).into());
                }
            }
            Message::StepFrame => {
                return Task::done(screen::Message::Update(self.debugger.step_frame()).into());
            }

            Message::Run => todo!(), //self.debugger.run(),

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
