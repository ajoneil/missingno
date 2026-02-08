use std::path::{Path, PathBuf};

use iced::Task;
use rfd::{AsyncFileDialog, FileHandle};

use crate::{
    app::{self, App, Game, LoadedGame},
    game_boy::{GameBoy, cartridge::Cartridge},
};

#[derive(Debug, Clone)]
pub enum Message {
    Pick,
    Picked(Option<FileHandle>),
    Loaded(PathBuf, Vec<u8>),
}

impl From<Message> for app::Message {
    fn from(value: Message) -> Self {
        Self::Load(value)
    }
}

pub fn save_path(rom_path: &Path) -> PathBuf {
    rom_path.with_extension("sav")
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
                let path = handle.path().to_path_buf();
                return Task::perform(async move { handle.read().await }, move |data| {
                    Message::Loaded(path.clone(), data).into()
                });
            } else {
                app.game = Game::Unloaded;
            }
        }

        Message::Loaded(rom_path, rom) => {
            let sav_path = save_path(&rom_path);
            let save_data = std::fs::read(&sav_path).ok();
            let game_boy = GameBoy::new(Cartridge::new(rom, save_data));
            app.save_path = Some(sav_path);
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
