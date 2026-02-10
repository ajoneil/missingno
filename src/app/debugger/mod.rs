use std::{path::PathBuf, time::Duration};

use iced::{Element, Subscription, Task, time, widget::container};

use crate::{
    app::{
        self,
        core::sizes::m,
        emulator::Emulator,
        screen::{GameBoyScreen, ScreenView, SgbScreen},
    },
    game_boy::{
        GameBoy,
        joypad::Button,
        recording::{Input, Recording},
        sgb::MaskMode,
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

struct ActivePlayback {
    recording: Recording,
    cursor: usize,
}

pub struct Debugger {
    debugger: crate::debugger::Debugger,
    panes: DebuggerPanes,
    running: bool,
    frame: u64,
    active_recording: Option<ActiveRecording>,
    active_playback: Option<ActivePlayback>,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes: DebuggerPanes::new(),
            running: false,
            frame: 0,
            active_recording: None,
            active_playback: None,
        }
    }

    pub fn from_emulator(game_boy: GameBoy, screen_view: ScreenView) -> Self {
        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes: DebuggerPanes::with_screen(screen_view),
            running: false,
            frame: 0,
            active_recording: None,
            active_playback: None,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }

    pub fn game_boy_mut(&mut self) -> &mut GameBoy {
        self.debugger.game_boy_mut()
    }

    pub fn disable_debugger(self) -> Emulator {
        let screen_view = self.panes.take_screen_view();
        Emulator::from_debugger(self.debugger.game_boy_take(), screen_view)
    }

    pub fn panes(&self) -> &DebuggerPanes {
        &self.panes
    }

    fn screen_update_task(
        &self,
        screen: Option<crate::game_boy::video::screen::Screen>,
    ) -> Task<app::Message> {
        let video_enabled = self.debugger.game_boy().video().control().video_enabled();
        let display = if let Some(sgb) = self.debugger.game_boy().sgb() {
            let render_data = sgb.render_data(video_enabled);
            if sgb.mask_mode == MaskMode::Freeze {
                SgbScreen::Freeze(render_data).into()
            } else if let Some(screen) = screen {
                SgbScreen::Display(screen, render_data).into()
            } else {
                return Task::none();
            }
        } else if !video_enabled {
            GameBoyScreen::Off.into()
        } else if let Some(screen) = screen {
            GameBoyScreen::Display(screen).into()
        } else {
            return Task::none();
        };
        Task::done(screen::Message::Update(display).into())
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
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
                self.apply_playback_events();
                let screen = self.debugger.step_frame();
                if screen.is_none() {
                    self.running = false;
                }
                self.screen_update_task(screen)
            }

            Message::SetBreakpoint(address) => {
                self.debugger.set_breakpoint(address);
                Task::none()
            }
            Message::ClearBreakpoint(address) => {
                self.debugger.clear_breakpoint(address);
                Task::none()
            }

            Message::Pane(message) => {
                let scan = matches!(
                    message,
                    panes::Message::ShowPane(panes::DebuggerPane::Playback)
                );
                self.panes.update(message, &mut self.debugger);
                if scan {
                    Task::done(app::Message::ScanRecordings)
                } else {
                    Task::none()
                }
            }
        }
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
        self.active_playback = None;
    }

    pub fn press_button(&mut self, button: Button) {
        if self.active_playback.is_some() {
            return;
        }
        if let Some(active) = &mut self.active_recording {
            active.recording.record(self.frame, Input::Press(button));
        }
        self.debugger.game_boy_mut().press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        if self.active_playback.is_some() {
            return;
        }
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

    pub fn stop_recording(&mut self) -> Option<(PathBuf, u64)> {
        let active = self.active_recording.take()?;
        let last_frame = active
            .recording
            .events()
            .last()
            .map(|e| e.frame())
            .unwrap_or(0);
        active.recording.save(&active.path);
        Some((active.path, last_frame))
    }

    pub fn is_recording(&self) -> bool {
        self.active_recording.is_some()
    }

    pub fn start_playback(&mut self, recording: Recording) {
        self.debugger.reset();
        self.frame = 0;
        self.active_recording = None;
        self.active_playback = Some(ActivePlayback {
            recording,
            cursor: 0,
        });
        self.running = true;
    }

    pub fn is_playing(&self) -> bool {
        self.active_playback.is_some()
    }

    pub fn playback_total_frames(&self) -> Option<u64> {
        self.active_playback
            .as_ref()
            .and_then(|p| p.recording.events().last().map(|e| e.frame()))
    }

    fn apply_playback_events(&mut self) {
        let Some(playback) = &mut self.active_playback else {
            return;
        };

        let events = playback.recording.events();
        while playback.cursor < events.len() && events[playback.cursor].frame() == self.frame {
            match events[playback.cursor].input() {
                Input::Press(button) => self.debugger.game_boy_mut().press_button(*button),
                Input::Release(button) => self.debugger.game_boy_mut().release_button(*button),
            }
            playback.cursor += 1;
        }

        if playback.cursor >= events.len() {
            self.active_playback = None;
            self.running = false;
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }
}
