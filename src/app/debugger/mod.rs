use std::time::Duration;

use iced::{
    Element, Event, Subscription, Task, event,
    keyboard::{self, Key, key},
    time,
    widget::container,
};

use crate::{
    app::{self, core::sizes::m},
    emulator::{GameBoy, joypad},
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

    PressButton(joypad::Button),
    ReleaseButton(joypad::Button),

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

            Message::Run => self.running = true,
            Message::Pause => self.running = false,
            Message::Reset => self.debugger.reset(),

            Message::PressButton(button) => self.debugger.game_boy_mut().press_button(button),
            Message::ReleaseButton(button) => self.debugger.game_boy_mut().release_button(button),

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
                time::every(Duration::from_micros(16740)).map(|_| Message::StepFrame.into()),
                event::listen_with(|event, _status, _id| match event {
                    Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                        if let Some(button) = Self::map_key(key) {
                            Some(Message::PressButton(button).into())
                        } else {
                            None
                        }
                    }
                    Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
                        if let Some(button) = Self::map_key(key) {
                            Some(Message::ReleaseButton(button).into())
                        } else {
                            None
                        }
                    }
                    _ => None,
                }),
            ])
        } else {
            Subscription::none()
        }
    }

    fn map_key(key: Key) -> Option<joypad::Button> {
        Some(match key.as_ref() {
            Key::Named(key::Named::ArrowUp) => {
                joypad::Button::DirectionalPad(joypad::DirectionalPad::Up)
            }
            Key::Named(key::Named::ArrowDown) => {
                joypad::Button::DirectionalPad(joypad::DirectionalPad::Down)
            }
            Key::Named(key::Named::ArrowLeft) => {
                joypad::Button::DirectionalPad(joypad::DirectionalPad::Left)
            }
            Key::Named(key::Named::ArrowRight) => {
                joypad::Button::DirectionalPad(joypad::DirectionalPad::Right)
            }
            Key::Named(key::Named::Enter) => joypad::Button::Start,
            Key::Named(key::Named::Shift) => joypad::Button::Select,
            Key::Character("x") => joypad::Button::A,
            Key::Character("z") => joypad::Button::B,

            _ => return None,
        })

        // let up: joypad::Button = joypad::Button::DirectionalPad(joypad::DirectionalPad::Up);
        // let m = match key {
        //     Key::Named(key::Named::ArrowUp) => {
        //         joypad::Button::DirectionalPad(joypad::DirectionalPad::Up)
        //     }
        //     _ => {}
        // };

        // None
        // Some(match key {
        //     Key::Named(key::Named::ArrowUp) => {
        //         joypad::Button::DirectionalPad(joypad::DirectionalPad::Up)
        //     }

        //     // key::Named::ArrowDown => Button::DirectionalPad(DirectionalPad::Down),
        //     // key::Named::ArrowLeft => Button::DirectionalPad(DirectionalPad::Left),
        //     // key::Named::ArrowRight => Button::DirectionalPad(DirectionalPad::Right),
        //     _ => return None,
        // })
    }

    pub fn running(&self) -> bool {
        self.running
    }
}
