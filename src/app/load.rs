use std::path::{Path, PathBuf};

use iced::Task;
use rfd::{AsyncFileDialog, FileHandle};

use crate::app::{self, App, Game, LoadedGame};
use missingno_core::game_boy::{GameBoy, cartridge::Cartridge};

#[derive(Debug, Clone)]
pub enum Message {
    Pick,
    Picked(Option<FileHandle>),
    LoadPath(PathBuf),
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
            let mut dialog = AsyncFileDialog::new().add_filter("Game Boy ROM", &["gb"]);
            if let Some(dir) = app.recent_games.most_recent_dir() {
                dialog = dialog.set_directory(dir);
            }
            return Task::perform(dialog.pick_file(), |file_handle| {
                Message::Picked(file_handle).into()
            });
        }

        Message::LoadPath(rom_path) => match std::fs::read(&rom_path) {
            Ok(rom) => return Task::done(Message::Loaded(rom_path, rom).into()),
            Err(_) => {
                app.recent_games.remove(&rom_path);
                app.recent_games.save();
            }
        },

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
            let cartridge = Cartridge::new(rom, save_data);
            let title = cartridge.title().to_string();
            let game_boy = GameBoy::new(cartridge);
            app.save_path = Some(sav_path);
            app.recent_games.add(rom_path.clone(), title);
            app.recent_games.save();
            let palette = app.settings.palette;
            if app.debugger_enabled {
                let mut debugger = app::debugger::Debugger::new(game_boy);
                debugger.set_palette(palette);
                app.game = Game::Loaded(LoadedGame::Debugger(debugger));
            } else {
                let mut emu = app::emulator::Emulator::new(game_boy);
                emu.set_palette(palette);
                emu.run();
                app.game = Game::Loaded(LoadedGame::Emulator(emu));
            }
        }
    }

    Task::none()
}
