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
use missingno_gb::joypad::{self, Button};

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
    /// SHA1 of the game being viewed in the detail page (may differ from current_game).
    viewing_sha1: Option<String>,
    store: library::store::GameStore,
    /// Action waiting for user confirmation (e.g. close game before launching another).
    pending_action: Option<PendingAction>,
    /// Index of the activity log entry currently hovered on the detail page.
    hovered_log_entry: Option<usize>,
    /// Whether the game header is hovered (to show secondary actions).
    header_hovered: bool,
    /// SHA1 of the game card currently hovered in the library.
    hovered_library_game: Option<String>,
    settings_section: settings_view::Section,
    /// Screen to return to when leaving settings.
    previous_screen: Option<Screen>,
    /// Whether the game was running before entering settings.
    was_running_before_settings: bool,
    /// Keybinding capture state: which slot we're listening for input on.
    listening_for: Option<settings_view::ListeningFor>,
    /// When set, shows a brief "Screenshot saved" toast overlay.
    screenshot_toast: Option<Instant>,
    /// Serial link cable connection (BGB link protocol), injected into GameBoy on load.
    serial_link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
    /// Screenshot gallery state (when viewing screenshots).
    gallery_state: Option<library::screenshot_gallery::GalleryState>,
    /// Homebrew browser state.
    homebrew_browser: Option<library::homebrew_browser::BrowserState>,
    /// Homebrew Hub API client (shared, thread-safe).
    homebrew_client: std::sync::Arc<library::homebrew_hub::HomebrewHubClient>,
    /// Bundled game catalogue (commercial + homebrew).
    catalogue: std::sync::Arc<library::catalogue::Catalogue>,
    /// Cartridge reader/writer devices detected on the system.
    detected_cartridge_devices: Vec<cartridge_rw::DetectedDevice>,
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Library,
    Settings,
    Detail,
    ScreenshotGallery,
    HomebrewBrowser,
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

    // Game management (detail page actions)
    OpenGameFolder,
    RefreshMetadata,
    ImportSave,
    ImportSaveSelected(Option<rfd::FileHandle>),
    PlayWithSave(String),
    ExportSave(String),
    ExportSaveSelected(String, Option<rfd::FileHandle>),
    OpenScreenshotGallery(String, usize), // (session filename, screenshot index)
    OpenHomebrewBrowser,
    HomebrewBrowser(library::homebrew_browser::Message),
    HomebrewDownloaded(String, Vec<u8>, library::catalogue::GameManifest),
    ScreenshotGallery(library::screenshot_gallery::Message),
    HoverLogEntry(usize),
    UnhoverLogEntry,
    HoverHeader,
    UnhoverHeader,
    RemoveGame,
    GameMetadataRefreshed(library::hasheous::GameInfo),

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
    Settings(settings_view::Message),
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
            store,
            pending_action: None,
            hovered_log_entry: None,
            header_hovered: false,
            hovered_library_game: None,
            settings_section: settings_view::Section::default(),
            previous_screen: None,
            was_running_before_settings: false,
            listening_for: None,
            screenshot_toast: None,
            serial_link,
            gallery_state: None,
            homebrew_browser: None,
            homebrew_client: std::sync::Arc::new(library::homebrew_hub::HomebrewHubClient::new()),
            catalogue: std::sync::Arc::new(library::catalogue::Catalogue::load()),
            detected_cartridge_devices: Vec::new(),
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
                            self.screen = Screen::Library;
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
                        if let Some(sha1) = &self.viewing_sha1 {
                            if let Some((game_dir, _)) = library::find_by_sha1(sha1) {
                                library::remove_game(&game_dir);
                            }
                            self.store.notify_game_removed(sha1);
                        }
                        self.viewing_sha1 = None;
                        self.screen = Screen::Library;
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
                        self.store.notify_metadata_changed(sha1);
                    }
                }
            }
            Message::ImportSave => {
                let dialog = rfd::AsyncFileDialog::new().add_filter("Game Boy Save", &["sav"]);
                return Task::perform(dialog.pick_file(), |handle| {
                    Message::ImportSaveSelected(handle)
                });
            }
            Message::ImportSaveSelected(handle) => {
                if let (Some(handle), Some(sha1)) = (handle, &self.viewing_sha1) {
                    if let Some((game_dir, _)) = library::find_by_sha1(sha1) {
                        if let Ok(data) = std::fs::read(handle.path()) {
                            library::activity::write_import(&game_dir, &data);
                        }
                    }
                }
            }
            Message::PlayWithSave(save_id) => {
                // Launch the game with a specific save
                if let Some(sha1) = self.viewing_sha1.clone() {
                    let same_game = self
                        .current_game
                        .as_ref()
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
            Message::ExportSave(save_id) => {
                let dialog = rfd::AsyncFileDialog::new()
                    .set_file_name("save.sav")
                    .add_filter("Game Boy Save", &["sav"]);
                return Task::perform(dialog.save_file(), move |handle| {
                    Message::ExportSaveSelected(save_id.clone(), handle)
                });
            }
            Message::ExportSaveSelected(save_id, handle) => {
                if let (Some(handle), Some(sha1)) = (handle, &self.viewing_sha1) {
                    if let Some((game_dir, _)) = library::find_by_sha1(sha1) {
                        if let Some(data) = library::activity::load_sram_from(&game_dir, &save_id) {
                            let _ = std::fs::write(handle.path(), data);
                        }
                    }
                }
            }
            Message::OpenScreenshotGallery(session_filename, screenshot_idx) => {
                if let Some(sha1) = &self.viewing_sha1 {
                    if let Some((game_dir, _)) = library::find_by_sha1(sha1) {
                        if let Some(mut state) = library::screenshot_gallery::GalleryState::load(
                            &game_dir,
                            &session_filename,
                        ) {
                            state.select(screenshot_idx);
                            self.gallery_state = Some(state);
                            self.screen = Screen::ScreenshotGallery;
                        }
                    }
                }
            }
            Message::ScreenshotGallery(msg) => {
                use library::screenshot_gallery::Message as G;
                match msg {
                    G::SelectScreenshot(idx) => {
                        if let Some(state) = &mut self.gallery_state {
                            state.select(idx);
                        }
                    }
                    G::SetPalette(pal) => {
                        if let Some(state) = &mut self.gallery_state {
                            state.palette = pal;
                        }
                    }
                    G::SetScale(scale) => {
                        if let Some(state) = &mut self.gallery_state {
                            state.scale = scale;
                        }
                    }
                    G::Export => {
                        let dialog = rfd::AsyncFileDialog::new()
                            .set_file_name("screenshot.png")
                            .add_filter("PNG Image", &["png"]);
                        return Task::perform(dialog.save_file(), |handle| {
                            Message::ScreenshotGallery(G::ExportSelected(handle))
                        });
                    }
                    G::ExportSelected(handle) => {
                        if let (Some(handle), Some(state)) = (handle, &self.gallery_state) {
                            let rgba = state.selected_rgba();
                            let width = 160 * state.scale;
                            let height = 144 * state.scale;
                            let scaled = library::screenshot_gallery::scale_nearest_neighbour(
                                &rgba,
                                160,
                                144,
                                state.scale,
                            );
                            if let Some(img) = image::RgbaImage::from_raw(width, height, scaled) {
                                let _ = img.save(handle.path());
                            }
                        }
                    }
                    G::Back => {
                        self.gallery_state = None;
                        if let Some(sha1) = self.viewing_sha1.clone() {
                            return self.go_to_detail(&sha1);
                        }
                    }
                }
            }
            Message::HomebrewDownloaded(title, rom_bytes, manifest) => {
                let sha1 = library::hasheous::rom_sha1(&rom_bytes);

                // Check if already in library
                if self.store.entry(&sha1).is_some() {
                    eprintln!("[homebrew] {title} already in library");
                    return Task::none();
                }

                let Some(game_dir) = library::game_dir_for(&title, &sha1) else {
                    return Task::none();
                };
                if let Err(e) = std::fs::create_dir_all(&game_dir) {
                    eprintln!("[homebrew] Failed to create game dir: {e}");
                }

                // Get filename from source
                let filename = match &manifest.source {
                    Some(library::catalogue::GameSource::HomebrewHub { filename, .. }) => {
                        filename.clone()
                    }
                    _ => format!("{}.gb", title.to_lowercase().replace(' ', "-")),
                };
                let filename = std::path::Path::new(&filename)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or(filename);
                let rom_path = game_dir.join(&filename);
                eprintln!(
                    "[homebrew] Saving {} bytes to {}",
                    rom_bytes.len(),
                    rom_path.display()
                );
                if let Err(e) = std::fs::write(&rom_path, &rom_bytes) {
                    eprintln!("[homebrew] Failed to write ROM: {e}");
                }

                // Create library entry
                let mut entry = library::GameEntry::new(sha1.clone(), title, rom_path);
                entry.platform = Some("Nintendo Game Boy".to_string());
                entry.year = manifest.date.clone();
                entry.description = manifest.description.clone();
                entry.publisher = manifest.developer.clone();
                library::save_entry(&game_dir, &entry);
                // Use cached cover bytes from the browser if available
                let slug = match &manifest.source {
                    Some(library::catalogue::GameSource::HomebrewHub { slug, .. }) => {
                        Some(slug.clone())
                    }
                    _ => None,
                };
                let cached_cover = slug.as_ref().and_then(|s| {
                    self.homebrew_browser
                        .as_ref()
                        .and_then(|b| b.cover_bytes.get(s).cloned())
                });

                if let Some(bytes) = &cached_cover {
                    library::save_cover(&game_dir, bytes);
                }

                self.store.notify_game_added(&sha1, game_dir.clone());
                self.homebrew_browser = None;
                let detail_task = self.go_to_detail(&sha1);

                // If no cached cover, download in background
                if cached_cover.is_none() {
                    if let Some(slug) = slug {
                        let cover_url = format!(
                            "https://raw.githubusercontent.com/gbdev/database/master/entries/{slug}/cover.png"
                        );
                        let client = self.homebrew_client.clone();
                        let gd = game_dir;
                        let sha1_clone = sha1;
                        let cover_task = Task::perform(
                            smol::unblock(move || {
                                if let Ok(bytes) = client.download_image(&cover_url) {
                                    library::save_cover(&gd, &bytes);
                                }
                                sha1_clone
                            }),
                            |sha1| {
                                Message::EnrichComplete(library::scanner::EnrichResult {
                                    sha1: Some(sha1),
                                    data_changed: true,
                                    has_more: false,
                                })
                            },
                        );
                        return Task::batch([detail_task, cover_task]);
                    }
                }
                return detail_task;
            }
            Message::OpenHomebrewBrowser => {
                self.homebrew_browser = Some(library::homebrew_browser::BrowserState::new());
                self.screen = Screen::HomebrewBrowser;
                // Load covers for the initial results
                return self.load_homebrew_covers();
            }
            Message::HomebrewBrowser(msg) => {
                use library::homebrew_browser::Message as H;
                match msg {
                    H::SearchTextChanged(text) => {
                        if let Some(state) = &mut self.homebrew_browser {
                            state.search_text = text;
                            state.visible_count = library::homebrew_browser::PAGE_SIZE;
                            state.error = None;
                        }
                    }
                    H::ShowMore => {
                        if let Some(state) = &mut self.homebrew_browser {
                            state.visible_count += library::homebrew_browser::PAGE_SIZE;
                            return self.load_homebrew_covers();
                        }
                    }
                    H::DownloadFailed(error) => {
                        if let Some(state) = &mut self.homebrew_browser {
                            state.error = Some(error);
                        }
                    }
                    H::DismissError => {
                        if let Some(state) = &mut self.homebrew_browser {
                            state.error = None;
                        }
                    }
                    H::CoverLoaded(slug, bytes) => {
                        if let Some(state) = &mut self.homebrew_browser {
                            state.covers.insert(
                                slug.clone(),
                                iced::widget::image::Handle::from_bytes(bytes.clone()),
                            );
                            state.cover_bytes.insert(slug, bytes);
                        }
                    }
                    H::SelectEntry(slug) => {
                        if let Some(state) = &mut self.homebrew_browser {
                            state.selected_slug = Some(slug.clone());

                            // Load cover image if not cached
                            if !state.covers.contains_key(&slug) {
                                if let Some(entry) = self.catalogue.lookup_slug(&slug) {
                                    if let Some(url) = entry.download_cover_url() {
                                        let client = self.homebrew_client.clone();
                                        let s = slug;
                                        return Task::perform(
                                            smol::unblock(move || {
                                                client
                                                    .download_image(&url)
                                                    .ok()
                                                    .map(|bytes| (s, bytes))
                                            }),
                                            |result| match result {
                                                Some((slug, bytes)) => Message::HomebrewBrowser(
                                                    H::CoverLoaded(slug, bytes),
                                                ),
                                                None => Message::None,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                    H::Download(slug) => {
                        if let Some(entry) = self.catalogue.lookup_slug(&slug) {
                            if let Some(url) = entry.download_url() {
                                let title = entry.manifest.title.clone();
                                let manifest = entry.manifest.clone();
                                return Task::perform(
                                    smol::unblock(move || {
                                        let response = ureq::get(&url)
                                            .call()
                                            .map_err(|e| format!("Download failed: {e}"))?;
                                        response
                                            .into_body()
                                            .read_to_vec()
                                            .map_err(|e| format!("Failed to read: {e}"))
                                            .map(|bytes| (title, bytes, manifest))
                                    }),
                                    |result| match result {
                                        Ok((title, rom_bytes, manifest)) => {
                                            Message::HomebrewDownloaded(title, rom_bytes, manifest)
                                        }
                                        Err(e) => {
                                            eprintln!("[homebrew] Download failed: {e}");
                                            Message::HomebrewBrowser(
                                                library::homebrew_browser::Message::DownloadFailed(
                                                    format!("Download failed: {e}"),
                                                ),
                                            )
                                        }
                                    },
                                );
                            }
                        }
                    }
                    H::Back => {
                        if let Some(state) = &mut self.homebrew_browser {
                            if state.selected_slug.is_some() {
                                // Back from detail to results
                                state.selected_slug = None;
                            } else {
                                // Back from results to library
                                self.homebrew_browser = None;
                                self.screen = Screen::Library;
                            }
                        }
                    }
                }
            }
            Message::HoverLogEntry(idx) => {
                self.hovered_log_entry = Some(idx);
            }
            Message::UnhoverLogEntry => {
                self.hovered_log_entry = None;
            }
            Message::HoverHeader => {
                self.header_hovered = true;
            }
            Message::UnhoverHeader => {
                self.header_hovered = false;
            }
            Message::RemoveGame => {
                self.pending_action = Some(PendingAction::RemoveGameFromLibrary);
            }

            Message::PlayFromDetail => {
                let viewing = self.viewing_sha1.clone();
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
                self.previous_screen = Some(self.screen);
                self.was_running_before_settings = self.running();
                self.pause();
                self.screen = Screen::Settings;
            }
            Message::SaveBattery => {
                self.save();
            }
            Message::Run => self.run(),
            Message::Pause => self.pause(),
            Message::TogglePause => {
                if self.running() {
                    self.pause();
                } else {
                    self.run();
                }
            }
            Message::TakeScreenshot => {
                // Grab the current framebuffer and SGB state from whichever game mode is active
                let capture_data = match &self.game {
                    Game::Loaded(LoadedGame::Emulator(emu)) => {
                        let gb = emu.game_boy();
                        let sgb_data = gb
                            .sgb()
                            .map(|sgb| sgb.render_data(gb.ppu().control().video_enabled()));
                        Some((gb.screen().clone(), sgb_data))
                    }
                    Game::Loaded(LoadedGame::Debugger(dbg)) => {
                        let gb = dbg.game_boy();
                        let sgb_data = gb
                            .sgb()
                            .map(|sgb| sgb.render_data(gb.ppu().control().video_enabled()));
                        Some((gb.screen().clone(), sgb_data))
                    }
                    _ => None,
                };
                if let Some((screen, sgb_render_data)) = capture_data {
                    let capture = library::activity::FrameCapture::capture(
                        screen.front(),
                        sgb_render_data.as_ref(),
                        self.settings.use_sgb_colors,
                        &self.settings.palette.to_string(),
                    );
                    if let Some(current) = &mut self.current_game {
                        if let Some(session) = &mut current.session {
                            session.events.push(library::activity::SessionEvent {
                                at: jiff::Timestamp::now(),
                                kind: library::activity::EventKind::Screenshot { frame: capture },
                            });
                            library::activity::write_session(&current.game_dir, session);
                            self.store.update_live_screenshots(session);
                        }
                    }
                    self.screenshot_toast = Some(Instant::now());
                }
            }
            Message::Reset => {
                self.pending_action = Some(PendingAction::ResetEmulator);
            }

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

            Message::DismissScreenshotToast => {
                self.screenshot_toast = None;
            }

            Message::CloseRequested => {
                self.settings.save(); // persist window size
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
                                let mut emu =
                                    debugger.disable_debugger(self.settings.use_sgb_colors);
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
            Message::CompleteSetup { internet_enabled } => {
                self.settings.internet_enabled = internet_enabled;
                self.settings.setup_complete = true;
                self.settings.save();
            }
            Message::Settings(message) => match message {
                settings_view::Message::SelectSection(section) => {
                    self.settings_section = section;
                    if section == settings_view::Section::Hardware
                        && self.settings.cartridge_rw_enabled
                    {
                        return Task::perform(
                            smol::unblock(cartridge_rw::detect_devices),
                            |devices| {
                                settings_view::Message::CartridgeDevicesFound(devices).into()
                            },
                        );
                    }
                }
                settings_view::Message::Back => {
                    self.screen = self.previous_screen.take().unwrap_or(Screen::Library);
                    self.listening_for = None;
                    if self.was_running_before_settings {
                        self.run();
                        self.was_running_before_settings = false;
                    }
                }
                settings_view::Message::SetInternetEnabled(enabled) => {
                    self.settings.internet_enabled = enabled;
                    self.settings.save();
                }
                settings_view::Message::SetHasheousEnabled(enabled) => {
                    self.settings.hasheous_enabled = enabled;
                    self.settings.save();
                }
                settings_view::Message::SetHomebrewHubEnabled(enabled) => {
                    self.settings.homebrew_hub_enabled = enabled;
                    self.settings.save();
                }
                settings_view::Message::PickRomDirectory => {
                    let dialog = rfd::AsyncFileDialog::new();
                    return Task::perform(dialog.pick_folder(), |folder| match folder {
                        Some(handle) => {
                            let path = handle.path().to_path_buf();
                            settings_view::Message::AddRomDirectory(path).into()
                        }
                        None => Message::None,
                    });
                }
                settings_view::Message::AddRomDirectory(path) => {
                    if !self.settings.rom_directories.contains(&path) {
                        self.settings.rom_directories.push(path.clone());
                        self.settings.save();
                        let dirs = vec![path];
                        let cat = self.catalogue.clone();
                        return Task::perform(
                            smol::unblock(move || library::scanner::scan_directories(&dirs, &cat)),
                            |entries| Message::ScanComplete(!entries.is_empty()),
                        );
                    }
                }
                settings_view::Message::RemoveRomDirectory(index) => {
                    if index < self.settings.rom_directories.len() {
                        self.settings.rom_directories.remove(index);
                        self.settings.save();
                    }
                }
                settings_view::Message::SelectPalette(palette) => {
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
                settings_view::Message::SetUseSgbColors(enabled) => {
                    self.settings.use_sgb_colors = enabled;
                    self.settings.save();
                    if let Game::Loaded(LoadedGame::Emulator(emu)) = &mut self.game {
                        emu.set_use_sgb_colors(enabled);
                    }
                }
                settings_view::Message::SetCartridgeRwEnabled(enabled) => {
                    self.settings.cartridge_rw_enabled = enabled;
                    self.settings.save();
                    if enabled {
                        // Auto-scan when enabling
                        return Task::perform(
                            smol::unblock(cartridge_rw::detect_devices),
                            |devices| {
                                settings_view::Message::CartridgeDevicesFound(devices).into()
                            },
                        );
                    } else {
                        self.detected_cartridge_devices.clear();
                    }
                }
                settings_view::Message::ScanCartridgeDevices => {
                    return Task::perform(
                        smol::unblock(cartridge_rw::detect_devices),
                        |devices| {
                            settings_view::Message::CartridgeDevicesFound(devices).into()
                        },
                    );
                }
                settings_view::Message::CartridgeDevicesFound(devices) => {
                    self.detected_cartridge_devices = devices;
                }
                settings_view::Message::StartListening(target) => {
                    self.listening_for = Some(target);
                }
                settings_view::Message::CaptureBinding(key_str) => {
                    if let Some(target) = self.listening_for.take() {
                        match target {
                            settings_view::ListeningFor::Keyboard(action) => {
                                self.settings.keyboard_bindings.set(action, key_str);
                            }
                            settings_view::ListeningFor::Gamepad(action) => {
                                self.settings.gamepad_bindings.set(action, key_str);
                            }
                        }
                        self.settings.save();
                        controls::update_bindings(
                            &self.settings.keyboard_bindings,
                            &self.settings.gamepad_bindings,
                        );
                    }
                }
                settings_view::Message::ClearBinding => {
                    if let Some(target) = self.listening_for.take() {
                        match target {
                            settings_view::ListeningFor::Keyboard(action) => {
                                self.settings.keyboard_bindings.0.remove(&action);
                            }
                            settings_view::ListeningFor::Gamepad(action) => {
                                self.settings.gamepad_bindings.0.remove(&action);
                            }
                        }
                        self.settings.save();
                        controls::update_bindings(
                            &self.settings.keyboard_bindings,
                            &self.settings.gamepad_bindings,
                        );
                    }
                }
                settings_view::Message::CancelCapture => {
                    self.listening_for = None;
                }
                settings_view::Message::ResetBindings => {
                    self.settings.keyboard_bindings = settings::Bindings::default_keyboard();
                    self.settings.gamepad_bindings = settings::Bindings::default_gamepad();
                    self.settings.save();
                    self.listening_for = None;
                    controls::update_bindings(
                        &self.settings.keyboard_bindings,
                        &self.settings.gamepad_bindings,
                    );
                }
            },
            Message::Library(message) => match message {
                library::view::Message::SelectGame(sha1) => {
                    return self.go_to_detail(&sha1);
                }
                library::view::Message::HoverGame(sha1) => {
                    self.hovered_library_game = Some(sha1);
                }
                library::view::Message::UnhoverGame => {
                    self.hovered_library_game = None;
                }
                library::view::Message::QuickPlay(sha1) => {
                    let same_game = self
                        .current_game
                        .as_ref()
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
            Message::ActivityLoaded(raw) => {
                // Only apply if we're still viewing the same game
                if self.viewing_sha1.as_deref() == Some(&raw.sha1) {
                    self.store.set_raw_activity_detail(raw);
                }
            }
            Message::ScanComplete(changed) => {
                if changed {
                    self.store.rebuild_index();
                }
                if self.settings.internet_enabled && self.settings.hasheous_enabled {
                    return Task::perform(
                        smol::unblock(|| library::scanner::enrich_next()),
                        |result| Message::EnrichComplete(result),
                    );
                }
            }
            Message::EnrichComplete(result) => {
                if let Some(sha1) = &result.sha1 {
                    if result.data_changed {
                        self.store.notify_metadata_changed(sha1);
                    }
                }

                // Sync recent game titles with enriched library entries
                for summary in self.store.all_summaries() {
                    self.recent_games
                        .update_title(&summary.entry.sha1, &summary.entry.display_title());
                }
                self.recent_games.save();

                // Also update the current game if loaded
                if let Some(current) = &mut self.current_game {
                    if let Some((_dir, entry)) = library::find_by_sha1(&current.entry.sha1) {
                        current.entry = entry;
                        current.cover = library::load_cover(&current.game_dir)
                            .map(|bytes| iced::widget::image::Handle::from_bytes(bytes));
                    }
                }

                // Chain: enrich next game if there are more
                if result.has_more
                    && self.settings.internet_enabled
                    && self.settings.hasheous_enabled
                {
                    return Task::perform(
                        smol::unblock(|| library::scanner::enrich_next()),
                        |result| Message::EnrichComplete(result),
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
        // First-boot setup
        if !self.settings.setup_complete {
            return self.setup_view();
        }

        // Build the main view based on the current screen
        let main: Element<'_, Message> = if self.screen == Screen::Emulator {
            let show_toast = self.screenshot_toast.is_some();

            if let Fullscreen::Active { cursor_hidden, .. } = self.fullscreen {
                let screen = self.emulator_view(true);
                let content = if show_toast {
                    let stk: Element<'_, Message> =
                        Stack::with_children(vec![screen, screenshot_toast()]).into();
                    container(stk)
                } else {
                    container(screen)
                }
                .center(Fill)
                .style(|_| container::Style {
                    background: Some(iced::Color::BLACK.into()),
                    ..Default::default()
                });

                let mut area = mouse_area(content).on_move(|_| Message::MouseMoved);
                if cursor_hidden {
                    area = area.interaction(mouse::Interaction::Hidden);
                }
                area.into()
            } else {
                let screen = container(self.emulator_view(false)).center(Fill);
                let screen: Element<'_, Message> = if show_toast {
                    Stack::with_children(vec![screen.into(), screenshot_toast()]).into()
                } else {
                    screen.into()
                };
                column![self.action_bar.view(self), horizontal_rule(), screen,].into()
            }
        } else if self.screen == Screen::Settings {
            settings_view::view(
                &self.settings,
                self.settings_section,
                self.listening_for,
                &self.detected_cartridge_devices,
            )
        } else {
            match self.screen {
                Screen::Detail => self.detail_view(),
                _ => {
                    let content = match self.screen {
                        Screen::Library => {
                            library::view::view(&self.store, self.hovered_library_game.as_deref())
                        }
                        Screen::HomebrewBrowser => {
                            if let Some(state) = &self.homebrew_browser {
                                library::homebrew_browser::view(state, &self.catalogue)
                            } else {
                                library::view::view(
                                    &self.store,
                                    self.hovered_library_game.as_deref(),
                                )
                            }
                        }
                        Screen::ScreenshotGallery => {
                            if let Some(state) = &self.gallery_state {
                                library::screenshot_gallery::view(state)
                            } else {
                                self.detail_view()
                            }
                        }
                        _ => unreachable!(),
                    };
                    column![
                        self.action_bar.view(self),
                        horizontal_rule(),
                        container(content).center(Fill)
                    ]
                    .into()
                }
            }
        };

        if let Some(action) = &self.pending_action {
            let (prompt, confirm_label) = match action {
                PendingAction::SwitchGame(_) => {
                    ("Close the current game and switch?", "Close Game")
                }
                PendingAction::CloseApp => ("Close the current game and quit?", "Quit"),
                PendingAction::ResetEmulator => (
                    "Reset the emulator? Unsaved progress will be lost.",
                    "Reset",
                ),
                PendingAction::StopGame => ("Stop playing and end this session?", "Stop"),
                PendingAction::RemoveGameFromLibrary => {
                    ("Remove this game and all its save data?", "Remove")
                }
            };

            let mut info = column![iced_text(prompt)].spacing(s());

            if let Some(current) = &self.current_game {
                info = info.push(
                    iced_text(current.entry.display_title())
                        .size(text::sizes::xl())
                        .font(fonts::heading()),
                );
                let last_save_time = current.session.as_ref().and_then(|s| s.last_save_time());
                if let Some(ts) = last_save_time {
                    info = info.push(
                        iced_text(format!("Last saved {}", friendly_ago(ts)))
                            .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
                    );
                } else {
                    info = info.push(
                        iced_text("No saves").color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
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
                            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
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

        let sha1 = match viewing_sha1 {
            Some(s) => s,
            None => return self.empty_detail_view(),
        };

        // Get entry from current game (if it's the one being viewed) or store
        let entry = if let Some(current) = &self.current_game {
            if current.entry.sha1 == sha1 {
                &current.entry
            } else {
                match self.store.summary(sha1) {
                    Some(s) => &s.entry,
                    None => return self.empty_detail_view(),
                }
            }
        } else {
            match self.store.summary(sha1) {
                Some(s) => &s.entry,
                None => return self.empty_detail_view(),
            }
        };

        // Use pre-rendered thumbnail from the store
        let cover = self.store.summary(sha1).and_then(|s| s.thumbnail.as_ref());

        let activity_state = self.store.activity_for(sha1);

        let live_session = self
            .current_game
            .as_ref()
            .filter(|c| sha1 == c.entry.sha1.as_str())
            .and_then(|c| c.session.as_ref());

        let is_loaded = self
            .current_game
            .as_ref()
            .map(|c| c.entry.sha1 == sha1 && matches!(self.game, Game::Loaded(_)))
            .unwrap_or(false);

        library::detail_view::view(library::detail_view::DetailData {
            entry,
            cover,
            activity_state,
            live_session,
            live_screenshots: self.store.live_screenshots(),
            hovered_log_entry: self.hovered_log_entry,
            header_hovered: self.header_hovered,
            is_loaded,
        })
    }

    /// Navigate to the detail screen for a game, loading activity in background.
    fn go_to_detail(&mut self, sha1: &str) -> Task<Message> {
        self.store.mark_activity_loading(sha1);
        self.viewing_sha1 = Some(sha1.to_string());
        self.screen = Screen::Detail;
        self.load_activity_async(sha1)
    }

    /// Load cover images for visible homebrew entries (first batch only).
    fn load_homebrew_covers(&self) -> Task<Message> {
        use library::homebrew_browser::Message as H;
        let Some(state) = &self.homebrew_browser else {
            return Task::none();
        };

        let results = if state.search_text.is_empty() {
            self.catalogue.homebrew()
        } else {
            self.catalogue.search_homebrew(&state.search_text)
        };

        let visible = state.visible_count.min(results.len());

        let tasks: Vec<Task<Message>> = results[..visible]
            .iter()
            .filter(|e| !state.covers.contains_key(&e.slug))
            .filter_map(|e| {
                let url = e.download_cover_url()?;
                let slug = e.slug.clone();
                let client = self.homebrew_client.clone();
                Some(Task::perform(
                    smol::unblock(move || {
                        client.download_image(&url).ok().map(|bytes| (slug, bytes))
                    }),
                    |result| match result {
                        Some((slug, bytes)) => {
                            Message::HomebrewBrowser(H::CoverLoaded(slug, bytes))
                        }
                        None => Message::None,
                    },
                ))
            })
            .collect();

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    /// Kick off a background load of activity detail for a game.
    fn load_activity_async(&self, sha1: &str) -> Task<Message> {
        let sha1 = sha1.to_string();
        if let Some(game_dir) = self.store.game_dir(&sha1) {
            let game_dir = game_dir.to_path_buf();
            Task::perform(
                smol::unblock(move || {
                    library::store::GameStore::load_raw_activity(&sha1, &game_dir)
                }),
                Message::ActivityLoaded,
            )
        } else {
            Task::none()
        }
    }

    fn empty_detail_view(&self) -> Element<'_, Message> {
        library::view::view(&self.store, self.hovered_library_game.as_deref())
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
        let (ram, cartridge_title) = match &self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                if !debugger.game_boy().cartridge().has_battery() {
                    return;
                }
                (
                    debugger.game_boy().cartridge().ram(),
                    debugger.game_boy().cartridge().title().to_string(),
                )
            }
            Game::Loaded(LoadedGame::Emulator(emulator)) => {
                if !emulator.game_boy().cartridge().has_battery() {
                    return;
                }
                (
                    emulator.game_boy().cartridge().ram(),
                    emulator.game_boy().cartridge().title().to_string(),
                )
            }
            _ => return,
        };
        let Some(ram) = ram else { return };
        let Some(current) = &mut self.current_game else {
            return;
        };

        if let Some(session) = &mut current.session {
            // Check if SRAM has meaningfully changed, ignoring scratch regions
            let previous = session.last_sram().or(current.initial_sram.as_deref());
            let changed = match previous {
                Some(prev) => library::game_db::sram_changed(&cartridge_title, &ram, prev),
                None => true, // No previous data at all — always record
            };

            if changed {
                session.events.push(library::activity::SessionEvent {
                    at: jiff::Timestamp::now(),
                    kind: library::activity::EventKind::Save { sram: ram.to_vec() },
                });
                // Write incrementally for crash safety
                library::activity::write_session(&current.game_dir, session);
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let listening_keyboard = matches!(
            self.listening_for,
            Some(settings_view::ListeningFor::Keyboard(_))
        );
        let listening_gamepad = matches!(
            self.listening_for,
            Some(settings_view::ListeningFor::Gamepad(_))
        );

        Subscription::batch([
            if listening_keyboard {
                event::listen_with(controls::capture_event_handler)
            } else if listening_gamepad {
                event::listen_with(controls::escape_cancel_handler)
            } else if self.running() {
                event::listen_with(controls::event_handler)
            } else {
                Subscription::none()
            },
            if listening_gamepad {
                controls::gamepad_capture_subscription()
            } else if self.running() {
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
                iced::Event::Window(window::Event::Resized(size)) => {
                    Some(Message::WindowResized(size))
                }
                iced::Event::Window(window::Event::CloseRequested) => Some(Message::CloseRequested),
                // Escape always exits fullscreen (not rebindable — it's an escape hatch)
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
            if self.screenshot_toast.is_some() {
                time::every(std::time::Duration::from_millis(1500))
                    .map(|_| Message::DismissScreenshotToast)
            } else {
                Subscription::none()
            },
        ])
    }
}

fn screenshot_toast<'a>() -> Element<'a, Message> {
    container(
        container(
            row![
                icons::m(Icon::Camera).style(|_, _| svg::Style {
                    color: Some(iced::Color::WHITE),
                }),
                iced_text("Screenshot saved").color(iced::Color::WHITE),
            ]
            .spacing(s())
            .align_y(Center),
        )
        .padding(s())
        .style(|_| container::Style {
            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
            border: iced::Border::default().rounded(6),
            ..Default::default()
        }),
    )
    .align_bottom(Fill)
    .align_right(Fill)
    .padding(l())
    .into()
}

fn friendly_ago(timestamp: jiff::Timestamp) -> String {
    let secs = jiff::Timestamp::now().duration_since(timestamp).as_secs();
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs} seconds ago")
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{mins} minutes ago")
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hours} hours ago")
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            "yesterday".to_string()
        } else {
            format!("{days} days ago")
        }
    }
}
