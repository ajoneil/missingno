use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, event, time,
    widget::{column, container, row, svg, text as iced_text},
    window,
};

use super::ui::{
    buttons, horizontal_rule,
    icons::{self, Icon},
    sizes::{l, s},
    text,
};
use super::{
    App, DetailSubScreen, Fullscreen, Game, LoadedGame, Message, Screen, controls, settings,
};

mod cartridge;
mod emulator;
mod library;
mod shell;

impl App {
    /// The display technology of the currently loaded console, if any — used to
    /// show only the matching cosmetic overlay option in settings.
    fn current_technology(&self) -> Option<missingno_core::video::DisplayTechnology> {
        match &self.game {
            Game::Loaded(LoadedGame::Emulator(emu)) => Some(emu.technology()),
            Game::Loaded(LoadedGame::Debugger(dbg)) => Some(dbg.technology()),
            _ => None,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        // First-boot setup
        if !self.settings.setup_complete {
            return self.setup_view();
        }

        // 1. Screen content — each screen owns its own chrome
        let content: Element<'_, Message> = match (&self.screen, &self.fullscreen) {
            (Screen::Emulator, Fullscreen::Active { cursor_hidden, .. }) => {
                self.fullscreen_emulator_view(*cursor_hidden)
            }
            (Screen::Emulator, _) => {
                let screen = container(self.emulator_view(false)).center(Fill);
                column![self.action_bar.view(self), horizontal_rule(), screen].into()
            }
            (
                Screen::Settings {
                    section,
                    listening_for,
                    ..
                },
                _,
            ) => settings::view::view(
                &self.settings,
                *section,
                *listening_for,
                &self.cartridge_rw.detected_devices,
                self.current_technology(),
            ),
            (
                Screen::ViewingGame {
                    sub_screen: DetailSubScreen::Detail { .. },
                    ..
                },
                _,
            ) => self.detail_view(),
            (
                Screen::ViewingGame {
                    sub_screen: DetailSubScreen::CartridgeActions { .. },
                    ..
                },
                _,
            ) => self.cartridge_actions_view(),
            (
                Screen::ViewingGame {
                    sub_screen: DetailSubScreen::FlashCartridge { flash_state },
                    ..
                },
                _,
            ) => self.flash_cartridge_view(flash_state),
            _ => {
                let page_content = self.page_content();
                let mut col = column![
                    self.action_bar.view(self),
                    horizontal_rule(),
                    container(page_content).center(Fill),
                ];
                if matches!(self.screen, Screen::Library { .. })
                    && let Some(bar) = self.missing_rom_dirs_bar()
                {
                    col = col.push(bar);
                }
                col.into()
            }
        };

        // 2. Shell overlays — applied once regardless of screen
        let content = self.apply_toast(content);
        let content = self.apply_menu(content);
        self.apply_confirmation_dialog(content)
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

    pub fn subscription(&self) -> Subscription<Message> {
        let listening_for = self.listening_for();
        let listening_keyboard = matches!(
            listening_for,
            Some(settings::view::ListeningFor::Keyboard(_))
        );
        let listening_gamepad = matches!(
            listening_for,
            Some(settings::view::ListeningFor::Gamepad(_))
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
            // One app-lifetime subscription bridges the current session's events
            // into the UI: its first item hands over the sink a per-game bridge
            // thread forwards through. Always included so the sink persists.
            Subscription::run(super::session_bridge::session_events_worker).map(Message::Session),
            if self.screenshot_toast.is_some() {
                time::every(std::time::Duration::from_millis(1500))
                    .map(|_| Message::DismissScreenshotToast)
            } else {
                Subscription::none()
            },
            if self.notice.is_some() {
                time::every(std::time::Duration::from_millis(3000)).map(|_| Message::DismissNotice)
            } else {
                Subscription::none()
            },
            if self.settings.cartridge_rw_enabled {
                time::every(std::time::Duration::from_secs(2)).map(|_| Message::CartridgeRwPoll)
            } else {
                Subscription::none()
            },
        ])
    }
}

pub(super) fn friendly_ago(timestamp: jiff::Timestamp) -> String {
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
