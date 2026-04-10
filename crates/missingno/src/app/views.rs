use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, Task, event, mouse, time,
    widget::{Stack, center, column, container, mouse_area, opaque, row, svg, text as iced_text},
    window,
};

use super::ui::{
    buttons, fonts, horizontal_rule,
    icons::{self, Icon},
    sizes::{l, m, s},
    text,
};
use super::{
    controls, library, settings,
    App, FlashState, Fullscreen, Game, LoadedGame, Message, PendingAction, Screen,
};
use crate::cartridge_rw;

impl App {
    pub fn view(&self) -> Element<'_, Message> {
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
            settings::view::view(
                &self.settings,
                self.settings_section,
                self.listening_for,
                &self.detected_cartridge_devices,
            )
        } else {
            match self.screen {
                Screen::Detail => self.detail_view(),
                Screen::FlashCartridge => self.flash_cartridge_view(),
                _ => {
                    let inserted_cartridge = self.inserted_cartridge();
                    let dump_progress = self.cartridge_dump_progress.as_ref();
                    let content = match self.screen {
                        Screen::Library => {
                            library::view::view(
                                &self.store,
                                self.hovered_library_game.as_deref(),
                                inserted_cartridge,
                                dump_progress,
                            )
                        }
                        Screen::HomebrewBrowser => {
                            if let Some(state) = &self.homebrew_browser {
                                library::homebrew_browser::view(state, &self.catalogue)
                            } else {
                                library::view::view(
                                    &self.store,
                                    self.hovered_library_game.as_deref(),
                                    inserted_cartridge,
                                    dump_progress,
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
            inserted_cartridge: self.inserted_cartridge(),
        })
    }

    /// Navigate to the detail screen for a game, loading activity in background.
    pub(super) fn go_to_detail(&mut self, sha1: &str) -> Task<Message> {
        self.store.mark_activity_loading(sha1);
        self.viewing_sha1 = Some(sha1.to_string());
        self.screen = Screen::Detail;
        self.load_activity_async(sha1)
    }

    /// Kick off a background load of activity detail for a game.
    pub(super) fn load_activity_async(&self, sha1: &str) -> Task<Message> {
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

    /// Get the cartridge header from the first connected device with a cartridge inserted.
    pub(super) fn inserted_cartridge(&self) -> Option<&cartridge_rw::CartridgeHeader> {
        self.detected_cartridge_devices
            .iter()
            .find_map(|d| d.cartridge.as_ref())
    }

    fn flash_cartridge_view(&self) -> Element<'_, Message> {
        use crate::cartridge_rw;

        // Catppuccin Mocha subtext0
        const MUTED: iced::Color = iced::Color::from_rgb(
            0xa6 as f32 / 255.0,
            0xad as f32 / 255.0,
            0xc8 as f32 / 255.0,
        );

        let content: Element<'_, Message> = match &self.flash_state {
            Some(FlashState::Confirming {
                game_title,
                rom_size,
                cart_title,
                flash_size,
                ..
            }) => {
                column![
                    row![
                        buttons::subtle(icons::m(Icon::Back))
                            .on_press(Message::FlashCartridgeCancel),
                        text::heading("Write to Cartridge"),
                    ]
                    .spacing(s())
                    .padding(m())
                    .align_y(Center),
                    horizontal_rule(),
                    container(
                        column![
                            text::label("ROM to write"),
                            iced_text(format!(
                                "{game_title} ({})",
                                cartridge_rw::format_size(*rom_size)
                            )),
                            iced::widget::Space::new().height(s()),
                            text::label("Currently on cartridge"),
                            iced_text(format!(
                                "{cart_title} (flash chip: {})",
                                cartridge_rw::format_size(*flash_size)
                            )),
                            iced::widget::Space::new().height(s()),
                            iced_text("This will erase all data on the cartridge.").color(MUTED),
                            iced::widget::Space::new().height(s()),
                            row![
                                buttons::standard("Cancel")
                                    .on_press(Message::FlashCartridgeCancel),
                                buttons::danger("Erase and Write")
                                    .on_press(Message::FlashCartridgeConfirm),
                            ]
                            .spacing(s()),
                        ]
                        .spacing(s())
                        .max_width(600),
                    )
                    .padding(l()),
                ]
                .into()
            }
            Some(FlashState::InProgress(progress)) => {
                let pct = match progress.phase {
                    cartridge_rw::FlashPhase::Erasing => None,
                    cartridge_rw::FlashPhase::Writing => Some(
                        if progress.bytes_total > 0 {
                            progress.bytes_done as f32 / progress.bytes_total as f32
                        } else {
                            0.0
                        },
                    ),
                };

                let mut progress_col = column![].spacing(s());

                match progress.phase {
                    cartridge_rw::FlashPhase::Erasing => {
                        progress_col = progress_col.push(iced_text("Erasing cartridge..."));
                    }
                    cartridge_rw::FlashPhase::Writing => {
                        progress_col = progress_col.push(text::progress_text(
                            "Writing…",
                            progress.bytes_done as u32,
                            progress.bytes_total as u32,
                            MUTED,
                        ));
                    }
                }

                if let Some(pct) = pct {
                    progress_col = progress_col.push(
                        iced::widget::progress_bar(0.0..=1.0, pct).girth(8),
                    );
                }

                progress_col = progress_col.push(
                    iced_text("Do not disconnect the cartridge or device.").color(MUTED),
                );

                column![
                    row![text::heading("Writing to Cartridge"),]
                        .spacing(s())
                        .padding(m())
                        .align_y(Center),
                    horizontal_rule(),
                    container(progress_col.max_width(600)).padding(l()),
                ]
                .into()
            }
            Some(FlashState::Complete) => {
                column![
                    row![text::heading("Write Complete"),]
                        .spacing(s())
                        .padding(m())
                        .align_y(Center),
                    horizontal_rule(),
                    container(
                        column![
                            iced_text("ROM written successfully."),
                            buttons::primary("Done")
                                .on_press(Message::FlashCartridgeCancel),
                        ]
                        .spacing(s())
                        .max_width(600),
                    )
                    .padding(l()),
                ]
                .into()
            }
            Some(FlashState::Failed(error)) => {
                column![
                    row![text::heading("Write Failed"),]
                        .spacing(s())
                        .padding(m())
                        .align_y(Center),
                    horizontal_rule(),
                    container(
                        column![
                            iced_text(format!("Error: {error}")),
                            buttons::primary("Back").on_press(Message::FlashCartridgeCancel),
                        ]
                        .spacing(s())
                        .max_width(600),
                    )
                    .padding(l()),
                ]
                .into()
            }
            None => {
                // Shouldn't happen — redirect to library
                self.empty_detail_view()
            }
        };

        container(content).height(Fill).width(Fill).into()
    }

    fn empty_detail_view(&self) -> Element<'_, Message> {
        library::view::view(
            &self.store,
            self.hovered_library_game.as_deref(),
            self.inserted_cartridge(),
            self.cartridge_dump_progress.as_ref(),
        )
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

    pub fn subscription(&self) -> Subscription<Message> {
        let listening_keyboard = matches!(
            self.listening_for,
            Some(settings::view::ListeningFor::Keyboard(_))
        );
        let listening_gamepad = matches!(
            self.listening_for,
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
            if self.settings.cartridge_rw_enabled {
                time::every(std::time::Duration::from_secs(2))
                    .map(|_| Message::CartridgeRwPoll)
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
