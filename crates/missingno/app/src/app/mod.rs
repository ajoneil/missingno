use std::{fs, path::PathBuf, time::Instant};

use std::collections::HashMap;

use iced::{Task, Theme, window};
use missingno_session::audio_output::AudioOutput;
use ui::fonts;
use ui::icons::Icon;

mod action_bar;
pub(crate) mod automation;
mod console;
mod controls;
mod debugger;
mod emulation;
mod emulator;
mod launch;
pub mod library;
mod load;
pub(crate) use load::file_stem_title;
mod recent;
mod session_bridge;
pub mod settings;
pub(crate) mod system;
mod ui;
mod views;

use missingno_session::{SessionEvent, SharedSession};

#[cfg(unix)]
use missingno_session::AttachEndpoint;

// Cartridge reader/writer hardware support
use crate::cartridge_rw;

/// Gamescope scales a window's picture to the output but clamps the pointer to
/// the window's own extent, so only a fullscreen window keeps them aligned.
pub(crate) fn running_under_gamescope() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop == "gamescope")
}

pub fn run(
    rom_path: Option<PathBuf>,
    debugger: bool,
    link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
    boot_rom: Option<missingno_gb::BootRom>,
    ui_automation: bool,
) -> iced::Result {
    // Load settings early to get saved window size
    let saved = settings::Settings::load();
    let window_width = saved.window_width.unwrap_or(1280.0);
    let window_height = saved.window_height.unwrap_or(720.0);

    // Wrap in a Cell so the non-Clone link can be taken from the FnMut closure.
    let link_cell = std::cell::Cell::new(link);
    let mut app = iced::application(
        move || {
            App::new(
                rom_path.clone(),
                debugger,
                link_cell.take(),
                boot_rom.clone(),
                ui_automation,
            )
        },
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
        fullscreen: running_under_gamescope(),
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
    /// The UI-thread cpal stream for the current game's audio; the session holds
    /// the matching sink. Replaced per game load, `None` when nothing is loaded.
    audio_output: Option<AudioOutput>,
    /// The shared session hosting the current game's console, `None` until a game
    /// loads. Owns the session thread; dropping it shuts the thread down.
    session: Option<SharedSession>,
    /// The socket the current session is published on, so clients in other
    /// processes can drive it. `None` unless the user allows external clients;
    /// dropping it unpublishes.
    #[cfg(unix)]
    attach_endpoint: Option<AttachEndpoint>,
    /// The Iced sink a per-game bridge thread forwards session events into,
    /// handed over once at startup by the app-lifetime subscription.
    event_sink: Option<iced::futures::channel::mpsc::UnboundedSender<SessionEvent>>,
    /// The shared slot the automation subscription hands its call sink into,
    /// read by the automation endpoint's socket threads.
    automation_sink: automation::bridge::SharedSink,
    /// The UI-automation socket, open while the setting or CLI flag is on.
    /// App-lifetime, independent of whether a game is loaded.
    #[cfg(unix)]
    automation_endpoint: Option<missingno_session::attach::SocketHost>,
    /// Parked `ui_tree` replies awaiting the bounds walk that will answer them.
    automation_pending: HashMap<u64, automation::update::PendingReply>,
    automation_next_request: u64,
    /// The window's last-known scale factor, reported through `status`/`ui_tree`.
    /// Filled by a scale query at startup and after each resize.
    window_scale: Option<f32>,
    /// `--allow-ui-automation`: opens the endpoint for this run without
    /// persisting the setting.
    automation_flag: bool,
    recent_games: recent::RecentGames,
    settings: settings::Settings,
    /// The running emulation session. Only set when a game is actually loaded.
    current_game: Option<CurrentGame>,
    store: library::store::GameStore,
    /// Action waiting for user confirmation (e.g. close game before launching another).
    pending_action: Option<PendingAction>,
    /// A transient status line (screenshot, save/load result, recording
    /// lifecycle, replay divergence), shown as a toast until it times out.
    notice: Option<(Notice, Instant)>,
    /// The launch options window, standing over whatever screen raised it.
    launch_window: Option<launch::Window>,
    /// Serial link cable connection (BGB link protocol), injected into GameBoy on load.
    serial_link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
    /// Finished Game Boy Printer prints, sent from the printer (on the emu
    /// thread) and drained on the UI thread to log against the play session.
    print_tx: std::sync::mpsc::Sender<crate::printer::CompletedPrint>,
    print_rx: std::sync::mpsc::Receiver<crate::printer::CompletedPrint>,
    /// Boot ROM supplied on the CLI, applied to every Game Boy family load.
    boot_rom: Option<missingno_gb::BootRom>,
    /// Homebrew Hub API client (shared, thread-safe).
    homebrew_client: std::sync::Arc<library::homebrew_hub::HomebrewHubClient>,
    /// Bundled game catalogue (commercial + homebrew).
    catalogue: std::sync::Arc<library::catalogue::Catalogue>,
    /// Cartridge reader/writer device detection and dump progress.
    cartridge_rw: CartridgeRwState,
    /// Whether the hamburger menu overlay is open.
    menu_open: bool,
    /// Whether an input recording is currently being captured (play mode).
    recording: bool,
    /// Live library search text. Transient — not persisted, survives screen
    /// transitions (unlike the per-screen hover state).
    library_search: String,
    /// Live library system filter. Transient, like `library_search`.
    library_filter: library::store::SystemFilter,
    /// The running machine's ports and built-in controls, cached at load and
    /// after every plug so input events resolve against what is plugged now.
    control_surfaces: Option<missingno_session::ControlSurfaces>,
    /// Which host device drives which console port, keyed by the ids this
    /// session's input events carry; seeded from what the settings remember.
    port_assignments: controls::PortAssignments,
    /// The gamepads the host has connected, in connection order.
    gamepads: Vec<controls::ConnectedPad>,
    /// Where the machine's latching panel switches sit, seated at load and
    /// worked by both the Console panel and the keys bound to them.
    switch_levels: emulation::SwitchLevels,
}

/// Cartridge reader/writer polling state (device detection and active-dump progress).
#[derive(Default)]
struct CartridgeRwState {
    /// Devices detected on the system.
    detected_devices: Vec<cartridge_rw::DetectedDevice>,
    /// Last-seen port names (to detect changes cheaply).
    known_ports: Vec<String>,
    /// Progress of an active ROM dump, if any.
    dump_progress: Option<cartridge_rw::DumpProgress>,
}

impl App {
    /// Get the SHA1 of the game being viewed, if on a detail/sub-screen.
    fn viewing_sha1(&self) -> Option<&str> {
        match &self.screen {
            Screen::ViewingGame { sha1, .. } => Some(sha1),
            _ => None,
        }
    }

    /// Play `sha1`: resume it when it is the game already loaded, ask before
    /// dropping a different one, otherwise load and start it.
    pub(in crate::app) fn play_game(&mut self, sha1: String) -> Task<Message> {
        let same_game = self
            .current_game
            .as_ref()
            .map(|current| current.entry.sha1 == sha1)
            .unwrap_or(false);

        if same_game {
            self.run();
            self.screen = Screen::Emulator;
        } else if matches!(self.game, Game::Loaded(_)) {
            self.pending_action = Some(PendingAction::SwitchGame(sha1));
        } else {
            load::select_game(self, &sha1);
            return load::play_current_game(self);
        }
        Task::none()
    }

    /// Get the keybinding capture state, if on the settings screen.
    fn listening_for(&self) -> Option<settings::view::ListeningFor> {
        match &self.screen {
            Screen::Settings { listening_for, .. } => *listening_for,
            _ => None,
        }
    }

    /// Open or close the UI-automation socket to match the setting and the CLI
    /// flag. App-lifetime: it does not depend on a game being loaded.
    #[cfg(unix)]
    fn reconcile_automation(&mut self) {
        let wanted = self.settings.allow_ui_automation || self.automation_flag;
        if wanted && self.automation_endpoint.is_none() {
            match automation::endpoint::open(self.automation_sink.clone()) {
                Ok(endpoint) => self.automation_endpoint = Some(endpoint),
                Err(error) => {
                    self.notice = Some((
                        Notice::text(format!("Could not enable UI automation: {error}")),
                        Instant::now(),
                    ));
                }
            }
        } else if !wanted {
            self.automation_endpoint = None;
        }
    }

    #[cfg(not(unix))]
    fn reconcile_automation(&mut self) {}

    /// Hand the input event handlers the routing state for the machine and the
    /// bindings as they now stand.
    fn push_routing(&self) {
        use settings::Surface;
        let platform = self.platform();
        let system = |surface| {
            platform
                .map(|platform| self.settings.controls.system_map(platform, surface))
                .unwrap_or_default()
        };
        controls::publish(controls::Routing {
            emulator_keyboard: self.settings.controls.emulator_map(Surface::Keyboard),
            emulator_gamepad: self.settings.controls.emulator_map(Surface::Gamepad),
            system_keyboard: system(Surface::Keyboard),
            system_gamepad: system(Surface::Gamepad),
            surfaces: self.control_surfaces.clone(),
            assignments: self.port_assignments.clone(),
            pointer_drives_knob: platform
                .is_none_or(|platform| self.settings.controls.pointer_knob(platform)),
            ..controls::Routing::default()
        });
    }

    /// Seat the host devices on a machine that has just come up: read what its
    /// ports carry, then put each device back on the port this system last saw
    /// it play.
    fn seat_input_devices(&mut self) {
        self.control_surfaces = self.handle().map(|handle| handle.control_surfaces());
        let panel = self
            .control_surfaces
            .as_ref()
            .map(|surfaces| surfaces.panel)
            .unwrap_or_default();
        self.switch_levels.seat(panel);
        self.port_assignments = self.seated_assignments();
        self.push_routing();
    }

    /// The machine's first port — where a device with nothing recorded plays.
    fn first_port(&self) -> missingno_core::ports::PortId {
        self.control_surfaces
            .as_ref()
            .and_then(controls::first_port)
            .unwrap_or(missingno_core::ports::PortId(0))
    }

    /// Where the connected devices sit on the loaded machine: the ports this
    /// system remembers them on, and its first port for anything it has never
    /// seen. Pads are matched by identity, so a pad that was unplugged and
    /// plugged back in returns to its own port.
    fn seated_assignments(&self) -> controls::PortAssignments {
        let first = self.first_port();
        let remembered = self
            .platform()
            .and_then(|platform| self.settings.controls.assignments(platform));

        let mut gamepads = HashMap::new();
        let mut occurrences: HashMap<&settings::GamepadIdentity, usize> = HashMap::new();
        for pad in &self.gamepads {
            let occurrence = occurrences.entry(&pad.identity).or_insert(0);
            if let Some(remembered) = remembered
                && let Some(port) = remembered.port_for(&pad.identity, *occurrence)
            {
                gamepads.insert(pad.id, port);
            }
            *occurrence += 1;
        }

        controls::PortAssignments {
            keyboard: remembered
                .and_then(|remembered| remembered.keyboard)
                .unwrap_or(first),
            gamepads,
        }
    }

    /// The machine's controller ports and the host devices playing them, as the
    /// play screen's Controllers section shows them. A port whose peripherals
    /// the host supplies, or which carries no controls at all, is not a
    /// controller jack and is left out — the Game Boy's link socket.
    fn controller_seating(&self) -> emulator::Controllers {
        use missingno_core::ports::Provider;

        let Some(surfaces) = &self.control_surfaces else {
            return emulator::Controllers::default();
        };
        let ports: Vec<emulator::PortSeat> = surfaces
            .ports
            .iter()
            .filter(|plugged| {
                plugged.descriptor.accepts.iter().any(|peripheral| {
                    peripheral.provider == Provider::Console && !peripheral.controls.is_empty()
                })
            })
            .map(|plugged| emulator::PortSeat {
                port: plugged.descriptor.port,
                label: plugged.descriptor.label,
                // Unplugged is a console-built peripheral with no controls, so
                // it stands among the choices; a host-supplied one cannot.
                choices: plugged
                    .descriptor
                    .accepts
                    .iter()
                    .filter(|peripheral| peripheral.provider == Provider::Console)
                    .map(|peripheral| emulator::ControllerChoice {
                        peripheral: peripheral.id,
                        label: peripheral.label,
                    })
                    .collect(),
                plugged: plugged.plugged,
            })
            .collect();

        let first = self.first_port();
        let devices = std::iter::once(emulator::DeviceSeat {
            source: controls::InputSource::Keyboard,
            name: "Keyboard".to_string(),
            port: self.port_assignments.keyboard,
        })
        .chain(self.gamepads.iter().map(|pad| {
            emulator::DeviceSeat {
                source: controls::InputSource::Gamepad(pad.id),
                name: pad.identity.name.clone(),
                port: self
                    .port_assignments
                    .gamepads
                    .get(&pad.id)
                    .copied()
                    .unwrap_or(first),
            }
        }))
        .collect();

        emulator::Controllers { ports, devices }
    }

    /// Point a host device at a port, for this session and for the next time
    /// this system is played.
    fn assign_device(
        &mut self,
        source: controls::InputSource,
        port: missingno_core::ports::PortId,
    ) {
        let platform = self.platform();
        match source {
            controls::InputSource::Keyboard => {
                self.port_assignments.keyboard = port;
                if let Some(platform) = platform {
                    self.settings.controls.set_keyboard_port(platform, port);
                }
            }
            controls::InputSource::Gamepad(id) => {
                let default = self.first_port();
                self.port_assignments.gamepads.insert(id, port);
                if let (Some(platform), Some((identity, occurrence))) =
                    (platform, self.pad_seat(id))
                {
                    self.settings
                        .controls
                        .set_gamepad_port(platform, identity, occurrence, port, default);
                }
            }
        }
        self.settings.save();
        self.push_routing();
    }

    /// How a connected pad is remembered: its identity, and how many identical
    /// twins connected before it.
    fn pad_seat(&self, id: gilrs::GamepadId) -> Option<(settings::GamepadIdentity, usize)> {
        let pad = self.gamepads.iter().find(|pad| pad.id == id)?;
        let occurrence = self
            .gamepads
            .iter()
            .take_while(|earlier| earlier.id != id)
            .filter(|earlier| earlier.identity == pad.identity)
            .count();
        Some((pad.identity.clone(), occurrence))
    }

    /// Stamp the current session's end time and flush it to disk.
    fn end_current_session(&mut self) {
        if let Some(current) = &mut self.current_game
            && let Some(session) = &mut current.session
        {
            session.end = Some(jiff::Timestamp::now());
            library::activity::write_session(&current.game_dir, session);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum FlashState {
    /// Flash in progress.
    InProgress(cartridge_rw::FlashProgress),
    /// Flash completed successfully.
    Complete,
    /// Flash failed.
    Failed(String),
}

/// A transient toast: what happened, and an optional icon for the ones the app
/// raises itself. Outcomes reported by the session arrive as text alone.
#[derive(Clone)]
struct Notice {
    icon: Option<Icon>,
    message: String,
    /// Something the user can do about what happened. A notice carrying one
    /// stays up until it is taken or dismissed.
    action: Option<NoticeAction>,
}

/// The offer a notice makes: a label and the message pressing it sends.
#[derive(Clone)]
struct NoticeAction {
    label: &'static str,
    message: Box<Message>,
}

impl Notice {
    fn text(message: impl Into<String>) -> Self {
        Notice {
            icon: None,
            message: message.into(),
            action: None,
        }
    }

    fn with_icon(icon: Icon, message: impl Into<String>) -> Self {
        Notice {
            icon: Some(icon),
            message: message.into(),
            action: None,
        }
    }

    /// Something went wrong that the user can act on — no launch fails silently.
    fn failure(message: impl Into<String>, label: &'static str, action: Message) -> Self {
        Notice {
            icon: Some(Icon::Warning),
            message: message.into(),
            action: Some(NoticeAction {
                label,
                message: Box::new(action),
            }),
        }
    }
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
        /// Which page the Controls section shows, and the controller each of its
        /// port blocks has tabbed to.
        controls: settings::view::ControlsState,
        listening_for: Option<settings::view::ListeningFor>,
        previous_screen: Box<Screen>,
        was_running: bool,
    },
    Emulator,
}

enum DetailSubScreen {
    Detail {
        section: library::detail_view::Section,
        hovered_log_entry: Option<usize>,
        header_hovered: bool,
        /// The launch options this game's media leaves open and what fills the
        /// ones the user has not set, read off the media once on arrival rather
        /// than on every frame.
        media_options: launch::MediaOptions,
    },
    CartridgeActions {
        /// Whether to write saves alongside a flash operation.
        flash_write_save: bool,
        /// Whether the game has save data in the library.
        has_save: bool,
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
    SelectSection(library::detail_view::Section),
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

// A single Game exists for the app's lifetime.
#[allow(clippy::large_enum_variant)]
enum Game {
    Unloaded,
    Loading,
    Loaded(LoadedGame),
}

// One long-lived value swapped in place; the debugger arm is the larger.
#[allow(clippy::large_enum_variant)]
enum LoadedGame {
    Debugger(debugger::Debugger),
    Emulator(emulator::Emulator),
}

/// What both shells answer for, so the app drives a loaded game without caring
/// which one is showing.
impl LoadedGame {
    fn platform(&self) -> system::Platform {
        match self {
            Self::Debugger(debugger) => debugger.platform(),
            Self::Emulator(emulator) => emulator.platform(),
        }
    }

    fn technology(&self) -> missingno_core::video::DisplayTechnology {
        match self {
            Self::Debugger(debugger) => debugger.technology(),
            Self::Emulator(emulator) => emulator.technology(),
        }
    }

    fn running(&self) -> bool {
        match self {
            Self::Debugger(debugger) => debugger.running(),
            Self::Emulator(emulator) => emulator.running(),
        }
    }

    fn run(&mut self) {
        match self {
            Self::Debugger(debugger) => debugger.run(),
            Self::Emulator(emulator) => emulator.run(),
        }
    }

    fn pause(&mut self) {
        match self {
            Self::Debugger(debugger) => debugger.pause(),
            Self::Emulator(emulator) => emulator.pause(),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Debugger(debugger) => debugger.reset(),
            Self::Emulator(emulator) => emulator.reset(),
        }
    }

    fn apply_frame(&mut self, display: missingno_iced::Frame) {
        match self {
            Self::Debugger(debugger) => debugger.apply_frame(display),
            Self::Emulator(emulator) => emulator.apply_frame(display),
        }
    }

    /// The debugger shell, when that is the one showing — the inspection
    /// surfaces the play screen has no use for.
    fn debugger_mut(&mut self) -> Option<&mut debugger::Debugger> {
        match self {
            Self::Debugger(debugger) => Some(debugger),
            Self::Emulator(_) => None,
        }
    }
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
    /// Cartridge header title, cached so an SRAM save can run the game-specific
    /// scratch-region comparison without reaching into the session-owned console.
    cartridge_title: String,
}

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),
    Launch(launch::Message),

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
    HomebrewDownloaded(String, Vec<u8>, library::catalogue::CatalogueEntry),
    ScreenshotGallery(library::screenshot_gallery::Message),

    // Emulation
    Run,
    Pause,
    TogglePause,
    Reset,
    TakeScreenshot,
    /// Save the running machine state to the game's save-state slot.
    SaveState,
    /// Restore the game's save-state slot.
    LoadState,
    /// Start or stop capturing an input recording of the running game.
    ToggleRecording,
    /// Replay the game's recording slot, driving the running console.
    Replay,
    /// Export the session capture at this event index to a PNG (opens a dialog).
    ExportCapture(usize),
    /// The dialog resolved; write the capture at this event index, if a path
    /// was chosen.
    ExportCaptureSaved(usize, Option<rfd::FileHandle>),

    /// What an input event works on the running machine, at the given level; a
    /// release works no latching switch.
    SetControl(Vec<controls::Actuation>, bool),
    /// An analog control, normalised 0-1.
    SetAxis(missingno_core::system::ControlId, f32),
    /// The pads the host reports, as a freshly started subscription sees them:
    /// its ids are its own, so this replaces the roster rather than adding to
    /// it.
    GamepadRoster(Vec<controls::ConnectedPad>),
    /// A host gamepad appeared, with the name its driver reports.
    GamepadConnected(gilrs::GamepadId, settings::GamepadIdentity),
    GamepadDisconnected(gilrs::GamepadId),
    /// Put a controller type in a console port, from the play screen.
    PlugPeripheral(
        missingno_core::ports::PortId,
        missingno_core::ports::PeripheralId,
    ),
    /// Point a host input device at a console port.
    AssignDevice(controls::InputSource, missingno_core::ports::PortId),

    ToggleDebugger(bool),
    CompleteSetup {
        internet_enabled: bool,
    },
    Settings(settings::view::Message),
    Library(library::view::Message),
    ScanComplete(bool),
    ActivityLoaded(library::store::RawActivityDetail),
    EnrichComplete(library::scanner::EnrichResult),
    OpenUrl(String),

    WindowResized(iced::Size),
    ToggleFullscreen,
    ExitFullscreen,
    MouseMoved,
    HideCursorTick,
    CloseRequested,

    /// Raise the transient status-line toast with an outcome to report.
    ShowNotice(String),
    /// Time out the transient status-line toast.
    DismissNotice,

    // Cartridge reader/writer (device-level, not screen-specific)
    CartridgeRwPoll,
    CartridgeRwPortsChanged(Vec<cartridge_rw::DetectedDevice>),
    CartridgeRwDumpProgress(cartridge_rw::DumpProgress),
    CartridgeRwDumpComplete(Result<(Vec<u8>, Option<Vec<u8>>), String>),

    ToggleMenu,
    DismissMenu,
    /// A menu item was clicked — dismiss the menu and execute the inner message.
    MenuAction(Box<Message>),

    Debugger(debugger::Message),
    Emulator(emulator::Message),
    /// An item from the app-lifetime session subscription: the event sink handed
    /// over at startup, then every session event forwarded through it.
    Session(session_bridge::SessionBridge),
    /// An item from the app-lifetime UI-automation subscription: the call sink
    /// handed over at startup, then every automation call and its follow-ups.
    Automation(automation::Msg),

    None,
}

impl App {
    fn new(
        rom_path: Option<PathBuf>,
        debugger: bool,
        serial_link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
        boot_rom: Option<missingno_gb::BootRom>,
        ui_automation: bool,
    ) -> (Self, Task<Message>) {
        let settings = settings::Settings::load();
        let recent_games = recent::RecentGames::load();

        let store = library::store::GameStore::new();

        let (print_tx, print_rx) = std::sync::mpsc::channel();

        let mut app = Self {
            screen: Screen::Library { hovered_game: None },
            game: Game::Unloaded,
            debugger_enabled: debugger,
            fullscreen: if running_under_gamescope() {
                Fullscreen::Active {
                    cursor_hidden: false,
                    last_mouse_move: Instant::now(),
                }
            } else {
                Fullscreen::Windowed
            },
            audio_output: None,
            session: None,
            #[cfg(unix)]
            attach_endpoint: None,
            event_sink: None,
            automation_sink: automation::bridge::SharedSink::default(),
            #[cfg(unix)]
            automation_endpoint: None,
            automation_pending: HashMap::new(),
            automation_next_request: 0,
            window_scale: None,
            automation_flag: ui_automation,
            recent_games,
            settings,
            current_game: None,
            store,
            pending_action: None,
            notice: None,
            launch_window: None,
            serial_link,
            print_tx,
            print_rx,
            boot_rom,
            homebrew_client: std::sync::Arc::new(library::homebrew_hub::HomebrewHubClient::new()),
            catalogue: std::sync::Arc::new(library::catalogue::Catalogue::load()),
            cartridge_rw: CartridgeRwState::default(),
            menu_open: false,
            recording: false,
            library_search: String::new(),
            library_filter: library::store::SystemFilter::default(),
            control_surfaces: None,
            port_assignments: controls::PortAssignments::default(),
            gamepads: Vec::new(),
            switch_levels: emulation::SwitchLevels::default(),
        };

        app.push_routing();

        app.reconcile_automation();

        let mut tasks = vec![automation::update::query_scale()];

        if let Some(rom_path) = rom_path
            && let Ok(rom) = fs::read(&rom_path)
        {
            let rom = crate::patch::soft_patch(&rom_path, rom);
            tasks.push(load::setup_game(&mut app, rom_path, rom));
        }

        // Scan configured ROM directories on startup
        if !app.settings.rom_directories.is_empty() {
            let dirs = app.settings.rom_directories.clone();
            let cat = app.catalogue.clone();
            tasks.push(Task::perform(
                smol::unblock(move || library::scanner::scan_directories(&dirs, &cat)),
                |entries| Message::ScanComplete(!entries.is_empty()),
            ));
        } else if app.settings.internet_enabled {
            // No directories to scan, but still enrich any unenriched games
            tasks.push(library::update::enrich_task(&app));
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
            Message::Launch(message) => return launch::update(message, self),

            // Emulation messages
            Message::Run
            | Message::Pause
            | Message::TogglePause
            | Message::Reset
            | Message::TakeScreenshot
            | Message::SaveState
            | Message::LoadState
            | Message::ToggleRecording
            | Message::Replay
            | Message::ExportCapture(_)
            | Message::ExportCaptureSaved(..)
            | Message::ShowNotice(_)
            | Message::DismissNotice
            | Message::SetControl(..)
            | Message::SetAxis(..)
            | Message::PlugPeripheral(..)
            | Message::AssignDevice(..)
            | Message::ToggleDebugger(_) => return self.handle_emulation_message(message),

            Message::Session(bridge) => return self.handle_session_bridge(bridge),

            Message::GamepadRoster(pads) => {
                let present: Vec<_> = pads.iter().map(|pad| pad.id).collect();
                let lifted = controls::release_missing_pads(&present);
                self.gamepads = pads;
                self.port_assignments = self.seated_assignments();
                self.push_routing();
                if let Some(lifted) = lifted {
                    return Task::done(lifted);
                }
            }
            Message::GamepadConnected(id, identity) => {
                match self.gamepads.iter_mut().find(|pad| pad.id == id) {
                    // A reused id is a different pad, or the same one under a
                    // name its driver now reports differently.
                    Some(pad) => pad.identity = identity,
                    None => self.gamepads.push(controls::ConnectedPad { id, identity }),
                }
                // The pad may be one this system remembers, so re-seat rather
                // than leaving it on the first port.
                self.port_assignments = self.seated_assignments();
                self.push_routing();
            }
            Message::GamepadDisconnected(id) => {
                let lifted = controls::release_source(controls::InputSource::Gamepad(id));
                self.gamepads.retain(|pad| pad.id != id);
                self.port_assignments.gamepads.remove(&id);
                self.push_routing();
                if let Some(lifted) = lifted {
                    return Task::done(lifted);
                }
            }

            Message::Automation(msg) => return automation::update::handle(self, msg),

            // Settings messages
            Message::CompleteSetup { internet_enabled } => {
                self.settings.internet_enabled = internet_enabled;
                self.settings.setup_complete = true;
                self.settings.save();
            }
            Message::Settings(message) => return settings::update::handle(self, message),

            // Library messages
            Message::Library(_)
            | Message::Detail(_)
            | Message::Cartridge(_)
            | Message::HomebrewDownloaded(..)
            | Message::OpenHomebrewBrowser
            | Message::HomebrewBrowser(_)
            | Message::ScreenshotGallery(_)
            | Message::ActivityLoaded(_)
            | Message::ScanComplete(_)
            | Message::EnrichComplete(_)
            | Message::OpenUrl(_)
            | Message::CartridgeRwDumpProgress(_)
            | Message::CartridgeRwDumpComplete(_) => return library::update::handle(self, message),

            // Navigation
            Message::BackToLibrary => {
                self.menu_open = false;
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
                        // Recover the console and flush SRAM before unloading.
                        self.pause();
                        self.end_current_session();
                        self.game = Game::Unloaded;
                        self.current_game = None;

                        if load::select_game(self, &sha1) {
                            return load::play_current_game(self);
                        } else {
                            self.screen = Screen::Library { hovered_game: None };
                        }
                    }
                    Some(PendingAction::StopGame) => {
                        // Recover the console and flush SRAM before unloading.
                        self.pause();
                        self.end_current_session();
                        let sha1 = if let Some(current) = &self.current_game {
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
                        // Recover the console and flush SRAM before exiting.
                        self.pause();
                        self.end_current_session();
                        self.shutdown_emu();
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
                if let Some(sha1) = self.viewing_sha1().map(str::to_string) {
                    return self.play_game(sha1);
                }
            }
            Message::StopGame => {
                self.pending_action = Some(PendingAction::StopGame);
            }
            Message::BackToDetail => {
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
                let previous =
                    std::mem::replace(&mut self.screen, Screen::Library { hovered_game: None });
                self.screen = Screen::Settings {
                    section: settings::view::Section::default(),
                    controls: settings::view::ControlsState::default(),
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
                return Task::batch([
                    automation::update::on_window_resized(self, size),
                    automation::update::query_scale(),
                ]);
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
                    self.shutdown_emu();
                    return window::latest().and_then(window::close);
                }
            }

            // Cartridge RW polling (stays here — not library-specific)
            Message::CartridgeRwPoll => {
                let ports = cartridge_rw::list_ports();
                if ports != self.cartridge_rw.known_ports {
                    // Find which ports are new (need querying)
                    let new_ports: Vec<String> = ports
                        .iter()
                        .filter(|p| !self.cartridge_rw.known_ports.contains(p))
                        .cloned()
                        .collect();

                    // Remove devices on ports that disappeared
                    self.cartridge_rw
                        .detected_devices
                        .retain(|d| ports.contains(&d.port_name));

                    self.cartridge_rw.known_ports = ports;

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
                        .cartridge_rw
                        .detected_devices
                        .iter()
                        .any(|d| d.port_name == device.port_name)
                    {
                        self.cartridge_rw.detected_devices.push(device);
                    }
                }
            }

            // Delegated subsystems
            Message::Emulator(message) => {
                if let Game::Loaded(LoadedGame::Emulator(emulator)) = &mut self.game {
                    return emulator.update(message);
                }
            }

            Message::Debugger(message) => {
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    return debugger.update(message);
                }
            }

            Message::None => {}
        }

        Task::none()
    }
}
