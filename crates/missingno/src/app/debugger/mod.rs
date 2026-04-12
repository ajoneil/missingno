use std::time::Duration;

use iced::{Element, Subscription, Task, time, widget::row};

use crate::app::{
    self,
    ui::sizes::s,
    emulator::Emulator,
    screen::{GameBoyScreen, ScreenView, SgbScreen},
};
use missingno_gb::{
    GameBoy,
    joypad::Button,
    ppu::types::palette::{Palette, PaletteChoice},
    sgb::MaskMode,
};

use panes::DebuggerPanes;
use sidebar::Sidebar;

mod audio;
mod instructions;
mod interrupts;
pub mod panes;
mod ppu;
mod screen;
mod sidebar;

#[derive(Debug, Clone)]
pub enum Message {
    Step,
    StepOver,
    StepFrame,
    CaptureFrame,
    CaptureFrameTo(std::path::PathBuf),

    SetBreakpoint(u16),
    ClearBreakpoint(u16),

    Sidebar(sidebar::Message),
    Pane(panes::Message),
}

impl Into<super::Message> for Message {
    fn into(self) -> super::Message {
        super::Message::Debugger(self)
    }
}

pub struct Debugger {
    debugger: missingno_gb::debugger::Debugger,
    sidebar: Sidebar,
    panes: DebuggerPanes,
    running: bool,
    frame: u64,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            debugger: missingno_gb::debugger::Debugger::new(game_boy),
            sidebar: Sidebar::new(),
            panes: DebuggerPanes::new(),
            running: false,
            frame: 0,
        }
    }

    pub fn from_emulator(game_boy: GameBoy, screen_view: ScreenView) -> Self {
        Self {
            debugger: missingno_gb::debugger::Debugger::new(game_boy),
            sidebar: Sidebar::new(),
            panes: DebuggerPanes::with_screen(screen_view),
            running: false,
            frame: 0,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }

    pub fn game_boy_mut(&mut self) -> &mut GameBoy {
        self.debugger.game_boy_mut()
    }

    pub fn disable_debugger(self, use_sgb_colors: bool) -> Emulator {
        let screen_view = self.panes.take_screen_view();
        Emulator::from_debugger(self.debugger.game_boy_take(), screen_view, use_sgb_colors)
    }

    fn screen_update_task(
        &self,
        screen: Option<missingno_gb::ppu::screen::Screen>,
    ) -> Task<app::Message> {
        let video_enabled = self.debugger.game_boy().ppu().control().video_enabled();
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
                let screen = self.debugger.step_frame();
                if screen.is_none() {
                    self.running = false;
                }
                self.screen_update_task(screen)
            }
            Message::CaptureFrame => {
                let title = self
                    .debugger
                    .game_boy()
                    .cartridge()
                    .title()
                    .to_lowercase()
                    .replace(' ', "_");
                let default_name = format!("{title}_frame{}.gbtrace", self.frame);

                let dialog = rfd::AsyncFileDialog::new()
                    .set_file_name(&default_name)
                    .add_filter("gbtrace", &["gbtrace"]);

                return Task::perform(dialog.save_file(), |handle| match handle {
                    Some(h) => Message::CaptureFrameTo(h.path().to_path_buf()).into(),
                    None => app::Message::None,
                });
            }
            Message::CaptureFrameTo(path) => match self.debugger.capture_frame(&path) {
                Ok(screen) => {
                    self.frame += 1;
                    self.screen_update_task(Some(screen))
                }
                Err(_) => Task::none(),
            },

            Message::SetBreakpoint(address) => {
                self.debugger.set_breakpoint(address);
                Task::none()
            }
            Message::ClearBreakpoint(address) => {
                self.debugger.clear_breakpoint(address);
                Task::none()
            }

            Message::Sidebar(message) => {
                self.sidebar.update(&message, &mut self.debugger);
                Task::none()
            }

            Message::Pane(message) => {
                self.panes.update(message);
                Task::none()
            }
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.panes.set_palette(palette);
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        let pal = self.display_palette();
        row![
            self.sidebar.view(&self.debugger, pal),
            self.panes.view(&self.debugger, pal),
            self.panes.icon_rail(),
        ]
        .spacing(s())
        .padding(s())
        .into()
    }

    fn display_palette(&self) -> &Palette {
        if self.debugger.game_boy().sgb().is_some() {
            &Palette::CLASSIC
        } else {
            self.panes.palette()
        }
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
    }

    pub fn press_button(&mut self, button: Button) {
        self.debugger.game_boy_mut().press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.debugger.game_boy_mut().release_button(button);
    }
}
