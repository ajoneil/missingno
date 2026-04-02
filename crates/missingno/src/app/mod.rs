use std::{fs, path::PathBuf, time::Instant};

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, Task, Theme, event, mouse, time,
    widget::{column, container, mouse_area, row, svg, text as iced_text},
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
    game: Game,
    debugger_enabled: bool,
    fullscreen: Fullscreen,
    action_bar: ActionBar,
    audio_output: Option<AudioOutput>,
    recent_games: recent::RecentGames,
    settings: settings::Settings,
    settings_shown: bool,
    current_game: Option<CurrentGame>,
    game_info_shown: bool,
    library_cache: library::view::LibraryCache,
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
}

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),

    BackToLibrary,
    Run,
    Pause,
    Reset,
    SaveBattery,

    PressButton(joypad::Button),
    ReleaseButton(joypad::Button),

    ToggleDebugger(bool),
    SelectPalette(PaletteChoice),
    CompleteSetup { internet_enabled: bool },
    ShowSettings,
    Settings(settings_view::Message),
    Library(library::view::Message),
    ScanComplete,
    EnrichComplete,
    ToggleGameInfo,
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
            game: Game::Unloaded,
            debugger_enabled: debugger,
            fullscreen: Fullscreen::Windowed,
            action_bar: ActionBar::new(),
            audio_output: AudioOutput::new(),
            recent_games,
            settings,
            settings_shown: false,
            current_game: None,
            game_info_shown: false,
            library_cache,
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
                smol::unblock(|| library::scanner::enrich_library()),
                |_| Message::EnrichComplete,
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
                self.save();
                self.game = Game::Unloaded;
                self.current_game = None;
                self.game_info_shown = false;
            }
            Message::SaveBattery => {
                self.save();
            }
            Message::Run => self.run(),
            Message::Pause => self.pause(),
            Message::Reset => {
                self.save();
                self.reset();
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
                self.save();
                return window::latest().and_then(window::close);
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
            Message::ShowSettings => {
                self.settings_shown = true;
            }
            Message::Settings(message) => match message {
                settings_view::Message::Back => {
                    self.settings_shown = false;
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
                library::view::Message::PlayGame(sha1) => {
                    if let Some((_game_dir, entry)) = library::find_by_sha1(&sha1) {
                        // Find a ROM path that exists
                        if let Some(rom_path) = entry.rom_paths.iter().find(|p| p.exists()) {
                            if let Ok(rom) = std::fs::read(rom_path) {
                                return load::setup_game(self, rom_path.clone(), rom);
                            }
                        }
                    }
                }
            },
            Message::ScanComplete => {
                self.library_cache = library::view::LibraryCache::load();
                if self.settings.internet_enabled {
                    return Task::perform(
                        smol::unblock(|| library::scanner::enrich_library()),
                        |_| Message::EnrichComplete,
                    );
                }
            }
            Message::EnrichComplete => {
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
            }
            Message::ToggleGameInfo => {
                self.game_info_shown = !self.game_info_shown;
                if let Game::Loaded(LoadedGame::Emulator(emulator)) = &mut self.game {
                    emulator.reset_hover();
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
        if let Fullscreen::Active { cursor_hidden, .. } = self.fullscreen {
            let content = container(self.inner())
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
        } else if self.settings_shown {
            return settings_view::view(&self.settings);
        } else {
            column![
                self.action_bar.view(self),
                horizontal_rule(),
                container(self.inner()).center(Fill)
            ]
            .into()
        }
    }

    fn inner(&self) -> Element<'_, Message> {
        let fullscreen = matches!(self.fullscreen, Fullscreen::Active { .. });
        match &self.game {
            Game::Loaded(game) => {
                let has_info = self.current_game.is_some();
                let game_view = match game {
                    LoadedGame::Debugger(debugger) => debugger.view(),
                    LoadedGame::Emulator(emulator) => emulator.view(fullscreen, has_info),
                };

                if self.game_info_shown {
                    if let Some(current) = &self.current_game {
                        return row![
                            container(game_view).center(Fill),
                            library::info_panel::view(
                                &current.entry,
                                current.cover.as_ref(),
                            ),
                        ]
                        .into();
                    }
                }

                game_view
            }
            _ if !self.settings.setup_complete => self.setup_view(),
            _ => library::view::view(&self.library_cache),
        }
    }

    fn setup_view(&self) -> Element<'_, Message> {
        column![
            icons::xl(Icon::GameBoy)
                .width(120)
                .height(120)
                .style(|_, _| svg::Style { color: None }),
            text::xl("Welcome to Missingno"),
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
        .spacing(l())
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

    fn save(&self) {
        let Some(current) = &self.current_game else {
            return;
        };
        let cartridge = match &self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.game_boy().cartridge(),
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.game_boy().cartridge(),
            _ => return,
        };
        if !cartridge.has_battery() {
            return;
        }
        if let Some(ram) = cartridge.ram() {
            library::save_battery(&current.game_dir, &ram);
        }
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
