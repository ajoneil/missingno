use std::time::Duration;

use iced::{Element, Subscription, Task, time, widget::container};

use crate::{
    app::{self, core::sizes::m, emulator::Emulator},
    emulator::{GameBoy, joypad::Button},
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
    running: bool,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes: DebuggerPanes::new(),
            running: false,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }

    pub fn disable_debugger(self) -> Emulator {
        app::emulator::Emulator::new(self.debugger.game_boy_take())
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
                if let Some(screen) = self.debugger.step_frame() {
                    return Task::done(screen::Message::Update(screen).into());
                } else {
                    self.running = false
                }
            }

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

    pub fn subscription(&self) -> Subscription<app::Message> {
        if self.running {
            Subscription::batch([
                time::every(Duration::from_micros(16740)).map(|_| Message::StepFrame.into())
            ])
        } else {
            Subscription::none()
        }
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn run(&mut self) {
        self.running = true;
    }

    pub fn pause(&mut self) {
        self.running = false;
    }

    pub fn reset(&mut self) {
        self.debugger.reset();
    }

    pub fn press_button(&mut self, button: Button) {
        self.debugger.game_boy_mut().press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.debugger.game_boy_mut().release_button(button);
    }
}
