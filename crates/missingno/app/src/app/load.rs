use std::path::PathBuf;

use iced::Task;
use jiff::Timestamp;
use missingno_session::SharedSession;
use rfd::{AsyncFileDialog, FileHandle};

use crate::app::emulator::ConsoleFacts;
use crate::app::launch;
use crate::app::system::LaunchValues;
use crate::app::system::SystemConsole;
use crate::app::{self, App, CurrentGame, Game, LoadedGame, Notice, Screen, library, system};
use missingno_iced::ScreenView;

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
            for family in system::families_by_name() {
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

        // Every file the user picks goes through the launch window, so what it
        // will boot with — and what to change when it refuses — is in front of
        // them before it runs.
        Message::Loaded(rom_path, rom) => {
            let rom = crate::patch::soft_patch(&rom_path, rom);
            return Task::done(
                launch::Message::Opened(rom_path, rom, launch::Target::Transient).into(),
            );
        }
    }

    Task::none()
}

/// One launch, whatever raised it.
struct Request<'a> {
    rom: Vec<u8>,
    rom_path: &'a std::path::Path,
    save_data: Option<Vec<u8>>,
    /// The system to run it on, where the user named one for media no family
    /// claims; otherwise the media classifies itself.
    platform: Option<system::Platform>,
    /// The user's own word on the options this launch runs with.
    overrides: LaunchValues,
}

/// Build the console for a ROM and wrap it for the active mode (debugger or
/// emulator), storing it in `app.game`. Returns the game's title; `Err`
/// carries what the core refused the media with, for the caller to put in
/// front of the user.
fn start(app: &mut App, request: Request<'_>) -> Result<String, String> {
    let family = match request.platform {
        Some(platform) => system::family_of(platform),
        None => system::family_for(request.rom_path, &request.rom),
    }
    .ok_or("no system recognises this file")?;

    let sha1 = library::hasheous::rom_sha1(&request.rom);
    let facts = launch::facts(
        family,
        &request.rom,
        &app.catalogue,
        &sha1,
        app.boot_rom.as_ref(),
    );
    let values = launch::resolve(&(family.options)(), &request.overrides, &facts);

    let mut console = (family.create_console)(system::MediaLoad {
        rom: &request.rom,
        fallback_title: file_stem_title(request.rom_path),
        save_data: request.save_data,
        launch: values,
        serial_link: &mut app.serial_link,
        print_sink: Some(app.print_tx.clone()),
    })?;
    // The game's catalogued controllers decide what its ports carry: paddle
    // input is inert until a paddle pair is in the jack.
    let controllers = launch::catalogued_controllers(&app.catalogue, &sha1);
    for (port, peripheral) in (family.port_config)(&controllers) {
        let _ = console.plug(port, peripheral);
    }
    Ok(finish_start(
        app,
        console,
        request.rom_path,
        family.platform,
    ))
}

/// The one place a refusal reaches the user from a path that skipped the launch
/// window: nothing boots, and the notice offers the window where the console,
/// board or standard the launch stated can be changed.
fn refused(app: &mut App, error: String, reopen: launch::Message) {
    eprintln!("{error}");
    app.game = Game::Unloaded;
    app.show_notice(Notice::failure(error, "Launch options…", reopen.into()));
}

/// Start what the launch window is showing. A refusal goes back onto the
/// window, which stays open over whatever raised it.
pub fn launch_from_window(app: &mut App) -> Task<app::Message> {
    let Some(mut window) = app.launch_window.take() else {
        return Task::none();
    };

    let outcome = match window.target {
        launch::Target::Library => {
            if select_game(app, &window.sha1) {
                start_current_game(app, None, window.overrides.clone())
            } else {
                Err("this game is no longer in the library".to_string())
            }
        }
        launch::Target::Transient => install_and_start(
            app,
            window.rom_path.clone(),
            window.rom.clone(),
            window.platform,
            window.overrides.clone(),
        ),
    };

    match outcome {
        Ok(()) => Task::none(),
        Err(error) => {
            eprintln!("{}: {error}", window.rom_path.display());
            app.game = Game::Unloaded;
            window.error = Some(error);
            app.launch_window = Some(window);
            Task::none()
        }
    }
}

pub(crate) fn file_stem_title(rom_path: &std::path::Path) -> String {
    rom_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

/// Spawn the shared session hosting a built console and wrap it in the shell
/// for the active mode (debugger where the system supports one, plain emulation
/// otherwise), storing it in `app.game`.
fn finish_start(
    app: &mut App,
    console: Box<dyn SystemConsole>,
    rom_path: &std::path::Path,
    platform: system::Platform,
) -> String {
    let title = console.game_title();
    let palette = app.settings.palette;
    let (audio, sink) = App::open_audio();

    if app.debugger_enabled {
        let technology = console.video_out();
        let core = console.into_debugger();
        let regions = core.memory_regions();
        let session = SharedSession::spawn_with_audio(core, sink);
        let handle = session.handle();
        let mut screen_view = ScreenView::new();
        screen_view.set_technology(technology);
        let mut debugger = app::debugger::Debugger::new(handle, platform, regions, screen_view);
        debugger.load_sidecars(rom_path);
        debugger.set_palette(palette);
        // The game is installed first so the attach socket, if the user allows
        // one, publishes the platform it will serve.
        app.game = Game::Loaded(LoadedGame::Debugger(debugger));
        app.install_session(session, audio);
        return title;
    }

    // Plain emulation: the console runs on the fast path, auto-started.
    let facts = ConsoleFacts::of(console.as_ref());
    let session = SharedSession::spawn_console_with_audio(console, sink);
    let handle = session.handle();
    let mut emu =
        app::emulator::Emulator::new(handle, facts, platform, app.settings.presentation());
    emu.set_palette(palette);
    emu.run();
    app.game = Game::Loaded(LoadedGame::Emulator(emu));
    app.install_session(session, audio);
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

/// Start the currently selected library game, optionally from a save in its
/// activity log. `Err` carries what the core refused the media with.
fn start_current_game(
    app: &mut App,
    from_save: Option<&str>,
    overrides: LaunchValues,
) -> Result<(), String> {
    let (rom_path, game_dir, platform) = {
        let current = app
            .current_game
            .as_ref()
            .ok_or("no game is selected".to_string())?;
        let rom_path = current
            .entry
            .rom_paths
            .iter()
            .find(|path| path.exists())
            .cloned()
            .ok_or("this game's ROM file is missing".to_string())?;
        // The entry's own platform, so media no family claims still runs on the
        // system the user named for it.
        (rom_path, current.game_dir.clone(), current.entry.platform)
    };

    let rom = std::fs::read(&rom_path).map_err(|error| format!("{error}"))?;
    let rom = crate::patch::soft_patch(&rom_path, rom);

    let save_data = match from_save {
        Some(activity_filename) => library::activity::load_sram_from(&game_dir, activity_filename),
        None => library::activity::load_current_sram(&game_dir),
    };
    let initial_sram = save_data.clone();

    let cartridge_title = start(
        app,
        Request {
            rom,
            rom_path: &rom_path,
            save_data,
            platform,
            overrides,
        },
    )?;

    if let Some(current) = &mut app.current_game {
        let started_from = from_save
            .map(str::to_owned)
            .or_else(|| current.started_from.clone());
        let session = library::activity::SessionFile::new(Timestamp::now(), started_from);
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
    Ok(())
}

/// The overrides the selected game's library entry holds.
fn current_overrides(app: &App) -> LaunchValues {
    app.current_game
        .as_ref()
        .map(|current| current.entry.overrides.clone())
        .unwrap_or_default()
}

/// Play the selected game from a quick path — one that skips the launch window,
/// so a refusal has to come back as something the user can act on.
pub fn play_current_game(app: &mut App) -> Task<app::Message> {
    play_from(app, None)
}

/// Play the selected game from a specific save in its activity log.
pub fn play_with_save(app: &mut App, activity_filename: &str) -> Task<app::Message> {
    play_from(app, Some(activity_filename))
}

fn play_from(app: &mut App, from_save: Option<&str>) -> Task<app::Message> {
    let overrides = current_overrides(app);
    let sha1 = app
        .current_game
        .as_ref()
        .map(|current| current.entry.sha1.clone());
    if let Err(error) = start_current_game(app, from_save, overrides)
        && let Some(sha1) = sha1
    {
        refused(app, error, launch::Message::OpenForGame(sha1));
    }
    Task::none()
}

/// Full pipeline for a ROM at a path: give it a library entry, then start it.
/// `Err` carries what the core refused it with.
fn install_and_start(
    app: &mut App,
    rom_path: PathBuf,
    rom: Vec<u8>,
    platform: Option<system::Platform>,
    overrides: LaunchValues,
) -> Result<(), String> {
    // Classify before touching the library so unsupported files don't get
    // library entries.
    let family = match platform {
        Some(platform) => system::family_of(platform),
        None => system::family_for(&rom_path, &rom),
    }
    .ok_or("no system recognises this file")?;

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
            .ok_or("could not determine the library directory".to_string())?;

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

    let save_data = library::activity::load_current_sram(&game_dir);
    let initial_sram = save_data.clone();
    let cover = library::load_cover(&game_dir).map(iced::widget::image::Handle::from_bytes);

    let cartridge_title = start(
        app,
        Request {
            rom,
            rom_path: &rom_path,
            save_data,
            platform,
            overrides,
        },
    )?;

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
    Ok(())
}

/// Load a ROM named outside the library — on the command line, or from the
/// recent list — using whatever its entry already records.
pub fn setup_game(app: &mut App, rom_path: PathBuf, rom: Vec<u8>) -> Task<app::Message> {
    let sha1 = library::hasheous::rom_sha1(&rom);
    let overrides = library::find_by_sha1(&sha1)
        .map(|(_, entry)| entry.overrides)
        .unwrap_or_default();
    if let Err(error) = install_and_start(app, rom_path.clone(), rom, None, overrides) {
        refused(app, error, launch::Message::OpenForPath(rom_path));
    }
    Task::none()
}
