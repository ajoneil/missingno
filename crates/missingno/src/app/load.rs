use std::path::PathBuf;

use iced::Task;
use jiff::Timestamp;
use rfd::{AsyncFileDialog, FileHandle};

use crate::app::{self, App, CurrentGame, Game, LoadedGame, Screen, library, system};
use missingno_gb::cartridge::Cartridge;

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
            let mut dialog = AsyncFileDialog::new()
                .add_filter(system::gb::ROM_FILTER_NAME, system::gb::ROM_EXTENSIONS);
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
            let rom = crate::patch::soft_patch(&rom_path, rom);
            return setup_game(app, rom_path, rom);
        }
    }

    Task::none()
}

/// A cartridge from ROM + saved battery contents: any RTC tail in the save
/// restores the clock and catches it up on the time since the save.
fn build_cartridge(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Cartridge {
    let (ram, rtc) = match save_data {
        Some(blob) => {
            let (ram, rtc) = crate::sram::split_blob(blob);
            (Some(ram), rtc)
        }
        None => (None, None),
    };
    let mut cartridge = Cartridge::new(rom, ram);
    if let Some((snapshot, saved_at)) = rtc {
        let elapsed = crate::sram::now_unix().saturating_sub(saved_at);
        cartridge.restore_rtc(snapshot, elapsed);
    }
    cartridge
}

/// Build the console for a ROM and wrap it for the active mode (debugger or
/// emulator), storing it in `app.game`. Returns the cartridge header title.
fn start_console(
    app: &mut App,
    cartridge: Cartridge,
    rom_path: &std::path::Path,
    game_dir: &std::path::Path,
) -> String {
    let cartridge_title = cartridge.title().to_string();
    let mut console = system::gb::create_console(cartridge, None);
    if let Some(link) = app.serial_link.take() {
        console.set_link(link);
    } else {
        // A virtual printer sits on the link port by default; it stays inert
        // unless a game prints, and prints land in the game's folder.
        console.set_link(Box::new(crate::printer::GbPrinter::new(
            game_dir.join("prints"),
        )));
    }
    let palette = app.settings.palette;

    if app.debugger_enabled {
        let mut debugger = app::debugger::Debugger::new(console);
        debugger.load_symbols(rom_path);
        debugger.set_palette(palette);
        debugger.set_frame_blending(app.settings.frame_blending);
        app.game = Game::Loaded(LoadedGame::Debugger(debugger));
    } else {
        let mut emu = app::emulator::Emulator::new(
            console,
            app.settings.use_sgb_colors,
            app.settings.frame_blending,
        );
        emu.set_palette(palette);
        emu.set_running(true);
        app.game = Game::Loaded(LoadedGame::Emulator(emu));
        app.start_running();
    }
    cartridge_title
}

/// Select a game from the library by SHA1 and populate CurrentGame.
/// Does NOT start emulation — just loads metadata and cover.
pub fn select_game(app: &mut App, sha1: &str) -> bool {
    let Some((game_dir, entry)) = library::find_by_sha1(sha1) else {
        return false;
    };

    let cover = library::load_cover(&game_dir).map(iced::widget::image::Handle::from_bytes);

    app.current_game = Some(CurrentGame {
        entry,
        game_dir,
        cover,
        session: None,
        started_from: None,
        initial_sram: None,
        cartridge_title: String::new(),
    });
    true
}

/// Start emulation for the currently selected game.
/// Requires current_game to be set (via select_game or setup_game).
pub fn play_current_game(app: &mut App) -> Task<app::Message> {
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
    let rom = crate::patch::soft_patch(&rom_path, rom);

    let save_data = library::activity::load_current_sram(&game_dir);
    let initial_sram = save_data.clone();
    let cartridge_title = start_console(app, build_cartridge(rom, save_data), &rom_path, &game_dir);

    // Start play session
    if let Some(current) = &mut app.current_game {
        let session =
            library::activity::SessionFile::new(Timestamp::now(), current.started_from.clone());
        library::activity::write_session(&current.game_dir, &session);
        current.session = Some(session);
        current.started_from = None;
        current.initial_sram = initial_sram;
        current.cartridge_title = cartridge_title;
        app.store.reset_live_screenshots();

        app.recent_games.add(
            &current.entry.sha1,
            &current.entry.display_title(),
            &rom_path,
        );
        app.recent_games.save();
    }

    app.screen = Screen::Emulator;
    if let Some(current) = &app.current_game {
        app.store.notify_activity_changed(&current.entry.sha1);
    }

    Task::none()
}

/// Start emulation with a specific save from an activity file.
pub fn play_with_save(app: &mut App, activity_filename: &str) -> Task<app::Message> {
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
    let rom = crate::patch::soft_patch(&rom_path, rom);

    let save_data = library::activity::load_sram_from(&game_dir, activity_filename);
    let initial_sram = save_data.clone();
    let cartridge_title = start_console(app, build_cartridge(rom, save_data), &rom_path, &game_dir);

    if let Some(current) = &mut app.current_game {
        let session = library::activity::SessionFile::new(
            Timestamp::now(),
            Some(activity_filename.to_string()),
        );
        library::activity::write_session(&current.game_dir, &session);
        current.session = Some(session);
        current.started_from = None;
        current.initial_sram = initial_sram;
        current.cartridge_title = cartridge_title;

        app.recent_games.add(
            &current.entry.sha1,
            &current.entry.display_title(),
            &rom_path,
        );
        app.recent_games.save();
    }

    app.screen = Screen::Emulator;
    if let Some(current) = &app.current_game {
        app.store.notify_activity_changed(&current.entry.sha1);
    }

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
        let header_title = Cartridge::peek_title(&rom);
        let title = if header_title.is_empty() {
            "Unknown".to_string()
        } else {
            header_title.clone()
        };
        let mut entry = library::GameEntry::new(sha1.clone(), title, rom_path.clone());
        entry.header_title = if header_title.is_empty() {
            None
        } else {
            Some(header_title)
        };
        let game_dir = library::game_dir_for(&entry.title, &entry.sha1)
            .expect("Could not determine library directory");

        // Import .sav from next to ROM if no activity exists yet
        let legacy_sav = rom_path.with_extension("sav");
        if legacy_sav.exists() {
            library::activity::import_legacy_sav(&game_dir, &legacy_sav);
        }

        library::save_entry(&game_dir, &entry);
        (game_dir, entry)
    };

    // Add this ROM path if not already tracked
    entry.add_rom_path(rom_path.clone());
    library::save_entry(&game_dir, &entry);

    // Load save data and cover
    let save_data = library::activity::load_current_sram(&game_dir);
    let initial_sram = save_data.clone();
    let cover = library::load_cover(&game_dir).map(iced::widget::image::Handle::from_bytes);

    // Create cartridge and start emulation
    let cartridge_title = start_console(app, build_cartridge(rom, save_data), &rom_path, &game_dir);

    let session = library::activity::SessionFile::new(Timestamp::now(), None);
    library::activity::write_session(&game_dir, &session);

    let game_dir_clone = game_dir.clone();
    app.current_game = Some(CurrentGame {
        entry: entry.clone(),
        game_dir,
        cover,
        session: Some(session),
        started_from: None,
        initial_sram,
        cartridge_title,
    });
    app.screen = Screen::Emulator;

    app.recent_games
        .add(&entry.sha1, &entry.display_title(), &rom_path);
    app.recent_games.save();
    app.store.notify_game_added(&entry.sha1, game_dir_clone);

    Task::none()
}
