mod debugger;

use crate::{
    emulator::{GameBoy, cartridge::Cartridge},
    ui::debugger::debugger,
};

use debugger::DebuggerUi;
use iced::{
    Element,
    Length::Fill,
    Task, Theme,
    widget::{button, container},
};
use rfd::{AsyncFileDialog, FileHandle};
use std::{fs, path::PathBuf};

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
    Loaded(DebuggerUi),
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
                load_state: LoadState::Loaded(DebuggerUi::new(GameBoy::new(Cartridge::new(
                    fs::read(rom_path).unwrap(),
                )))),
            },
            None => Self {
                load_state: LoadState::Unloaded,
            },
        }
    }

    fn title(&self) -> String {
        if let LoadState::Loaded(debugger) = &self.load_state {
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
                    self.load_state = LoadState::Loading;
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
                        self.load_state = LoadState::Unloaded;
                    }
                }

                LoadMessage::GameRomLoaded(rom) => {
                    self.load_state =
                        LoadState::Loaded(DebuggerUi::new(GameBoy::new(Cartridge::new(rom))));
                }
            },

            Message::Debugger(message) => match &mut self.load_state {
                LoadState::Loaded(debugger) => return debugger::update(debugger, message),
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
        match &self.load_state {
            LoadState::Unloaded => button("Load game")
                .on_press(Message::Load(LoadMessage::PickGameRom))
                .into(),
            LoadState::Loading => button("Load game").into(),
            LoadState::Loaded(debugger) => debugger::debugger(&debugger),
        }
    }
}
