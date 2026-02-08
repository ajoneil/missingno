use std::{fs, path::PathBuf};

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, Task, Theme, event,
    widget::{column, container, svg},
    window,
};
use replace_with::replace_with_or_abort;

use crate::game_boy::{
    GameBoy,
    cartridge::Cartridge,
    joypad::{self, Button},
};
use action_bar::ActionBar;
use audio_output::AudioOutput;
use core::{
    fonts, horizontal_rule,
    icons::{self, Icon},
    sizes::l,
    text,
};

mod action_bar;
mod audio_output;
mod controls;
mod core;
mod debugger;
mod emulator;
mod load;
mod screen;
mod texture_renderer;

pub fn run(rom_path: Option<PathBuf>, debugger: bool) -> iced::Result {
    iced::application(
        move || App::new(rom_path.clone(), debugger),
        App::update,
        App::view,
    )
    .title(App::title)
    .subscription(App::subscription)
    .settings(iced::Settings {
        default_font: fonts::default(),
        ..Default::default()
    })
    .theme(App::theme)
    .exit_on_close_request(false)
    .run()
}

struct App {
    game: Game,
    debugger_enabled: bool,
    action_bar: ActionBar,
    audio_output: Option<AudioOutput>,
    save_path: Option<PathBuf>,
}

enum Game {
    Unloaded,
    Loading,
    Loaded(LoadedGame),
}

enum LoadedGame {
    Debugger(debugger::Debugger),
    Emulator(emulator::Emulator),
}

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),

    Run,
    Pause,
    Reset,

    PressButton(joypad::Button),
    ReleaseButton(joypad::Button),

    ToggleDebugger(bool),
    ShowSettings,

    CloseRequested,

    ActionBar(action_bar::Message),
    Debugger(debugger::Message),
    Emulator(emulator::Message),

    None,
}

impl App {
    fn new(rom_path: Option<PathBuf>, debugger: bool) -> Self {
        let (game, save_path) = match rom_path {
            Some(rom_path) => {
                let sav_path = load::save_path(&rom_path);
                let save_data = fs::read(&sav_path).ok();
                let game_boy =
                    GameBoy::new(Cartridge::new(fs::read(&rom_path).unwrap(), save_data));
                let game = Game::Loaded(if debugger {
                    LoadedGame::Debugger(debugger::Debugger::new(game_boy))
                } else {
                    let mut emu = emulator::Emulator::new(game_boy);
                    emu.run();
                    LoadedGame::Emulator(emu)
                });
                (game, Some(sav_path))
            }

            None => (Game::Unloaded, None),
        };

        Self {
            game,
            debugger_enabled: debugger,
            action_bar: ActionBar::new(),
            audio_output: AudioOutput::new(),
            save_path,
        }
    }

    fn title(&self) -> String {
        if let Game::Loaded(game) = &self.game {
            match game {
                LoadedGame::Debugger(debugger) => {
                    format!("{} - MissingNo.", debugger.game_boy().cartridge().title())
                }
                LoadedGame::Emulator(emulator) => {
                    format!("{} - MissingNo.", emulator.game_boy().cartridge().title())
                }
            }
        } else {
            "MissingNo.".into()
        }
    }

    fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Load(message) => return load::update(message, self),

            Message::Run => self.run(),
            Message::Pause => self.pause(),
            Message::Reset => {
                self.save();
                self.reset();
            }

            Message::CloseRequested => {
                self.save();
                return window::latest().and_then(window::close);
            }

            Message::PressButton(button) => self.press_button(button),
            Message::ReleaseButton(button) => self.release_button(button),

            Message::ToggleDebugger(debugger_enabled) => {
                self.debugger_enabled = debugger_enabled;

                if let Game::Loaded(game) = &mut self.game {
                    replace_with_or_abort(game, |game| match game {
                        LoadedGame::Debugger(debugger) => {
                            if debugger_enabled {
                                LoadedGame::Debugger(debugger)
                            } else {
                                LoadedGame::Emulator(debugger.disable_debugger())
                            }
                        }
                        LoadedGame::Emulator(emulator) => {
                            if debugger_enabled {
                                LoadedGame::Debugger(emulator.enable_debugger())
                            } else {
                                LoadedGame::Emulator(emulator)
                            }
                        }
                    });
                }
            }
            Message::ShowSettings => {}

            Message::ActionBar(message) => return self.action_bar.update(message),
            Message::Emulator(message) => {
                if let Game::Loaded(LoadedGame::Emulator(emulator)) = &mut self.game {
                    let task = emulator.update(message);
                    self.drain_audio();
                    return task;
                }
            }

            Message::Debugger(message) => {
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    let task = debugger.update(message);
                    self.drain_audio();
                    return task;
                }
            }

            Message::None => {}
        }

        return Task::none();
    }

    fn view(&self) -> Element<'_, Message> {
        column![
            self.action_bar.view(self),
            horizontal_rule(),
            container(self.inner()).center(Fill)
        ]
        .into()
    }

    fn inner(&self) -> Element<'_, Message> {
        match &self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.view(),
                LoadedGame::Emulator(emulator) => emulator.view(),
            },
            _ => column![
                text::xl("Welcome to MissingNo.!"),
                icons::xl(Icon::GameBoy)
                    .width(200)
                    .height(200)
                    .style(|theme, _| {
                        svg::Style {
                            color: Some(theme.extended_palette().success.strong.color),
                        }
                    })
            ]
            .align_x(Center)
            .spacing(l())
            .into(),
        }
    }

    fn drain_audio(&mut self) {
        let game_boy = match &mut self.game {
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.game_boy_mut(),
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.game_boy_mut(),
            _ => return,
        };
        let samples = game_boy.drain_audio_samples();
        if let Some(audio) = &mut self.audio_output {
            audio.push_samples(&samples);
        }
    }

    pub fn running(&self) -> bool {
        match &self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.running(),
                LoadedGame::Emulator(emulator) => emulator.running(),
            },
            _ => false,
        }
    }

    fn run(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.run(),
                LoadedGame::Emulator(emulator) => emulator.run(),
            },
            _ => {}
        }
    }

    fn pause(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.pause(),
                LoadedGame::Emulator(emulator) => emulator.pause(),
            },
            _ => {}
        }
    }

    fn reset(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.reset(),
                LoadedGame::Emulator(emulator) => emulator.reset(),
            },
            _ => {}
        }
    }

    fn press_button(&mut self, button: Button) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.press_button(button),
                LoadedGame::Emulator(emulator) => emulator.press_button(button),
            },
            _ => {}
        }
    }

    fn release_button(&mut self, button: Button) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.release_button(button),
                LoadedGame::Emulator(emulator) => emulator.release_button(button),
            },
            _ => {}
        }
    }

    fn save(&self) {
        let Some(save_path) = &self.save_path else {
            return;
        };
        let cartridge = match &self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.game_boy().cartridge(),
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.game_boy().cartridge(),
            _ => return,
        };
        if !cartridge.has_battery() {
            return;
        }
        if let Some(ram) = cartridge.ram() {
            let _ = fs::write(save_path, ram);
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            if self.running() {
                event::listen_with(controls::event_handler)
            } else {
                Subscription::none()
            },
            if self.running() {
                controls::gamepad_subscription()
            } else {
                Subscription::none()
            },
            event::listen_with(|event, _, _| match event {
                iced::Event::Window(window::Event::CloseRequested) => Some(Message::CloseRequested),
                _ => None,
            }),
            match &self.game {
                Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.subscription(),
                Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.subscription(),
                _ => Subscription::none(),
            },
        ])
    }
}
