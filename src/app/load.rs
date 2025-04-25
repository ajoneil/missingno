use iced::Task;
use rfd::{AsyncFileDialog, FileHandle};

use crate::{
    app::{self, App, Game, LoadedGame},
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

pub fn update(message: Message, app: &mut App) -> Task<app::Message> {
    match message {
        Message::Pick => {
            app.game = Game::Loading;
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
                app.game = Game::Unloaded;
            }
        }

        Message::Loaded(rom) => {
            let game_boy = GameBoy::new(Cartridge::new(rom));
            app.game = Game::Loaded(if app.debugger_enabled {
                LoadedGame::Debugger(app::debugger::Debugger::new(game_boy))
            } else {
                let mut emu = app::emulator::Emulator::new(game_boy);
                emu.run();
                LoadedGame::Emulator(emu)
            });
        }
    }

    Task::none()
}
