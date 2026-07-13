use std::path::PathBuf;

use iced::Task;
use jiff::Timestamp;
use rfd::{AsyncFileDialog, FileHandle};

use crate::app::{self, App, CurrentGame, Game, LoadedGame, Screen, library, system};

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
            // "All supported" first so it is the default filter, then one
            // per family for narrowing.
            let mut all_extensions: Vec<&str> = system::FAMILIES
                .iter()
                .flat_map(|family| family.extensions.iter().copied())
                .collect();
            all_extensions.sort_unstable();
            all_extensions.dedup();
            let mut dialog =
                AsyncFileDialog::new().add_filter("All supported ROMs", &all_extensions);
            for family in system::FAMILIES {
                dialog = dialog.add_filter(family.platform.name(), family.extensions);
            }
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

/// Build the console for a ROM and wrap it for the active mode (debugger or
/// emulator), storing it in `app.game`. Returns the game's title; `None`
/// when no family claims the media or it fails to parse.
fn start_console(
    app: &mut App,
    rom: Vec<u8>,
    save_data: Option<Vec<u8>>,
    rom_path: &std::path::Path,
    game_dir: &std::path::Path,
) -> Option<String> {
    let family = system::family_for(rom_path, &rom)?;
    let entry = crate::app::library::load_entry(game_dir);
    let console = (family.create_console)(system::MediaLoad {
        rom: &rom,
        fallback_title: file_stem_title(rom_path),
        save_data,
        boot_rom: app.boot_rom.clone(),
        tv_standard: entry.as_ref().and_then(|e| e.tv_standard),
        cart_type: entry.as_ref().and_then(|e| e.cart_type.clone()),
        serial_link: &mut app.serial_link,
        print_sink: Some(app.print_tx.clone()),
    })?;
    Some(finish_start(app, console, rom_path))
}

pub(crate) fn file_stem_title(rom_path: &std::path::Path) -> String {
    rom_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

/// Wrap a built console for the active mode (debugger where the system
/// supports one, plain emulation otherwise) and store it in `app.game`.
fn finish_start(
    app: &mut App,
    console: Box<dyn system::SystemConsole>,
    rom_path: &std::path::Path,
) -> String {
    let title = console.game_title();
    let palette = app.settings.palette;

    let console = if app.debugger_enabled {
        match app::debugger::Debugger::new(console) {
            Ok(mut debugger) => {
                debugger.load_sidecars(rom_path);
                debugger.set_palette(palette);
                debugger.set_frame_blending(app.settings.frame_blending);
                app.game = Game::Loaded(LoadedGame::Debugger(debugger));
                return title;
            }
            // No debugger backend for this system: plain emulation.
            Err(console) => console,
        }
    } else {
        console
    };
    let mut emu = app::emulator::Emulator::new(
        console,
        app.settings.use_sgb_colors,
        app.settings.frame_blending,
    );
    emu.set_palette(palette);
    emu.set_running(true);
    app.game = Game::Loaded(LoadedGame::Emulator(emu));
    app.start_running();
    title
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
    let Some(cartridge_title) = start_console(app, rom, save_data, &rom_path, &game_dir) else {
        eprintln!("unsupported ROM: {}", rom_path.display());
        app.game = Game::Unloaded;
        return Task::none();
    };

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
    let Some(cartridge_title) = start_console(app, rom, save_data, &rom_path, &game_dir) else {
        eprintln!("unsupported ROM: {}", rom_path.display());
        app.game = Game::Unloaded;
        return Task::none();
    };

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
    // Classify before touching the library so unsupported files don't get
    // library entries.
    let Some(family) = system::family_for(&rom_path, &rom) else {
        eprintln!("unsupported ROM: {}", rom_path.display());
        app.game = Game::Unloaded;
        return Task::none();
    };

    let sha1 = library::hasheous::rom_sha1(&rom);

    // Check library for existing game
    let (game_dir, mut entry) = if let Some((dir, existing)) = library::find_by_sha1(&sha1) {
        (dir, existing)
    } else {
        // New game — create entry with ROM header title
        let header_title = (family.title_from_rom)(&rom);
        let title = header_title
            .clone()
            .unwrap_or_else(|| file_stem_title(&rom_path));
        let mut entry = library::GameEntry::new(sha1.clone(), title, rom_path.clone());
        entry.header_title = header_title;
        entry.platform = Some(family.platform);
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

    // Add this ROM path if not already tracked; older entries may predate
    // platform classification, so stamp it while the ROM is at hand.
    entry.platform.get_or_insert(family.platform);
    entry.add_rom_path(rom_path.clone());
    library::save_entry(&game_dir, &entry);

    // Load save data and cover
    let save_data = library::activity::load_current_sram(&game_dir);
    let initial_sram = save_data.clone();
    let cover = library::load_cover(&game_dir).map(iced::widget::image::Handle::from_bytes);

    // Create cartridge and start emulation
    let Some(cartridge_title) = start_console(app, rom, save_data, &rom_path, &game_dir) else {
        eprintln!("unsupported ROM: {}", rom_path.display());
        app.game = Game::Unloaded;
        return Task::none();
    };

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
