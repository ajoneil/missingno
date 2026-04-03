use std::path::PathBuf;

use iced::Task;
use rfd::{AsyncFileDialog, FileHandle};

use crate::app::{self, App, CurrentGame, Game, LoadedGame, Screen, library};
use missingno_gb::{GameBoy, cartridge::Cartridge};

#[derive(Debug, Clone)]
#[allow(dead_code)]
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

/// Select a game from the library by SHA1 and populate CurrentGame.
/// Does NOT start emulation — just loads metadata, cover, and play log.
pub fn select_game(app: &mut App, sha1: &str) -> bool {
    let Some((game_dir, entry)) = library::find_by_sha1(sha1) else {
        return false;
    };

    // Migrate legacy saves if present
    library::saves::migrate_legacy_battery(&game_dir);
    library::saves::migrate_individual_saves(&game_dir);

    let cover = library::load_cover(&game_dir)
        .map(|bytes| iced::widget::image::Handle::from_bytes(bytes));

    let play_log = library::play_log::load(&game_dir);
    let save_manifest = library::saves::load_manifest(&game_dir);

    app.current_game = Some(CurrentGame {
        entry,
        game_dir,
        cover,
        play_log,
        save_manifest,
    });
    true
}

/// Start emulation for the currently selected game.
/// Requires current_game to be set (via select_game or setup_game).
pub fn play_current_game(app: &mut App) -> Task<app::Message> {
    // Extract what we need before borrowing mutably
    let (rom_path, game_dir) = {
        let Some(current) = &app.current_game else {
            return Task::none();
        };
        let Some(rom_path) = current.entry.rom_paths.iter().find(|p| p.exists()).cloned() else {
            return Task::none();
        };
        (rom_path, current.game_dir.clone())
    };

    let Ok(rom) = std::fs::read(&rom_path) else {
        return Task::none();
    };

    let save_data = library::saves::load_current_save(&game_dir);
    let cartridge = Cartridge::new(rom, save_data);
    let game_boy = GameBoy::new(cartridge, None);
    let palette = app.settings.palette;

    if app.debugger_enabled {
        let mut debugger = app::debugger::Debugger::new(game_boy);
        debugger.set_palette(palette);
        app.game = Game::Loaded(LoadedGame::Debugger(debugger));
    } else {
        let mut emu = app::emulator::Emulator::new(game_boy, app.settings.use_sgb_colors);
        emu.set_palette(palette);
        emu.run();
        app.game = Game::Loaded(LoadedGame::Emulator(emu));
    }

    // Start play session
    if let Some(current) = &mut app.current_game {
        current.play_log.start_session();
        library::play_log::save(&current.game_dir, &current.play_log);
        app.recent_games
            .add(&current.entry.sha1, &current.entry.display_title(), &rom_path);
        app.recent_games.save();
    }

    app.screen = Screen::Emulator;
    app.library_cache = app::library::view::LibraryCache::load();

    Task::none()
}

/// Start emulation with a specific save file.
pub fn play_with_save(app: &mut App, save_id: &str) -> Task<app::Message> {
    let (rom_path, game_dir) = {
        let Some(current) = &app.current_game else {
            return Task::none();
        };
        let Some(rom_path) = current.entry.rom_paths.iter().find(|p| p.exists()).cloned() else {
            return Task::none();
        };
        (rom_path, current.game_dir.clone())
    };

    let Ok(rom) = std::fs::read(&rom_path) else {
        return Task::none();
    };

    let save_data = library::saves::load_save_by_id(&game_dir, save_id);
    let cartridge = Cartridge::new(rom, save_data);
    let game_boy = GameBoy::new(cartridge, None);
    let palette = app.settings.palette;

    if app.debugger_enabled {
        let mut debugger = app::debugger::Debugger::new(game_boy);
        debugger.set_palette(palette);
        app.game = Game::Loaded(LoadedGame::Debugger(debugger));
    } else {
        let mut emu = app::emulator::Emulator::new(game_boy, app.settings.use_sgb_colors);
        emu.set_palette(palette);
        emu.run();
        app.game = Game::Loaded(LoadedGame::Emulator(emu));
    }

    if let Some(current) = &mut app.current_game {
        current.play_log.start_session();
        library::play_log::save(&current.game_dir, &current.play_log);
        app.recent_games
            .add(&current.entry.sha1, &current.entry.display_title(), &rom_path);
        app.recent_games.save();
    }

    app.screen = Screen::Emulator;
    app.library_cache = app::library::view::LibraryCache::load();

    Task::none()
}

/// Full pipeline for loading a ROM from a file path: create library entry + start emulation.
pub fn setup_game(app: &mut App, rom_path: PathBuf, rom: Vec<u8>) -> Task<app::Message> {
    let sha1 = library::hasheous::rom_sha1(&rom);

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

        // Import .sav from next to ROM if the library doesn't have saves yet
        let legacy_sav = rom_path.with_extension("sav");
        let mut save_manifest = library::saves::load_manifest(&game_dir);
        if legacy_sav.exists() && save_manifest.saves.is_empty() {
            if let Ok(data) = std::fs::read(&legacy_sav) {
                let entry_save = save_manifest.record_legacy_import(data.len() as u32);
                let archive_idx = entry_save.archive_index.unwrap();
                library::saves::write_save_data(&game_dir, &data, archive_idx, None);
                library::saves::save_manifest(&game_dir, &save_manifest);
            }
        }

        library::save_entry(&game_dir, &entry);
        (game_dir, entry)
    };

    // Migrate legacy battery.sav if present
    library::saves::migrate_legacy_battery(&game_dir);

    // Add this ROM path if not already tracked
    entry.add_rom_path(rom_path.clone());
    library::save_entry(&game_dir, &entry);

    // Load save data and cover
    let save_data = library::saves::load_current_save(&game_dir);
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
        let mut emu = app::emulator::Emulator::new(game_boy, app.settings.use_sgb_colors);
        emu.set_palette(palette);
        emu.run();
        app.game = Game::Loaded(LoadedGame::Emulator(emu));
    }

    // Update app state
    let mut play_log = library::play_log::load(&game_dir);
    play_log.start_session();
    library::play_log::save(&game_dir, &play_log);

    let save_manifest = library::saves::load_manifest(&game_dir);

    app.current_game = Some(CurrentGame {
        entry: entry.clone(),
        game_dir,
        cover,
        play_log,
        save_manifest,
    });
    app.screen = Screen::Emulator;

    app.recent_games
        .add(&entry.sha1, &entry.display_title(), &rom_path);
    app.recent_games.save();
    app.library_cache = app::library::view::LibraryCache::load();

    Task::none()
}
