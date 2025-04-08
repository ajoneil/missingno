use crate::{
    emulation::{Cartridge, GameBoy},
    ui::emulator::emulator,
};
use iced::{
    Element,
    Length::Fill,
    Task, Theme,
    widget::{button, container},
};
use rfd::{AsyncFileDialog, FileHandle};
use std::{fs, path::PathBuf};

mod cpu;
mod emulator;
mod instructions;

pub fn run(rom_path: Option<PathBuf>) -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .theme(App::theme)
        .run_with(|| (App::new(rom_path), Task::none()))
}

struct App {
    load_state: LoadState,
}

enum LoadState {
    Unloaded,
    Loading,
    Loaded(GameBoy),
}

#[derive(Debug, Clone)]
enum Message {
    Load(LoadMessage),
    Emulator(emulator::Message),
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
                load_state: LoadState::Loaded(GameBoy::new(Cartridge::new(
                    fs::read(rom_path).unwrap(),
                ))),
            },
            None => Self {
                load_state: LoadState::Unloaded,
            },
        }
    }

    fn title(&self) -> String {
        if let LoadState::Loaded(game_boy) = &self.load_state {
            format!("{} - MissingNo.", game_boy.cartridge().title())
        } else {
            "MissingNo.".into()
        }
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Load(load_message) => match load_message {
                LoadMessage::PickGameRom => {
                    self.load_state = LoadState::Loading;
                    Task::perform(
                        AsyncFileDialog::new()
                            .add_filter("Game Boy ROM", &["gb"])
                            .pick_file(),
                        |result| Message::Load(LoadMessage::GameRomPicked(result)),
                    )
                }
                LoadMessage::GameRomPicked(file_handle) => {
                    if let Some(handle) = file_handle {
                        let file = handle.clone();
                        Task::perform(async move { file.read().await }, |result| {
                            Message::Load(LoadMessage::GameRomLoaded(result))
                        })
                    } else {
                        self.load_state = LoadState::Unloaded;
                        Task::none()
                    }
                }
                LoadMessage::GameRomLoaded(rom) => {
                    self.load_state = LoadState::Loaded(GameBoy::new(Cartridge::new(rom)));
                    Task::none()
                }
            },
            Message::Emulator(message) => match &mut self.load_state {
                LoadState::Loaded(game_boy) => emulator::update(game_boy, message),
                _ => Task::none(),
            },
        }
    }

    fn view(&self) -> Element<'_, Message> {
        container(self.inner()).center_x(Fill).center_y(Fill).into()
    }

    fn inner(&self) -> Element<'_, Message> {
        match &self.load_state {
            LoadState::Unloaded => button("Load game")
                .on_press(Message::Load(LoadMessage::PickGameRom))
                .into(),
            LoadState::Loading => button("Load game").into(),
            LoadState::Loaded(game_boy) => emulator(&game_boy),
        }
    }
}
