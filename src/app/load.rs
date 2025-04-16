use iced::Task;
use rfd::{AsyncFileDialog, FileHandle};

use crate::{
    app::{self, Game},
    emulator::{GameBoy, cartridge::Cartridge},
};

#[derive(Debug, Clone)]
pub enum Message {
    Pick,
    Picked(Option<FileHandle>),
    Loaded(Vec<u8>),
}

impl From<Message> for app::Message {
    fn from(value: Message) -> Self {
        Self::Load(value)
    }
}

pub fn update(message: Message, game: &mut Game) -> Task<app::Message> {
    match message {
        Message::Pick => {
            *game = Game::Loading;
            return Task::perform(
                AsyncFileDialog::new()
                    .add_filter("Game Boy ROM", &["gb"])
                    .pick_file(),
                |file_handle| Message::Picked(file_handle).into(),
            );
        }

        Message::Picked(file_handle) => {
            if let Some(handle) = file_handle {
                let file = handle.clone();
                return Task::perform(async move { file.read().await }, |data| {
                    Message::Loaded(data).into()
                });
            } else {
                *game = Game::Unloaded;
            }
        }

        Message::Loaded(rom) => {
            *game = Game::Loaded(app::debugger::Debugger::new(GameBoy::new(Cartridge::new(
                rom,
            ))));
        }
    }

    Task::none()
}
