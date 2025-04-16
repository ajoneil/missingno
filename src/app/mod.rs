use std::{fs, path::PathBuf};

use iced::{
    Element,
    Length::Fill,
    Task, Theme,
    widget::{button, container},
};
use rfd::{AsyncFileDialog, FileHandle};

use crate::emulator::{GameBoy, cartridge::Cartridge};
use core::fonts;

mod core;
mod debugger;

pub fn run(rom_path: Option<PathBuf>) -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .settings(iced::Settings {
            default_font: fonts::default(),
            ..Default::default()
        })
        .theme(App::theme)
        .run_with(|| (App::new(rom_path), Task::none()))
}

struct App {
    game: Game,
}

enum Game {
    Unloaded,
    Loading,
    Loaded(debugger::State),
}

#[derive(Debug, Clone)]
enum Message {
    Load(LoadMessage),
    Debugger(debugger::Message),

    None,
}

#[derive(Debug, Clone)]
enum LoadMessage {
    PickGameRom,
    GameRomPicked(Option<FileHandle>),
    GameRomLoaded(Vec<u8>),
}

impl From<LoadMessage> for Message {
    fn from(value: LoadMessage) -> Self {
        Self::Load(value)
    }
}

impl App {
    fn new(rom_path: Option<PathBuf>) -> Self {
        match rom_path {
            Some(rom_path) => Self {
                game: Game::Loaded(debugger::State::new(GameBoy::new(Cartridge::new(
                    fs::read(rom_path).unwrap(),
                )))),
            },
            None => Self {
                game: Game::Unloaded,
            },
        }
    }

    fn title(&self) -> String {
        if let Game::Loaded(debugger) = &self.game {
            format!("{} - MissingNo.", debugger.game_boy().cartridge().title())
        } else {
            "MissingNo.".into()
        }
    }

    fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Load(load_message) => match load_message {
                LoadMessage::PickGameRom => {
                    self.game = Game::Loading;
                    return Task::perform(
                        AsyncFileDialog::new()
                            .add_filter("Game Boy ROM", &["gb"])
                            .pick_file(),
                        |result| Message::Load(LoadMessage::GameRomPicked(result)),
                    );
                }

                LoadMessage::GameRomPicked(file_handle) => {
                    if let Some(handle) = file_handle {
                        let file = handle.clone();
                        return Task::perform(async move { file.read().await }, |result| {
                            Message::Load(LoadMessage::GameRomLoaded(result))
                        });
                    } else {
                        self.game = Game::Unloaded;
                    }
                }

                LoadMessage::GameRomLoaded(rom) => {
                    self.game =
                        Game::Loaded(debugger::State::new(GameBoy::new(Cartridge::new(rom))));
                }
            },

            Message::Debugger(message) => match &mut self.game {
                Game::Loaded(debugger) => return debugger::update(debugger, message),
                _ => {}
            },
            Message::None => {}
        }

        return Task::none();
    }

    fn view(&self) -> Element<'_, Message> {
        container(self.inner()).center_x(Fill).center_y(Fill).into()
    }

    fn inner(&self) -> Element<'_, Message> {
        match &self.game {
            Game::Unloaded => button("Load game")
                .on_press(Message::Load(LoadMessage::PickGameRom))
                .into(),
            Game::Loading => button("Load game").into(),
            Game::Loaded(debugger) => debugger::debugger(&debugger),
        }
    }
}
