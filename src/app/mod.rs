use std::{fs, path::PathBuf, time::Instant};

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, Task, Theme, event, mouse, time,
    widget::{
        Stack, button, center, column, container, mouse_area, opaque, row, svg, text as iced_text,
    },
    window,
};
use replace_with::replace_with_or_abort;

use crate::game_boy::{
    GameBoy,
    cartridge::Cartridge,
    joypad::{self, Button},
    video::palette::PaletteChoice,
};
use action_bar::ActionBar;
use audio_output::AudioOutput;
use core::{
    fonts, horizontal_rule,
    icons::{self, Icon},
    sizes::{l, s, xl},
    text,
};

mod action_bar;
mod audio_output;
mod controls;
mod core;
mod debugger;
mod emulator;
mod load;
mod recent;
mod screen;
pub mod settings;
mod texture_renderer;

pub fn run(rom_path: Option<PathBuf>, debugger: bool) -> iced::Result {
    iced::application(
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
    .theme(App::theme)
    .exit_on_close_request(false)
    .run()
}

struct App {
    game: Game,
    debugger_enabled: bool,
    fullscreen: Fullscreen,
    action_bar: ActionBar,
    audio_output: Option<AudioOutput>,
    save_path: Option<PathBuf>,
    recent_games: recent::RecentGames,
    settings: settings::Settings,
    about_shown: bool,
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

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),

    Run,
    Pause,
    Reset,

    PressButton(joypad::Button),
    ReleaseButton(joypad::Button),

    ToggleDebugger(bool),
    SelectPalette(PaletteChoice),
    ShowAbout,
    DismissAbout,
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
        let mut recent_games = recent::RecentGames::load();

        let (game, save_path) = match rom_path {
            Some(rom_path) => {
                let sav_path = load::save_path(&rom_path);
                let save_data = fs::read(&sav_path).ok();
                let cartridge = Cartridge::new(fs::read(&rom_path).unwrap(), save_data);
                recent_games.add(rom_path, cartridge.title().to_string());
                recent_games.save();
                let game_boy = GameBoy::new(cartridge);
                let game = Game::Loaded(if debugger {
                    let mut dbg = debugger::Debugger::new(game_boy);
                    dbg.set_palette(settings.palette);
                    LoadedGame::Debugger(dbg)
                } else {
                    let mut emu = emulator::Emulator::new(game_boy);
                    emu.set_palette(settings.palette);
                    emu.run();
                    LoadedGame::Emulator(emu)
                });
                (game, Some(sav_path))
            }

            None => (Game::Unloaded, None),
        };

        (
            Self {
                game,
                debugger_enabled: debugger,
                fullscreen: Fullscreen::Windowed,
                action_bar: ActionBar::new(),
                audio_output: AudioOutput::new(),
                save_path,
                recent_games,
                settings,
                about_shown: false,
            },
            Task::none(),
        )
    }

    fn title(&self) -> String {
        if let Game::Loaded(game) = &self.game {
            match game {
                LoadedGame::Debugger(debugger) => {
                    format!("{} - MissingNo.", debugger.game_boy().cartridge().title())
                }
                LoadedGame::Emulator(emulator) => {
                    format!("{} - MissingNo.", emulator.game_boy().cartridge().title())
                }
            }
        } else {
            "MissingNo.".into()
        }
    }

    fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Load(message) => return load::update(message, self),

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
            Message::ShowAbout => self.about_shown = true,
            Message::DismissAbout => self.about_shown = false,
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
        } else {
            let main = column![
                self.action_bar.view(self),
                horizontal_rule(),
                container(self.inner()).center(Fill)
            ];

            if self.about_shown {
                Stack::new()
                    .push(main)
                    .push(opaque(
                        mouse_area(
                            center(
                                container(
                                    column![
                                        icons::xl(Icon::GameBoy)
                                            .width(80)
                                            .height(80)
                                            .style(|_, _| svg::Style { color: None }),
                                        text::xl("MissingNo."),
                                        iced_text(format!(
                                            "Version {}",
                                            env!("CARGO_PKG_VERSION").trim_end_matches(".0")
                                        )),
                                        iced_text("A Game Boy emulator and debugger"),
                                        iced_text("by Andrew O'Neil"),
                                        mouse_area(
                                            row![
                                                icons::m(Icon::Globe),
                                                iced_text("andyofniall.net"),
                                            ]
                                            .spacing(s())
                                            .align_y(Center),
                                        )
                                        .on_press(Message::OpenUrl("https://andyofniall.net/"))
                                        .interaction(mouse::Interaction::Pointer),
                                        mouse_area(
                                            row![
                                                icons::m(Icon::GitHub),
                                                iced_text("github.com/ajoneil/missingno"),
                                            ]
                                            .spacing(s())
                                            .align_y(Center),
                                        )
                                        .on_press(Message::OpenUrl(
                                            "https://github.com/ajoneil/missingno",
                                        ))
                                        .interaction(mouse::Interaction::Pointer),
                                        button("Close").on_press(Message::DismissAbout),
                                    ]
                                    .spacing(l())
                                    .align_x(Center),
                                )
                                .padding(xl())
                                .style(container::bordered_box),
                            )
                            .style(|_| container::Style {
                                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                                ..Default::default()
                            }),
                        )
                        .on_press(Message::DismissAbout),
                    ))
                    .into()
            } else {
                main.into()
            }
        }
    }

    fn inner(&self) -> Element<'_, Message> {
        let fullscreen = matches!(self.fullscreen, Fullscreen::Active { .. });
        match &self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.view(),
                LoadedGame::Emulator(emulator) => emulator.view(fullscreen),
            },
            _ => {
                let mut col = column![
                    text::xl("Welcome to MissingNo.!"),
                    icons::xl(Icon::GameBoy)
                        .width(200)
                        .height(200)
                        .style(|_, _| svg::Style { color: None })
                ]
                .align_x(Center)
                .spacing(l());

                if !self.recent_games.is_empty() {
                    col = col.push(self.recent_games.view());
                }

                col.into()
            }
        }
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
        let Some(save_path) = &self.save_path else {
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
            let _ = fs::write(save_path, ram);
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
