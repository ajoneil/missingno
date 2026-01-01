use std::{fs, path::PathBuf};

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, Task, Theme, event,
    widget::{column, container, svg},
};
use replace_with::replace_with_or_abort;

use crate::game_boy::{
    GameBoy,
    cartridge::Cartridge,
    joypad::{self, Button},
};
use action_bar::ActionBar;
use core::{
    fonts, horizontal_rule,
    icons::{self, Icon},
    sizes::l,
    text,
};

mod action_bar;
mod controls;
mod core;
mod debugger;
mod emulator;
mod load;
mod screen;

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
    .run()
}

struct App {
    game: Game,
    debugger_enabled: bool,
    action_bar: ActionBar,
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

    ActionBar(action_bar::Message),
    Debugger(debugger::Message),
    Emulator(emulator::Message),

    None,
}

impl App {
    fn new(rom_path: Option<PathBuf>, debugger: bool) -> Self {
        let game = match rom_path {
            Some(rom_path) => {
                let game_boy = GameBoy::new(Cartridge::new(fs::read(rom_path).unwrap()));
                Game::Loaded(if debugger {
                    LoadedGame::Debugger(debugger::Debugger::new(game_boy))
                } else {
                    let mut emu = emulator::Emulator::new(game_boy);
                    emu.run();
                    LoadedGame::Emulator(emu)
                })
            }

            None => Game::Unloaded,
        };

        Self {
            game,
            debugger_enabled: debugger,
            action_bar: ActionBar::new(),
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
            Message::Reset => self.reset(),

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
            Message::Emulator(message) => match &mut self.game {
                Game::Loaded(LoadedGame::Emulator(emulator)) => return emulator.update(message),
                _ => {}
            },

            Message::Debugger(message) => match &mut self.game {
                Game::Loaded(LoadedGame::Debugger(debugger)) => return debugger.update(message),
                _ => {}
            },

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

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            if self.running() {
                event::listen_with(controls::event_handler)
            } else {
                Subscription::none()
            },
            match &self.game {
                Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.subscription(),
                Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.subscription(),
                _ => Subscription::none(),
            },
        ])
    }
}
