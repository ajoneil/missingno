use std::{fs, path::PathBuf, time::Instant};

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, Task, Theme, event, mouse, time,
    widget::{Stack, center, column, container, mouse_area, opaque, row, svg, text as iced_text},
    window,
};
use replace_with::replace_with_or_abort;

use action_bar::ActionBar;
use audio_output::AudioOutput;
use core::{
    buttons, fonts, horizontal_rule,
    icons::{self, Icon},
    sizes::{l, s},
    text,
};
use missingno_gb::{
    joypad::{self, Button},
    ppu::types::palette::PaletteChoice,
};

mod action_bar;
mod audio_output;
mod controls;
mod core;
mod debugger;
mod emulator;
pub mod library;
mod load;
mod recent;
mod screen;
pub mod settings;
mod settings_view;
mod texture_renderer;

pub fn run(rom_path: Option<PathBuf>, debugger: bool) -> iced::Result {
    let mut app = iced::application(
        move || App::new(rom_path.clone(), debugger),
        App::update,
        App::view,
    )
    .title(App::title)
    .subscription(App::subscription)
    .settings(iced::Settings {
        default_font: fonts::default(),
        ..Default::default()
    })
    .window(window::Settings {
        min_size: Some(iced::Size::new(800.0, 500.0)),
        platform_specific: window::settings::PlatformSpecific {
            application_id: "net.andyofniall.missingno".to_string(),
            ..Default::default()
        },
        ..Default::default()
    })
    .theme(App::theme)
    .exit_on_close_request(false);

    for font_data in fonts::load() {
        app = app.font(font_data);
    }

    app.run()
}

struct App {
    screen: Screen,
    game: Game,
    debugger_enabled: bool,
    fullscreen: Fullscreen,
    action_bar: ActionBar,
    audio_output: Option<AudioOutput>,
    recent_games: recent::RecentGames,
    settings: settings::Settings,
    /// The running emulation session. Only set when a game is actually loaded.
    current_game: Option<CurrentGame>,
    /// SHA1 of the game being viewed in the detail page (may differ from current_game).
    viewing_sha1: Option<String>,
    library_cache: library::view::LibraryCache,
    /// Action waiting for user confirmation (e.g. close game before launching another).
    pending_action: Option<PendingAction>,
    /// Index of the activity log entry currently hovered on the detail page.
    hovered_log_entry: Option<usize>,
}

#[derive(Debug, Clone)]
enum PendingAction {
    /// User wants to launch a different game — close current first.
    SwitchGame(String),
    /// User wants to close the app.
    CloseApp,
    /// User wants to reset the emulator.
    ResetEmulator,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Library,
    Settings,
    Detail,
    Emulator,
}

enum Fullscreen {
    Windowed,
    Active {
        cursor_hidden: bool,
        last_mouse_move: Instant,
    },
}

enum Game {
    Unloaded,
    Loading,
    Loaded(LoadedGame),
}

enum LoadedGame {
    Debugger(debugger::Debugger),
    Emulator(emulator::Emulator),
}

struct CurrentGame {
    entry: library::GameEntry,
    game_dir: PathBuf,
    cover: Option<iced::widget::image::Handle>,
    play_log: library::play_log::PlayLog,
    save_manifest: library::saves::SaveManifest,
}

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),

    // Navigation
    BackToLibrary,
    PlayFromDetail,
    BackToDetail,
    ShowSettings,
    ConfirmAction,
    DismissConfirm,

    // Game management (detail page actions)
    OpenGameFolder,
    RefreshMetadata,
    ImportSave,
    ImportSaveSelected(Option<rfd::FileHandle>),
    PlayWithSave(String),
    HoverLogEntry(usize),
    UnhoverLogEntry,
    RemoveGame,
    GameMetadataRefreshed(library::hasheous::GameInfo),

    // Emulation
    Run,
    Pause,
    Reset,
    SaveBattery,

    PressButton(joypad::Button),
    ReleaseButton(joypad::Button),

    ToggleDebugger(bool),
    SelectPalette(PaletteChoice),
    CompleteSetup { internet_enabled: bool },
    Settings(settings_view::Message),
    Library(library::view::Message),
    ScanComplete,
    EnrichComplete(bool),
    OpenUrl(&'static str),

    ToggleFullscreen,
    ExitFullscreen,
    MouseMoved,
    HideCursorTick,
    CloseRequested,

    StartRecording,
    StopRecording,
    StartPlayback,

    ActionBar(action_bar::Message),
    Debugger(debugger::Message),
    Emulator(emulator::Message),

    None,
}

impl App {
    fn new(rom_path: Option<PathBuf>, debugger: bool) -> (Self, Task<Message>) {
        let settings = settings::Settings::load();
        let recent_games = recent::RecentGames::load();

        let library_cache = library::view::LibraryCache::load();

        let mut app = Self {
            screen: Screen::Library,
            game: Game::Unloaded,
            debugger_enabled: debugger,
            fullscreen: Fullscreen::Windowed,
            action_bar: ActionBar::new(),
            audio_output: AudioOutput::new(),
            recent_games,
            settings,
            current_game: None,
            viewing_sha1: None,
            library_cache,
            pending_action: None,
            hovered_log_entry: None,
        };

        let mut tasks = Vec::new();

        if let Some(rom_path) = rom_path {
            if let Ok(rom) = fs::read(&rom_path) {
                tasks.push(load::setup_game(&mut app, rom_path, rom));
            }
        }

        // Scan configured ROM directories on startup
        if !app.settings.rom_directories.is_empty() {
            let dirs = app.settings.rom_directories.clone();
            tasks.push(Task::perform(
                smol::unblock(move || library::scanner::scan_directories(&dirs)),
                |_| Message::ScanComplete,
            ));
        } else if app.settings.internet_enabled {
            // No directories to scan, but still enrich any unenriched games
            tasks.push(Task::perform(
                smol::unblock(|| library::scanner::enrich_next()),
                |has_more| Message::EnrichComplete(has_more),
            ));
        }

        (app, Task::batch(tasks))
    }

    fn title(&self) -> String {
        if let Some(current) = &self.current_game {
            format!("{} - Missingno", current.entry.display_title())
        } else {
            "Missingno".into()
        }
    }

    fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Load(message) => return load::update(message, self),

            Message::BackToLibrary => {
                self.flush_pending_save();
                self.pause();
                self.screen = Screen::Library;
            }
            Message::ConfirmAction => {
                let action = self.pending_action.take();

                match action {
                    Some(PendingAction::ResetEmulator) => {
                        self.reset();
                    }
                    Some(PendingAction::SwitchGame(sha1)) => {
                        // Close current game
                        if let Some(current) = &mut self.current_game {
                            current.play_log.end_session();
                            library::play_log::save(&current.game_dir, &current.play_log);
                        }
                        self.game = Game::Unloaded;
                        self.current_game = None;

                        if load::select_game(self, &sha1) {
                            return load::play_current_game(self);
                        } else {
                            self.screen = Screen::Library;
                        }
                    }
                    Some(PendingAction::CloseApp) => {
                        if let Some(current) = &mut self.current_game {
                            current.play_log.end_session();
                            library::play_log::save(&current.game_dir, &current.play_log);
                        }
                        return window::latest().and_then(window::close);
                    }
                    None => {}
                }
            }
            Message::DismissConfirm => {
                self.pending_action = None;
            }

            // Game management
            Message::OpenGameFolder => {
                if let Some(sha1) = &self.viewing_sha1 {
                    if let Some(dir) = library::find_by_sha1(sha1).map(|(d, _)| d) {
                        let _ = open::that(&dir);
                    }
                }
            }
            Message::RefreshMetadata => {
                if let Some(sha1) = self.viewing_sha1.clone() {
                    return Task::perform(
                        smol::unblock(move || library::hasheous::lookup(&sha1).ok().flatten()),
                        move |info| {
                            if let Some(info) = info {
                                Message::GameMetadataRefreshed(info)
                            } else {
                                Message::None
                            }
                        },
                    );
                }
            }
            Message::GameMetadataRefreshed(info) => {
                if let Some(sha1) = &self.viewing_sha1 {
                    if let Some((game_dir, mut entry)) = library::find_by_sha1(sha1) {
                        entry.title = info.name;
                        entry.platform = info.platform;
                        entry.publisher = info.publisher;
                        entry.year = info.year;
                        entry.description = info.description;
                        entry.wikipedia_url = info.wikipedia_url;
                        entry.igdb_url = info.igdb_url;
                        entry.enrichment_attempted = true;
                        library::save_entry(&game_dir, &entry);
                        if let Some(bytes) = &info.cover_art {
                            library::save_cover(&game_dir, bytes);
                        }
                        self.library_cache = library::view::LibraryCache::load();
                    }
                }
            }
            Message::ImportSave => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter("Game Boy Save", &["sav"]);
                return Task::perform(dialog.pick_file(), |handle| {
                    Message::ImportSaveSelected(handle)
                });
            }
            Message::ImportSaveSelected(handle) => {
                if let (Some(handle), Some(sha1)) = (handle, &self.viewing_sha1) {
                    if let Some((game_dir, _)) = library::find_by_sha1(sha1) {
                        if let Ok(data) = std::fs::read(handle.path()) {
                            let mut manifest = library::saves::load_manifest(&game_dir);
                            let entry = manifest.record_import(data.len() as u32);
                            let id = entry.id.clone();
                            library::saves::write_save_data(&game_dir, &id, &data);
                            library::saves::save_manifest(&game_dir, &manifest);
                        }
                    }
                }
            }
            Message::PlayWithSave(save_id) => {
                // Launch the game with a specific save
                if let Some(sha1) = self.viewing_sha1.clone() {
                    let same_game = self.current_game.as_ref()
                        .map(|c| c.entry.sha1 == sha1)
                        .unwrap_or(false);

                    if matches!(self.game, Game::Loaded(_)) && !same_game {
                        // Different game loaded — would need confirmation
                        // For now, just go to the detail page
                    } else {
                        if !same_game || !matches!(self.game, Game::Loaded(_)) {
                            load::select_game(self, &sha1);
                        }
                        return load::play_with_save(self, &save_id);
                    }
                }
            }
            Message::HoverLogEntry(idx) => {
                self.hovered_log_entry = Some(idx);
            }
            Message::UnhoverLogEntry => {
                self.hovered_log_entry = None;
            }
            Message::RemoveGame => {
                if let Some(sha1) = &self.viewing_sha1 {
                    if let Some((game_dir, _)) = library::find_by_sha1(sha1) {
                        library::remove_game(&game_dir);
                        self.library_cache = library::view::LibraryCache::load();
                        self.viewing_sha1 = None;
                        self.screen = Screen::Library;
                    }
                }
            }

            Message::PlayFromDetail => {
                let viewing = self.viewing_sha1.clone();
                let same_game = viewing.as_ref().and_then(|sha1| {
                    self.current_game.as_ref().map(|c| c.entry.sha1 == *sha1)
                }).unwrap_or(false);

                if same_game {
                    // Resume the already-loaded game
                    self.run();
                    self.screen = Screen::Emulator;
                } else if matches!(self.game, Game::Loaded(_)) {
                    // Different game loaded, confirm switch
                    if let Some(sha1) = viewing {
                        self.pending_action = Some(PendingAction::SwitchGame(sha1));
                    }
                } else if let Some(sha1) = viewing {
                    // Nothing loaded, start the viewed game
                    load::select_game(self, &sha1);
                    return load::play_current_game(self);
                }
            }
            Message::BackToDetail => {
                self.flush_pending_save();
                self.pause();
                if let Some(current) = &self.current_game {
                    self.viewing_sha1 = Some(current.entry.sha1.clone());
                }
                self.screen = Screen::Detail;
            }
            Message::ShowSettings => {
                self.screen = Screen::Settings;
            }
            Message::SaveBattery => {
                self.save();
            }
            Message::Run => self.run(),
            Message::Pause => self.pause(),
            Message::Reset => {
                self.pending_action = Some(PendingAction::ResetEmulator);
            }

            Message::ToggleFullscreen => {
                let (new_fullscreen, mode) = match self.fullscreen {
                    Fullscreen::Windowed => (
                        Fullscreen::Active {
                            cursor_hidden: false,
                            last_mouse_move: Instant::now(),
                        },
                        window::Mode::Fullscreen,
                    ),
                    Fullscreen::Active { .. } => (Fullscreen::Windowed, window::Mode::Windowed),
                };
                self.fullscreen = new_fullscreen;
                return window::latest().and_then(move |id| window::set_mode(id, mode));
            }

            Message::ExitFullscreen => {
                if matches!(self.fullscreen, Fullscreen::Active { .. }) {
                    self.fullscreen = Fullscreen::Windowed;
                    return window::latest()
                        .and_then(|id| window::set_mode(id, window::Mode::Windowed));
                }
            }

            Message::MouseMoved => {
                if let Fullscreen::Active {
                    cursor_hidden,
                    last_mouse_move,
                } = &mut self.fullscreen
                {
                    *last_mouse_move = Instant::now();
                    *cursor_hidden = false;
                }
            }
            Message::HideCursorTick => {
                if let Fullscreen::Active {
                    cursor_hidden,
                    last_mouse_move,
                } = &mut self.fullscreen
                    && last_mouse_move.elapsed().as_secs() >= 2
                {
                    *cursor_hidden = true;
                }
            }

            Message::CloseRequested => {
                if matches!(self.game, Game::Loaded(_)) {
                    self.pending_action = Some(PendingAction::CloseApp);
                } else {
                    return window::latest().and_then(window::close);
                }
            }

            Message::PressButton(button) => self.press_button(button),
            Message::ReleaseButton(button) => self.release_button(button),

            Message::ToggleDebugger(debugger_enabled) => {
                self.debugger_enabled = debugger_enabled;

                if let Game::Loaded(game) = &mut self.game {
                    let palette = self.settings.palette;
                    replace_with_or_abort(game, |game| match game {
                        LoadedGame::Debugger(debugger) => {
                            if debugger_enabled {
                                LoadedGame::Debugger(debugger)
                            } else {
                                let mut emu = debugger.disable_debugger();
                                emu.set_palette(palette);
                                LoadedGame::Emulator(emu)
                            }
                        }
                        LoadedGame::Emulator(emulator) => {
                            if debugger_enabled {
                                let mut dbg = emulator.enable_debugger();
                                dbg.set_palette(palette);
                                LoadedGame::Debugger(dbg)
                            } else {
                                LoadedGame::Emulator(emulator)
                            }
                        }
                    });
                }
            }
            Message::SelectPalette(palette) => {
                self.settings.palette = palette;
                self.settings.save();
                match &mut self.game {
                    Game::Loaded(LoadedGame::Emulator(emulator)) => {
                        emulator.set_palette(palette);
                    }
                    Game::Loaded(LoadedGame::Debugger(debugger)) => {
                        debugger.set_palette(palette);
                    }
                    _ => {}
                }
            }
            Message::CompleteSetup { internet_enabled } => {
                self.settings.internet_enabled = internet_enabled;
                self.settings.setup_complete = true;
                self.settings.save();
            }
            Message::Settings(message) => match message {
                settings_view::Message::Back => {
                    self.screen = Screen::Library;
                }
                settings_view::Message::SetInternetEnabled(enabled) => {
                    self.settings.internet_enabled = enabled;
                    self.settings.save();
                }
                settings_view::Message::PickRomDirectory => {
                    let dialog = rfd::AsyncFileDialog::new();
                    return Task::perform(dialog.pick_folder(), |folder| {
                        match folder {
                            Some(handle) => {
                                let path = handle.path().to_path_buf();
                                settings_view::Message::AddRomDirectory(path).into()
                            }
                            None => Message::None,
                        }
                    });
                }
                settings_view::Message::AddRomDirectory(path) => {
                    if !self.settings.rom_directories.contains(&path) {
                        self.settings.rom_directories.push(path.clone());
                        self.settings.save();
                        let dirs = vec![path];
                        return Task::perform(
                            smol::unblock(move || library::scanner::scan_directories(&dirs)),
                            |_| Message::ScanComplete,
                        );
                    }
                }
                settings_view::Message::RemoveRomDirectory(index) => {
                    if index < self.settings.rom_directories.len() {
                        self.settings.rom_directories.remove(index);
                        self.settings.save();
                    }
                }
            },
            Message::Library(message) => match message {
                library::view::Message::SelectGame(sha1) => {
                    // Just view the detail page — doesn't touch the running game
                    self.viewing_sha1 = Some(sha1);
                    self.screen = Screen::Detail;
                }
                library::view::Message::QuickPlay(sha1) => {
                    let same_game = self.current_game.as_ref()
                        .map(|c| c.entry.sha1 == sha1)
                        .unwrap_or(false);

                    if same_game {
                        // Already loaded, just resume
                        self.run();
                        self.screen = Screen::Emulator;
                    } else if matches!(self.game, Game::Loaded(_)) {
                        // Different game loaded, confirm first
                        self.pending_action = Some(PendingAction::SwitchGame(sha1));
                    } else {
                        // Nothing loaded, go ahead
                        load::select_game(self, &sha1);
                        return load::play_current_game(self);
                    }
                }
            },
            Message::ScanComplete => {
                self.library_cache = library::view::LibraryCache::load();
                if self.settings.internet_enabled {
                    return Task::perform(
                        smol::unblock(|| library::scanner::enrich_next()),
                        |has_more| Message::EnrichComplete(has_more),
                    );
                }
            }
            Message::EnrichComplete(has_more) => {
                self.library_cache = library::view::LibraryCache::load();

                // Sync recent game titles with enriched library entries
                for cached in &self.library_cache.entries {
                    self.recent_games
                        .update_title(&cached.entry.sha1, &cached.entry.display_title());
                }
                self.recent_games.save();

                // Also update the current game if loaded
                if let Some(current) = &mut self.current_game {
                    if let Some((_dir, entry)) = library::find_by_sha1(&current.entry.sha1) {
                        current.entry = entry;
                        current.cover = library::load_thumbnail(&current.game_dir)
                            .map(|bytes| iced::widget::image::Handle::from_bytes(bytes));
                    }
                }

                // Chain: enrich next game if there are more
                if has_more {
                    return Task::perform(
                        smol::unblock(|| library::scanner::enrich_next()),
                        |has_more| Message::EnrichComplete(has_more),
                    );
                }
            }
            Message::OpenUrl(url) => {
                let _ = open::that(url);
            }

            Message::StartRecording => {
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    debugger.start_recording();
                }
            }
            Message::StopRecording => {
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    debugger.stop_recording();
                }
            }
            Message::StartPlayback => {
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    debugger.start_playback();
                }
            }

            Message::ActionBar(message) => return self.action_bar.update(message),
            Message::Emulator(message) => {
                if let Game::Loaded(LoadedGame::Emulator(emulator)) = &mut self.game {
                    let task = emulator.update(message);
                    self.drain_audio();
                    return task;
                }
            }

            Message::Debugger(message) => {
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    let task = debugger.update(message);
                    self.drain_audio();
                    return task;
                }
            }

            Message::None => {}
        }

        return Task::none();
    }

    fn view(&self) -> Element<'_, Message> {
        // Fullscreen emulator bypasses the normal chrome
        if self.screen == Screen::Emulator {
            if let Fullscreen::Active { cursor_hidden, .. } = self.fullscreen {
                let content = container(self.emulator_view(true))
                    .center(Fill)
                    .style(|_| container::Style {
                        background: Some(iced::Color::BLACK.into()),
                        ..Default::default()
                    });

                let mut area = mouse_area(content).on_move(|_| Message::MouseMoved);
                if cursor_hidden {
                    area = area.interaction(mouse::Interaction::Hidden);
                }
                return area.into();
            }
        }

        // Settings has its own full layout
        if self.screen == Screen::Settings {
            return settings_view::view(&self.settings);
        }

        // First-boot setup
        if !self.settings.setup_complete {
            return self.setup_view();
        }

        // Standard layout: action bar + content
        let content = match self.screen {
            Screen::Library => library::view::view(&self.library_cache),
            Screen::Detail => self.detail_view(),
            Screen::Emulator => self.emulator_view(false),
            Screen::Settings => unreachable!(),
        };

        let main = column![
            self.action_bar.view(self),
            horizontal_rule(),
            container(content).center(Fill)
        ];

        if let Some(action) = &self.pending_action {
            let (prompt, confirm_label) = match action {
                PendingAction::SwitchGame(_) => ("Close the current game and switch?", "Close Game"),
                PendingAction::CloseApp => ("Close the current game and quit?", "Quit"),
                PendingAction::ResetEmulator => ("Reset the emulator? Unsaved progress will be lost.", "Reset"),
            };

            let mut info = column![iced_text(prompt)].spacing(s());

            if let Some(current) = &self.current_game {
                info = info.push(
                    iced_text(current.entry.display_title())
                        .size(text::sizes::xl())
                        .font(fonts::heading()),
                );
                if let Some(last_save) = current.save_manifest.saves.last() {
                    info = info.push(
                        iced_text(format!("Last saved {}", friendly_ago(last_save.created)))
                            .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
                    );
                } else {
                    info = info.push(
                        iced_text("No saves")
                            .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
                    );
                }
            }

            Stack::new()
                .push(main)
                .push(opaque(
                    mouse_area(
                        center(
                            container(
                                column![
                                    info,
                                    row![
                                        buttons::standard("Cancel")
                                            .on_press(Message::DismissConfirm),
                                        buttons::danger(confirm_label)
                                            .on_press(Message::ConfirmAction),
                                    ]
                                    .spacing(s()),
                                ]
                                .spacing(l())
                                .align_x(Center),
                            )
                            .padding(l())
                            .style(container::bordered_box),
                        )
                        .style(|_| container::Style {
                            background: Some(
                                iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into(),
                            ),
                            ..Default::default()
                        }),
                    )
                    .on_press(Message::DismissConfirm),
                ))
                .into()
        } else {
            main.into()
        }
    }

    fn detail_view(&self) -> Element<'_, Message> {
        let viewing_sha1 = self.viewing_sha1.as_deref();

        // If viewing the running game, use CurrentGame data (has live play_log)
        if let Some(current) = &self.current_game {
            if viewing_sha1 == Some(current.entry.sha1.as_str()) {
                return library::detail_view::view(library::detail_view::DetailData {
                    entry: &current.entry,
                    cover: current.cover.as_ref(),
                    play_log: Some(current.play_log.clone()),
                    save_manifest: Some(current.save_manifest.clone()),
                    is_running: matches!(self.game, Game::Loaded(_)),
                    hovered_log_entry: self.hovered_log_entry,
                });
            }
        }

        // Otherwise look up from the library cache
        if let Some(sha1) = viewing_sha1 {
            if let Some(cached) = self.library_cache.entries.iter().find(|g| g.entry.sha1 == sha1) {
                // Use find_by_sha1 to get the actual directory path
                let game_dir = library::find_by_sha1(sha1).map(|(d, _)| d);
                let play_log = game_dir.as_ref().map(|d| library::play_log::load(d));
                let save_manifest = game_dir.as_ref().map(|d| library::saves::load_manifest(d));
                return library::detail_view::view(library::detail_view::DetailData {
                    entry: &cached.entry,
                    cover: cached.cover.as_ref(),
                    play_log,
                    save_manifest,
                    is_running: false,
                    hovered_log_entry: self.hovered_log_entry,
                });
            }
        }

        // Fallback
        library::view::view(&self.library_cache)
    }

    fn emulator_view(&self, fullscreen: bool) -> Element<'_, Message> {
        match &self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.view(),
                LoadedGame::Emulator(emulator) => emulator.view(fullscreen),
            },
            _ => text::label("No game loaded").into(),
        }
    }

    fn setup_view(&self) -> Element<'_, Message> {
        container(
            column![
                icons::xl(Icon::GameBoy)
                    .width(120)
                    .height(120)
                    .style(|_, _| svg::Style { color: None }),
                text::heading("Welcome to Missingno"),
                column![
                    iced_text("Missingno can connect to the internet to look up game metadata, cover art, and manuals for your games."),
                    iced_text("No data about your games or usage is sent — only ROM checksums are used for identification."),
                    iced_text("You can change this anytime in Settings."),
                ]
                .spacing(s())
                .max_width(420),
                row![
                    buttons::standard("Stay offline")
                        .on_press(Message::CompleteSetup { internet_enabled: false }),
                    buttons::primary("Enable internet features")
                        .on_press(Message::CompleteSetup { internet_enabled: true }),
                ]
                .spacing(s()),
            ]
            .align_x(Center)
            .spacing(l()),
        )
        .center(Fill)
        .into()
    }

    fn drain_audio(&mut self) {
        let game_boy = match &mut self.game {
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.game_boy_mut(),
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.game_boy_mut(),
            _ => return,
        };
        let samples = game_boy.drain_audio_samples();
        if let Some(audio) = &mut self.audio_output {
            audio.push_samples(&samples);
        }
    }

    pub fn running(&self) -> bool {
        match &self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.running(),
                LoadedGame::Emulator(emulator) => emulator.running(),
            },
            _ => false,
        }
    }

    pub fn sgb_active(&self) -> bool {
        match &self.game {
            Game::Loaded(game) => {
                let gb = match game {
                    LoadedGame::Debugger(debugger) => debugger.game_boy(),
                    LoadedGame::Emulator(emulator) => emulator.game_boy(),
                };
                gb.sgb().is_some()
            }
            _ => false,
        }
    }

    fn run(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.run(),
                LoadedGame::Emulator(emulator) => emulator.run(),
            },
            _ => {}
        }
    }

    fn pause(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.pause(),
                LoadedGame::Emulator(emulator) => emulator.pause(),
            },
            _ => {}
        }
    }

    fn reset(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.reset(),
                LoadedGame::Emulator(emulator) => emulator.reset(),
            },
            _ => {}
        }
    }

    fn press_button(&mut self, button: Button) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.press_button(button),
                LoadedGame::Emulator(emulator) => emulator.press_button(button),
            },
            _ => {}
        }
    }

    fn release_button(&mut self, button: Button) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.release_button(button),
                LoadedGame::Emulator(emulator) => emulator.release_button(button),
            },
            _ => {}
        }
    }

    /// Flush any debounced SRAM save from the emulator.
    fn flush_pending_save(&mut self) {
        let flushed = match &mut self.game {
            Game::Loaded(LoadedGame::Emulator(emu)) => emu.flush_pending_save(),
            _ => false,
        };
        if flushed {
            self.save();
        }
    }

    fn save(&mut self) {
        let ram = match &self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                if !debugger.game_boy().cartridge().has_battery() { return; }
                debugger.game_boy().cartridge().ram()
            }
            Game::Loaded(LoadedGame::Emulator(emulator)) => {
                if !emulator.game_boy().cartridge().has_battery() { return; }
                emulator.game_boy().cartridge().ram()
            }
            _ => return,
        };
        let Some(ram) = ram else { return };
        let Some(current) = &mut self.current_game else { return };

        let session_index = if current.play_log.sessions.is_empty() {
            None
        } else {
            Some(current.play_log.sessions.len() - 1)
        };

        let entry = current.save_manifest.record_emulation_save(
            ram.len() as u32,
            session_index,
        );
        let id = entry.id.clone();
        library::saves::write_save_data(&current.game_dir, &id, &ram);
        library::saves::save_manifest(&current.game_dir, &current.save_manifest);
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            if self.running() {
                event::listen_with(controls::event_handler)
            } else {
                Subscription::none()
            },
            if self.running() {
                controls::gamepad_subscription()
            } else {
                Subscription::none()
            },
            if matches!(self.fullscreen, Fullscreen::Active { .. }) {
                time::every(std::time::Duration::from_millis(500)).map(|_| Message::HideCursorTick)
            } else {
                Subscription::none()
            },
            event::listen_with(|event, _, _| match event {
                iced::Event::Window(window::Event::CloseRequested) => Some(Message::CloseRequested),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::F11),
                    ..
                }) => Some(Message::ToggleFullscreen),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::ExitFullscreen),
                _ => None,
            }),
            match &self.game {
                Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.subscription(),
                Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.subscription(),
                _ => Subscription::none(),
            },
        ])
    }
}

fn friendly_ago(timestamp: jiff::Timestamp) -> String {
    let secs = jiff::Timestamp::now().duration_since(timestamp).as_secs();
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs} seconds ago")
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 { "1 minute ago".to_string() } else { format!("{mins} minutes ago") }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 { "1 hour ago".to_string() } else { format!("{hours} hours ago") }
    } else {
        let days = secs / 86400;
        if days == 1 { "yesterday".to_string() } else { format!("{days} days ago") }
    }
}
