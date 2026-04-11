use std::{fs, path::PathBuf, time::Instant};

use iced::{
    Task, Theme,
    window,
};
use action_bar::ActionBar;
use audio_output::AudioOutput;
use ui::fonts;
use missingno_gb::joypad;

mod action_bar;
mod audio_output;
mod controls;
mod ui;
mod debugger;
mod emulation;
mod emulator;
pub mod library;
mod load;
mod recent;
mod screen;
pub mod settings;
mod texture_renderer;
mod views;

// Cartridge reader/writer hardware support
use crate::cartridge_rw;

pub fn run(
    rom_path: Option<PathBuf>,
    debugger: bool,
    link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
) -> iced::Result {
    // Load settings early to get saved window size
    let saved = settings::Settings::load();
    let window_width = saved.window_width.unwrap_or(1280.0);
    let window_height = saved.window_height.unwrap_or(720.0);

    // Wrap in a Cell so the non-Clone link can be taken from the FnMut closure.
    let link_cell = std::cell::Cell::new(link);
    let mut app = iced::application(
        move || App::new(rom_path.clone(), debugger, link_cell.take()),
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
        size: iced::Size::new(window_width, window_height),
        min_size: Some(iced::Size::new(1000.0, 700.0)),
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
    store: library::store::GameStore,
    /// Action waiting for user confirmation (e.g. close game before launching another).
    pending_action: Option<PendingAction>,
    /// When set, shows a brief "Screenshot saved" toast overlay.
    screenshot_toast: Option<Instant>,
    /// Serial link cable connection (BGB link protocol), injected into GameBoy on load.
    serial_link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
    /// Homebrew Hub API client (shared, thread-safe).
    homebrew_client: std::sync::Arc<library::homebrew_hub::HomebrewHubClient>,
    /// Bundled game catalogue (commercial + homebrew).
    catalogue: std::sync::Arc<library::catalogue::Catalogue>,
    /// Cartridge reader/writer devices detected on the system.
    detected_cartridge_devices: Vec<cartridge_rw::DetectedDevice>,
    /// Last-seen port names for cartridge RW polling (to detect changes cheaply).
    cartridge_rw_known_ports: Vec<String>,
    /// Progress of an active ROM dump, if any.
    cartridge_dump_progress: Option<cartridge_rw::DumpProgress>,
    /// Whether the hamburger menu overlay is open.
    menu_open: bool,
}

impl App {
    /// Get the SHA1 of the game being viewed, if on a detail/sub-screen.
    fn viewing_sha1(&self) -> Option<&str> {
        match &self.screen {
            Screen::ViewingGame { sha1, .. } => Some(sha1),
            _ => None,
        }
    }

    /// Get the keybinding capture state, if on the settings screen.
    fn listening_for(&self) -> Option<settings::view::ListeningFor> {
        match &self.screen {
            Screen::Settings { listening_for, .. } => *listening_for,
            _ => None,
        }
    }

}

#[derive(Debug, Clone)]
pub(crate) enum FlashState {
    /// Confirming with the user before flashing.
    Confirming {
        sha1: String,
        game_title: String,
        rom_size: u32,
        cart_title: String,
        flash_size: u32,
        has_save: bool,
        write_save: bool,
    },
    /// Flash in progress.
    InProgress(cartridge_rw::FlashProgress),
    /// Flash completed successfully.
    Complete,
    /// Flash failed.
    Failed(String),
}

#[derive(Debug, Clone)]
enum PendingAction {
    /// User wants to launch a different game — close current first.
    SwitchGame(String),
    /// User wants to close the app.
    CloseApp,
    /// User wants to reset the emulator.
    ResetEmulator,
    /// User wants to stop and unload the game.
    StopGame,
    /// User wants to remove the game from the library.
    RemoveGameFromLibrary,
}

enum Screen {
    Library {
        hovered_game: Option<String>,
    },
    ViewingGame {
        sha1: String,
        sub_screen: DetailSubScreen,
    },
    HomebrewBrowser {
        state: library::homebrew_browser::BrowserState,
    },
    Settings {
        section: settings::view::Section,
        listening_for: Option<settings::view::ListeningFor>,
        previous_screen: Box<Screen>,
        was_running: bool,
    },
    Emulator,
}

enum DetailSubScreen {
    Detail {
        hovered_log_entry: Option<usize>,
        header_hovered: bool,
    },
    CartridgeActions {
        #[allow(dead_code)] // Used by the upcoming flash flow
        flash_write_save: bool,
    },
    FlashCartridge {
        flash_state: FlashState,
    },
    ScreenshotGallery {
        gallery_state: library::screenshot_gallery::GalleryState,
    },
}

/// Messages specific to the game detail screen.
#[derive(Debug, Clone)]
enum DetailMessage {
    HoverLogEntry(usize),
    UnhoverLogEntry,
    HoverHeader,
    UnhoverHeader,
    OpenGameFolder,
    RefreshMetadata,
    ImportSave,
    ImportSaveSelected(Option<rfd::FileHandle>),
    PlayWithSave(String),
    ExportSave(String),
    ExportSaveSelected(String, Option<rfd::FileHandle>),
    OpenScreenshotGallery(String, usize),
    RemoveGame,
    GameMetadataRefreshed(library::hasheous::GameInfo),
}

/// Messages specific to cartridge operations.
#[derive(Debug, Clone)]
enum CartridgeMessage {
    ShowActions(String),
    Back,
    ImportSave,
    ImportSaveComplete(Result<Vec<u8>, String>),
    WriteSave,
    WriteSaveComplete(Result<Vec<u8>, String>),
    Flash(String),
    FlashConfirm,
    FlashCancel,
    FlashToggleSave(bool),
    FlashProgress(cartridge_rw::FlashProgress),
    FlashComplete(Result<Option<Vec<u8>>, String>),
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
    /// The in-progress session, written incrementally to disk.
    session: Option<library::activity::SessionFile>,
    /// Which activity file we started from (for parent tracking).
    started_from: Option<String>,
    /// SRAM snapshot at session start, for detecting meaningful changes.
    initial_sram: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),

    // Navigation
    BackToLibrary,
    PlayFromDetail,
    BackToDetail,
    StopGame,
    ShowSettings,
    ConfirmAction,
    DismissConfirm,

    // Screen-specific messages
    Detail(DetailMessage),
    Cartridge(CartridgeMessage),
    OpenHomebrewBrowser,
    HomebrewBrowser(library::homebrew_browser::Message),
    HomebrewDownloaded(String, Vec<u8>, library::catalogue::GameManifest),
    ScreenshotGallery(library::screenshot_gallery::Message),

    // Emulation
    Run,
    Pause,
    TogglePause,
    Reset,
    SaveBattery,
    TakeScreenshot,

    PressButton(joypad::Button),
    ReleaseButton(joypad::Button),

    ToggleDebugger(bool),
    CompleteSetup { internet_enabled: bool },
    Settings(settings::view::Message),
    Library(library::view::Message),
    ScanComplete(bool),
    ActivityLoaded(library::store::RawActivityDetail),
    EnrichComplete(library::scanner::EnrichResult),
    OpenUrl(&'static str),

    WindowResized(iced::Size),
    ToggleFullscreen,
    ExitFullscreen,
    MouseMoved,
    HideCursorTick,
    CloseRequested,

    StartRecording,
    StopRecording,
    StartPlayback,

    DismissScreenshotToast,

    // Cartridge reader/writer (device-level, not screen-specific)
    CartridgeRwPoll,
    CartridgeRwPortsChanged(Vec<cartridge_rw::DetectedDevice>),
    CartridgeRwDumpProgress(cartridge_rw::DumpProgress),
    CartridgeRwDumpComplete(Result<(Vec<u8>, Option<Vec<u8>>), String>),

    ToggleMenu,
    DismissMenu,
    /// A menu item was clicked — dismiss the menu and execute the inner message.
    MenuAction(Box<Message>),

    ActionBar(action_bar::Message),
    Debugger(debugger::Message),
    Emulator(emulator::Message),

    None,
}

impl App {
    fn new(
        rom_path: Option<PathBuf>,
        debugger: bool,
        serial_link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
    ) -> (Self, Task<Message>) {
        let settings = settings::Settings::load();
        let recent_games = recent::RecentGames::load();

        let store = library::store::GameStore::new();

        let mut app = Self {
            screen: Screen::Library { hovered_game: None },
            game: Game::Unloaded,
            debugger_enabled: debugger,
            fullscreen: Fullscreen::Windowed,
            action_bar: ActionBar::new(),
            audio_output: AudioOutput::new(),
            recent_games,
            settings,
            current_game: None,
            store,
            pending_action: None,
            screenshot_toast: None,
            serial_link,
            homebrew_client: std::sync::Arc::new(library::homebrew_hub::HomebrewHubClient::new()),
            catalogue: std::sync::Arc::new(library::catalogue::Catalogue::load()),
            detected_cartridge_devices: Vec::new(),
            cartridge_rw_known_ports: Vec::new(),
            cartridge_dump_progress: None,
            menu_open: false,
        };

        controls::update_bindings(
            &app.settings.keyboard_bindings,
            &app.settings.gamepad_bindings,
        );

        let mut tasks = Vec::new();

        if let Some(rom_path) = rom_path {
            if let Ok(rom) = fs::read(&rom_path) {
                tasks.push(load::setup_game(&mut app, rom_path, rom));
            }
        }

        // Scan configured ROM directories on startup
        if !app.settings.rom_directories.is_empty() {
            let dirs = app.settings.rom_directories.clone();
            let cat = app.catalogue.clone();
            tasks.push(Task::perform(
                smol::unblock(move || library::scanner::scan_directories(&dirs, &cat)),
                |entries| Message::ScanComplete(!entries.is_empty()),
            ));
        } else if app.settings.internet_enabled && app.settings.hasheous_enabled {
            // No directories to scan, but still enrich any unenriched games
            tasks.push(Task::perform(
                smol::unblock(|| library::scanner::enrich_next()),
                |result| Message::EnrichComplete(result),
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

            // Emulation messages
            Message::Run | Message::Pause | Message::TogglePause | Message::Reset
            | Message::SaveBattery | Message::TakeScreenshot | Message::DismissScreenshotToast
            | Message::PressButton(_) | Message::ReleaseButton(_) | Message::ToggleDebugger(_)
            | Message::StartRecording | Message::StopRecording | Message::StartPlayback
                => return self.handle_emulation_message(message),

            // Settings messages
            Message::CompleteSetup { internet_enabled } => {
                self.settings.internet_enabled = internet_enabled;
                self.settings.setup_complete = true;
                self.settings.save();
            }
            Message::Settings(message) => return settings::update::handle(self, message),

            // Library messages
            Message::Library(message) => return library::update::handle_library_message(self, message),
            Message::Detail(msg) => return library::update::handle(self, Message::Detail(msg)),
            Message::Cartridge(msg) => return library::update::handle(self, Message::Cartridge(msg)),
            Message::HomebrewDownloaded(..) | Message::OpenHomebrewBrowser
            | Message::HomebrewBrowser(_) | Message::ScreenshotGallery(_)
            | Message::ActivityLoaded(_)
            | Message::ScanComplete(_) | Message::EnrichComplete(_) | Message::OpenUrl(_)
            | Message::CartridgeRwDumpProgress(_) | Message::CartridgeRwDumpComplete(_)
                => return library::update::handle(self, message),

            // Navigation
            Message::BackToLibrary => {
                self.menu_open = false;
                self.flush_pending_save();
                self.pause();
                self.screen = Screen::Library { hovered_game: None };
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
                            if let Some(session) = &mut current.session {
                                session.end = Some(jiff::Timestamp::now());
                                library::activity::write_session(&current.game_dir, session);
                            }
                        }
                        self.game = Game::Unloaded;
                        self.current_game = None;

                        if load::select_game(self, &sha1) {
                            return load::play_current_game(self);
                        } else {
                            self.screen = Screen::Library { hovered_game: None };
                        }
                    }
                    Some(PendingAction::StopGame) => {
                        let sha1 = if let Some(current) = &mut self.current_game {
                            if let Some(session) = &mut current.session {
                                session.end = Some(jiff::Timestamp::now());
                                library::activity::write_session(&current.game_dir, session);
                            }
                            self.store.notify_activity_changed(&current.entry.sha1);
                            Some(current.entry.sha1.clone())
                        } else {
                            None
                        };
                        self.game = Game::Unloaded;
                        self.current_game = None;
                        if let Some(sha1) = sha1 {
                            return self.go_to_detail(&sha1);
                        }
                    }
                    Some(PendingAction::RemoveGameFromLibrary) => {
                        if let Some(sha1) = self.viewing_sha1().map(|s| s.to_string()) {
                            if let Some((game_dir, _)) = library::find_by_sha1(&sha1) {
                                library::remove_game(&game_dir);
                            }
                            self.store.notify_game_removed(&sha1);
                        }
                        self.screen = Screen::Library { hovered_game: None };
                    }
                    Some(PendingAction::CloseApp) => {
                        if let Some(current) = &mut self.current_game {
                            if let Some(session) = &mut current.session {
                                session.end = Some(jiff::Timestamp::now());
                                library::activity::write_session(&current.game_dir, session);
                            }
                        }
                        return window::latest().and_then(window::close);
                    }
                    None => {}
                }
            }
            Message::DismissConfirm => {
                self.pending_action = None;
            }
            Message::PlayFromDetail => {
                self.menu_open = false;
                let viewing = self.viewing_sha1().map(|s| s.to_string());
                let same_game = viewing
                    .as_ref()
                    .and_then(|sha1| self.current_game.as_ref().map(|c| c.entry.sha1 == *sha1))
                    .unwrap_or(false);

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
            Message::StopGame => {
                self.pending_action = Some(PendingAction::StopGame);
            }
            Message::BackToDetail => {
                self.flush_pending_save();
                self.pause();
                if let Some(current) = &self.current_game {
                    let sha1 = current.entry.sha1.clone();
                    self.store.notify_activity_changed(&sha1);
                    return self.go_to_detail(&sha1);
                }
            }
            Message::ShowSettings => {
                self.menu_open = false;
                let was_running = self.running();
                self.pause();
                let previous = std::mem::replace(&mut self.screen, Screen::Library { hovered_game: None });
                self.screen = Screen::Settings {
                    section: settings::view::Section::default(),
                    listening_for: None,
                    previous_screen: Box::new(previous),
                    was_running,
                };
            }
            Message::ToggleMenu => {
                self.menu_open = !self.menu_open;
            }
            Message::DismissMenu => {
                self.menu_open = false;
            }
            Message::MenuAction(inner) => {
                self.menu_open = false;
                return self.update(*inner);
            }

            // Window management
            Message::WindowResized(size) => {
                if !matches!(self.fullscreen, Fullscreen::Active { .. }) {
                    self.settings.window_width = Some(size.width);
                    self.settings.window_height = Some(size.height);
                }
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
                self.settings.save(); // persist window size
                if matches!(self.game, Game::Loaded(_)) {
                    self.pending_action = Some(PendingAction::CloseApp);
                } else {
                    return window::latest().and_then(window::close);
                }
            }

            // Cartridge RW polling (stays here — not library-specific)
            Message::CartridgeRwPoll => {
                let ports = cartridge_rw::list_ports();
                if ports != self.cartridge_rw_known_ports {
                    // Find which ports are new (need querying)
                    let new_ports: Vec<String> = ports
                        .iter()
                        .filter(|p| !self.cartridge_rw_known_ports.contains(p))
                        .cloned()
                        .collect();

                    // Remove devices on ports that disappeared
                    self.detected_cartridge_devices
                        .retain(|d| ports.contains(&d.port_name));

                    self.cartridge_rw_known_ports = ports;

                    // Only query newly appeared ports
                    if !new_ports.is_empty() {
                        return Task::perform(
                            smol::unblock(move || cartridge_rw::detect_ports(&new_ports)),
                            Message::CartridgeRwPortsChanged,
                        );
                    }
                }
            }
            Message::CartridgeRwPortsChanged(new_devices) => {
                // Merge newly detected devices into the list
                for device in new_devices {
                    if !self
                        .detected_cartridge_devices
                        .iter()
                        .any(|d| d.port_name == device.port_name)
                    {
                        self.detected_cartridge_devices.push(device);
                    }
                }
            }

            // Delegated subsystems
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
}
