use std::{path::PathBuf, time::Duration};

use iced::{Element, Subscription, Task, time, widget::container};

use crate::{
    app::{self, core::sizes::m, emulator::Emulator},
    game_boy::{
        GameBoy,
        joypad::Button,
        recording::{Input, Recording},
        video::palette::PaletteChoice,
    },
};
use panes::DebuggerPanes;

mod audio;
mod breakpoints;
mod cpu;
mod instructions;
mod interrupts;
pub mod panes;
pub mod playback;
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

struct ActiveRecording {
    recording: Recording,
    path: PathBuf,
}

pub struct Debugger {
    debugger: crate::debugger::Debugger,
    panes: DebuggerPanes,
    running: bool,
    frame: u64,
    active_recording: Option<ActiveRecording>,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes: DebuggerPanes::new(),
            running: false,
            frame: 0,
            active_recording: None,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }

    pub fn game_boy_mut(&mut self) -> &mut GameBoy {
        self.debugger.game_boy_mut()
    }

    pub fn disable_debugger(self) -> Emulator {
        app::emulator::Emulator::new(self.debugger.game_boy_take())
    }

    pub fn panes(&self) -> &DebuggerPanes {
        &self.panes
    }

    fn screen_update_task(
        &self,
        screen: Option<crate::game_boy::video::screen::Screen>,
    ) -> Option<Task<app::Message>> {
        let screen = screen?;
        let video_enabled = self.debugger.game_boy().video().control().video_enabled();
        let sgb_data = self
            .debugger
            .game_boy()
            .sgb()
            .map(|sgb| sgb.render_data(video_enabled));
        Some(Task::done(screen::Message::Update(screen, sgb_data).into()))
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        let task = match message {
            Message::Step => {
                let screen = self.debugger.step();
                self.screen_update_task(screen)
            }
            Message::StepOver => {
                let screen = self.debugger.step_over();
                self.screen_update_task(screen)
            }
            Message::StepFrame => {
                self.frame += 1;
                let screen = self.debugger.step_frame();
                if screen.is_none() {
                    self.running = false;
                }
                self.screen_update_task(screen)
            }

            Message::SetBreakpoint(address) => {
                self.debugger.set_breakpoint(address);
                None
            }
            Message::ClearBreakpoint(address) => {
                self.debugger.clear_breakpoint(address);
                None
            }

            Message::Pane(message) => {
                self.panes.update(message, &mut self.debugger);
                None
            }
        };

        task.unwrap_or(Task::none())
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.panes.set_palette(palette);
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        container(self.panes.view(&self.debugger, self))
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
        self.frame = 0;
        self.active_recording = None;
    }

    pub fn press_button(&mut self, button: Button) {
        if let Some(active) = &mut self.active_recording {
            active.recording.record(self.frame, Input::Press(button));
        }
        self.debugger.game_boy_mut().press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        if let Some(active) = &mut self.active_recording {
            active.recording.record(self.frame, Input::Release(button));
        }
        self.debugger.game_boy_mut().release_button(button);
    }

    pub fn start_recording(&mut self, path: PathBuf) {
        self.debugger.reset();
        self.frame = 0;
        let game_boy = self.debugger.game_boy();
        let title = game_boy.cartridge().title().to_string();
        let checksum = game_boy.cartridge().global_checksum();
        self.active_recording = Some(ActiveRecording {
            recording: Recording::new(title, checksum),
            path,
        });
        self.running = true;
    }

    pub fn stop_recording(&mut self) -> Option<String> {
        let active = self.active_recording.take()?;
        active.recording.save(&active.path);
        active
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }

    pub fn is_recording(&self) -> bool {
        self.active_recording.is_some()
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }
}
