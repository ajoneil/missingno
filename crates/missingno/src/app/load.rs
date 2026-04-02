use std::path::PathBuf;

use iced::Task;
use rfd::{AsyncFileDialog, FileHandle};

use crate::app::{self, App, CurrentGame, Game, LoadedGame, hasheous, library};
use missingno_gb::{GameBoy, cartridge::Cartridge};

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
                app.recent_games.remove_path(&rom_path);
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
            return setup_game(app, rom_path, rom);
        }
    }

    Task::none()
}

pub fn setup_game(app: &mut App, rom_path: PathBuf, rom: Vec<u8>) -> Task<app::Message> {
    let sha1 = hasheous::rom_sha1(&rom);

    // Check library for existing game
    let (game_dir, mut entry) = if let Some((dir, existing)) = library::find_by_sha1(&sha1) {
        (dir, existing)
    } else {
        // New game — create entry with ROM header title
        let cartridge_title = Cartridge::peek_title(&rom);
        let title = if cartridge_title.is_empty() {
            "Unknown".to_string()
        } else {
            cartridge_title
        };
        let entry = library::GameEntry::new(sha1.clone(), title, rom_path.clone());
        let game_dir = library::game_dir_for(&entry.title, &entry.sha1)
            .expect("Could not determine library directory");

        // Copy .sav from next to ROM if the library doesn't have one yet
        let legacy_sav = rom_path.with_extension("sav");
        if legacy_sav.exists() && !library::battery_path(&game_dir).exists() {
            let _ = std::fs::create_dir_all(&game_dir);
            let _ = std::fs::copy(&legacy_sav, library::battery_path(&game_dir));
        }

        library::save_entry(&game_dir, &entry);
        (game_dir, entry)
    };

    // Add this ROM path if not already tracked
    entry.add_rom_path(rom_path.clone());
    library::save_entry(&game_dir, &entry);

    // Load save data from library
    let save_data = library::load_battery(&game_dir);

    // Load cached cover art
    let cover = library::load_cover(&game_dir)
        .map(|bytes| iced::widget::image::Handle::from_bytes(bytes));

    // Create cartridge and start emulation
    let cartridge = Cartridge::new(rom, save_data);
    let game_boy = GameBoy::new(cartridge, None);
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

    // Update app state
    app.current_game = Some(CurrentGame {
        entry: entry.clone(),
        game_dir,
        cover,
    });
    app.game_info_shown = false;

    // Update recent games
    app.recent_games.add(&entry.sha1, &entry.title, &rom_path);
    app.recent_games.save();

    // Fire async Hasheous lookup if internet enabled
    if app.settings.internet_enabled {
        Task::perform(
            smol::unblock(move || hasheous::lookup(&sha1).ok().flatten()),
            |info| app::Message::GameInfoLoaded(info),
        )
    } else {
        Task::none()
    }
}
