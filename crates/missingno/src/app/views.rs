use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Padding, Subscription, Task, event, mouse, time,
    widget::{Stack, center, column, container, mouse_area, opaque, row, scrollable, svg, text as iced_text},
    window,
};

use super::ui::{
    buttons, containers, fonts, horizontal_rule, menu_divider,
    icons::{self, Icon},
    palette::MUTED,
    sizes::{border_m, l, m, s},
    text,
};
use super::{
    controls, debugger, library, load, settings,
    App, CartridgeMessage, DetailMessage, DetailSubScreen, FlashState, Fullscreen, Game, LoadedGame, Message, PendingAction, Screen,
};
use crate::cartridge_rw;

impl App {
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
            (Screen::Settings { section, listening_for, .. }, _) => settings::view::view(
                &self.settings,
                *section,
                *listening_for,
                &self.detected_cartridge_devices,
            ),
            (Screen::ViewingGame { sub_screen: DetailSubScreen::Detail { .. }, .. }, _) => self.detail_view(),
            (Screen::ViewingGame { sub_screen: DetailSubScreen::CartridgeActions { .. }, .. }, _) => self.cartridge_actions_view(),
            (Screen::ViewingGame { sub_screen: DetailSubScreen::FlashCartridge { flash_state }, .. }, _) => self.flash_cartridge_view(flash_state),
            _ => {
                let page_content = self.page_content();
                column![
                    self.action_bar.view(self),
                    horizontal_rule(),
                    container(page_content).center(Fill)
                ]
                .into()
            }
        };

        // 2. Shell overlays — applied once regardless of screen
        let content = self.apply_toast(content);
        let content = self.apply_menu(content);
        self.apply_confirmation_dialog(content)
    }

    fn homebrew_enabled(&self) -> bool {
        self.settings.internet_enabled && self.settings.homebrew_hub_enabled
    }

    /// Library/Homebrew/ScreenshotGallery page content (no chrome).
    fn page_content(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Library { .. } => self.library_view(),
            Screen::HomebrewBrowser { state } => {
                library::homebrew_browser::view(state, &self.catalogue)
            }
            Screen::ViewingGame { sub_screen: DetailSubScreen::ScreenshotGallery { gallery_state }, .. } => {
                library::screenshot_gallery::view(gallery_state)
            }
            _ => unreachable!(),
        }
    }

    fn fullscreen_emulator_view(&self, cursor_hidden: bool) -> Element<'_, Message> {
        let screen = self.emulator_view(true);
        let content = container(screen)
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
    }

    fn apply_toast<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        if self.screenshot_toast.is_some() {
            Stack::with_children(vec![content, screenshot_toast()]).into()
        } else {
            content
        }
    }

    fn apply_menu<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        if !self.menu_open {
            return content;
        }

        let mut items = column![].spacing(2).width(220);
        let mut has_items = false;

        // Per-screen menu items
        match &self.screen {
            Screen::Library { .. } | Screen::HomebrewBrowser { .. } => {
                items = items.push(menu_item(Icon::FolderOpen, "Open ROM file...", load::Message::Pick.into()));
                has_items = true;
            }
            Screen::ViewingGame { sub_screen: DetailSubScreen::Detail { .. }, .. } => {
                items = items.push(menu_item(Icon::FolderOpen, "Open ROM file...", load::Message::Pick.into()));
                items = items.push(menu_divider());
                items = items.push(menu_item(Icon::Download, "Import Save...", Message::Detail(DetailMessage::ImportSave)));
                items = items.push(menu_item(Icon::FolderOpen, "Open Folder", Message::Detail(DetailMessage::OpenGameFolder)));
                items = items.push(menu_item(Icon::Globe, "Refresh Metadata", Message::Detail(DetailMessage::RefreshMetadata)));
                items = items.push(menu_divider());
                items = items.push(menu_item_danger(Icon::Close, "Remove Game", Message::Detail(DetailMessage::RemoveGame)));
                has_items = true;
            }
            Screen::Emulator => {
                if !self.debugger_enabled {
                    items = items.push(menu_item(Icon::Debug, "Debugger", Message::ToggleDebugger(true)));
                    items = items.push(menu_divider());
                }
                if self.debugger_enabled {
                    items = items.push(menu_item(Icon::Play, "Step Frame", debugger::Message::StepFrame.into()));
                }
                items = items.push(menu_item_danger(Icon::Close, "Reset", Message::Reset));
                items = items.push(menu_divider());
                items = items.push(menu_item(Icon::Camera, "Screenshot", Message::TakeScreenshot));
                if self.debugger_enabled {
                    items = items.push(menu_item(Icon::Download, "Capture Trace", debugger::Message::CaptureFrame.into()));
                }
                has_items = true;
            }
            _ => {}
        }

        // Settings always last
        if has_items {
            items = items.push(menu_divider());
        }
        items = items.push(menu_item(Icon::Gear, "Settings", Message::ShowSettings));

        let menu_panel = container(items.padding(s()))
            .style(containers::menu);

        // Anchor top-right: scrim covers everything, menu sits in corner
        Stack::new()
            .push(content)
            .push(opaque(
                mouse_area(
                    container(menu_panel)
                        .align_right(Fill)
                        .padding(Padding { top: m() + 40.0, right: m(), bottom: 0.0, left: 0.0 }),
                )
                .on_press(Message::DismissMenu),
            ))
            .into()
    }

    fn apply_confirmation_dialog<'a>(
        &self,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let Some(action) = &self.pending_action else {
            return content;
        };

        let (prompt, confirm_label) = match action {
            PendingAction::SwitchGame(_) => ("Close the current game and switch?", "Close Game"),
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
            .push(content)
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
                        .style(containers::menu),
                    )
                    .style(|_| container::Style {
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                        ..Default::default()
                    }),
                )
                .on_press(Message::DismissConfirm),
            ))
            .into()
    }

    fn detail_view(&self) -> Element<'_, Message> {
        let (viewing_sha1, hovered_log_entry, header_hovered) = match &self.screen {
            Screen::ViewingGame {
                sha1,
                sub_screen: DetailSubScreen::Detail { hovered_log_entry, header_hovered },
            } => (Some(sha1.as_str()), *hovered_log_entry, *header_hovered),
            _ => (None, None, false),
        };

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
            hovered_log_entry,
            header_hovered,
            is_loaded,
            inserted_cartridge: self.inserted_cartridge(),
        })
    }

    /// Navigate to the detail screen for a game, loading activity in background.
    pub(super) fn go_to_detail(&mut self, sha1: &str) -> Task<Message> {
        self.menu_open = false;
        self.store.mark_activity_loading(sha1);
        self.screen = Screen::ViewingGame {
            sha1: sha1.to_string(),
            sub_screen: DetailSubScreen::Detail {
                hovered_log_entry: None,
                header_hovered: false,
            },
        };
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

    fn cartridge_actions_view(&self) -> Element<'_, Message> {
        let cart = match self.inserted_cartridge() {
            Some(c) => c,
            None => {
                // Cart was disconnected — go back
                return container(
                    column![
                        screen_header("Cartridge", Message::Cartridge(CartridgeMessage::Back)),
                        container(iced_text("No cartridge inserted.").color(MUTED)).padding(l()),
                    ],
                )
                .height(Fill)
                .width(Fill)
                .into();
            }
        };

        let sha1 = self.viewing_sha1();
        let viewing_entry = sha1.and_then(|s| self.store.entry(s));
        let viewing_summary = sha1.and_then(|s| self.store.summary(s));

        let (flash_write_save, has_save) = match &self.screen {
            Screen::ViewingGame {
                sub_screen: DetailSubScreen::CartridgeActions { flash_write_save, has_save },
                ..
            } => (*flash_write_save, *has_save),
            _ => (true, false),
        };

        // Does the cart match the game we're viewing?
        let cart_matches = viewing_entry
            .and_then(|e| e.header_title.as_ref())
            .is_some_and(|ht| ht == &cart.title);

        // Find the cart's game in the library (may differ from viewing game)
        let cart_game = if cart_matches {
            viewing_summary
        } else {
            self.store.all_summaries().into_iter().find(|g| {
                g.entry
                    .header_title
                    .as_ref()
                    .is_some_and(|ht| ht == &cart.title)
            })
        };

        let max_width = if cart_matches { 600.0 } else { 900.0 };
        let mut body = column![].spacing(l()).max_width(max_width);

        if cart_matches {
            // ── Scenario 2: Cart matches the current game ──
            let title = viewing_entry.map(|e| e.display_title()).unwrap_or_default();
            let cover = viewing_summary.and_then(|s| s.thumbnail.as_ref());
            let hardware = if let Some(flash) = &cart.flash {
                let mut hw = format!("Flash {}", cartridge_rw::format_size(flash.size));
                if cart.ram_size > 0 {
                    hw.push_str(&format!(" · RAM {}", cartridge_rw::format_size(cart.ram_size)));
                }
                hw
            } else {
                format!("{} · {}", cart.mapper_name, cartridge_rw::format_size(cart.rom_size))
            };
            body = body.push(library::view::cartridge_tile(
                &title,
                &cart_subtitle(viewing_entry, &hardware),
                cover,
            ));

            // Save sync
            if cart.has_battery && cart.ram_size > 0 {
                body = body.push(self.save_sync_section(sha1));
            }

            // Reflash (troubleshooting)
            if cart.flashable() {
                if let Some(sha1) = sha1 {
                    let mut reflash_col = column![
                        text::label("Troubleshooting"),
                        iced_text("Reflash the ROM if the cartridge is not working correctly. Make sure your saves are synced first.").color(MUTED),
                    ]
                    .spacing(s());

                    if has_save && cart.ram_size > 0 {
                        reflash_col = reflash_col.push(
                            iced::widget::toggler(flash_write_save)
                                .label("Also write save to cartridge")
                                .on_toggle(|v| Message::Cartridge(CartridgeMessage::FlashToggleSave(v)))
                                .size(m()),
                        );
                    }

                    reflash_col = reflash_col.push(
                        buttons::subtle("Reflash ROM to Cartridge")
                            .on_press(Message::Cartridge(CartridgeMessage::Flash(sha1.to_string()))),
                    );

                    body = body.push(reflash_col);
                }
            }
        } else if cart.flashable() {
            // ── Scenario 3: Different game, flashable cart ──
            if let Some(sha1) = sha1 {
                let flash_title = viewing_entry.map(|e| e.display_title()).unwrap_or_default();
                let flash_cover = viewing_summary.and_then(|s| s.thumbnail.as_ref());

                let cart_title = if cart.title.is_empty() {
                    "Empty Flash Cart".to_string()
                } else {
                    cart.title.clone()
                };
                let cart_cover = cart_game.and_then(|g| g.thumbnail.as_ref());

                // Game file size from disk
                let rom_size = viewing_entry
                    .and_then(|e| e.rom_paths.first())
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| cartridge_rw::format_size(m.len() as u32))
                    .unwrap_or_default();

                // Side-by-side: game → cartridge
                let game_tile = library::view::game_tile(
                    &flash_title,
                    &rom_size,
                    flash_cover,
                );
                // Cart hardware info
                let flash_size = cart.flash.as_ref().map(|f| f.size).unwrap_or(0);
                let cart_hw = if cart.ram_size > 0 {
                    format!(
                        "Flash {} · RAM {}",
                        cartridge_rw::format_size(flash_size),
                        cartridge_rw::format_size(cart.ram_size),
                    )
                } else {
                    format!("Flash {}", cartridge_rw::format_size(flash_size))
                };

                let cart_entry = cart_game.map(|g| &g.entry);
                let cart_tile = library::view::cartridge_tile(
                    &cart_title,
                    &cart_subtitle(cart_entry, &cart_hw),
                    cart_cover,
                );
                let arrow = container(
                    icons::xl(Icon::Front)
                        .width(32)
                        .height(32)
                        .style(|_, _| svg::Style { color: Some(MUTED) }),
                )
                .center_y(library::view::COVER_HEIGHT);

                body = body.push(
                    row![game_tile, arrow, cart_tile]
                        .spacing(m())
                        .align_y(Center),
                );

                // Save toggle — show when the game has saves and the cart supports them
                if has_save && cart.ram_size > 0 {
                    body = body.push(
                        iced::widget::toggler(flash_write_save)
                            .label("Also write save to cartridge")
                            .on_toggle(|v| Message::Cartridge(CartridgeMessage::FlashToggleSave(v)))
                            .size(m()),
                    );
                }

                body = body.push(
                    column![
                        iced_text("This will erase the cartridge and replace it with this game's ROM.").color(MUTED),
                        buttons::danger("Erase and Write to Cartridge")
                            .on_press(Message::Cartridge(CartridgeMessage::Flash(sha1.to_string()))),
                    ]
                    .spacing(s()),
                );
            }
        }

        container(
            column![
                screen_header("Cartridge", Message::Cartridge(CartridgeMessage::Back)),
                container(scrollable(container(body).padding(l())).height(Fill))
                    .center_x(Fill),
            ],
        )
        .height(Fill)
        .width(Fill)
        .into()
    }

    /// Save sync status and buttons, used by the cartridge actions screen.
    fn save_sync_section(&self, sha1: Option<&str>) -> Element<'_, Message> {
        let sync_info = sha1.and_then(|s| {
            if let library::store::ActivityState::Loaded(detail) = self.store.activity_for(s) {
                detail.last_cart_sync.clone()
            } else {
                None
            }
        });

        let sync_status: Element<'_, Message> = if let Some((_, last_sync)) = &sync_info {
            iced_text(format!("Last synced {}", friendly_ago(*last_sync)))
                .color(MUTED)
                .into()
        } else {
            iced_text("Never synced with this cartridge.").color(MUTED).into()
        };

        column![
            text::label("Saves"),
            sync_status,
            row![
                buttons::standard("Import from Cartridge")
                    .on_press(Message::Cartridge(CartridgeMessage::ImportSave)),
                buttons::standard("Write to Cartridge")
                    .on_press(Message::Cartridge(CartridgeMessage::WriteSave)),
            ]
            .spacing(s()),
        ]
        .spacing(s())
        .into()
    }

    fn flash_cartridge_view(&self, flash_state: &FlashState) -> Element<'_, Message> {
        use crate::cartridge_rw;

        // Look up the game being flashed for the tile
        let sha1 = self.viewing_sha1();
        let game_entry = sha1.and_then(|s| self.store.entry(s));
        let game_title = game_entry
            .map(|e| e.display_title())
            .unwrap_or_default();
        let game_cover = sha1
            .and_then(|s| self.store.summary(s))
            .and_then(|s| s.thumbnail.as_ref());

        let content: Element<'_, Message> = match flash_state {
            FlashState::InProgress(progress) => {
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

                let mut body = column![
                    library::view::cartridge_tile(
                        &game_title,
                        &cart_subtitle(game_entry, "Writing to cartridge…"),
                        game_cover,
                    ),
                ]
                .spacing(s());

                match progress.phase {
                    cartridge_rw::FlashPhase::Erasing => {
                        body = body.push(iced_text("Erasing cartridge..."));
                    }
                    cartridge_rw::FlashPhase::Writing => {
                        body = body.push(text::progress_text(
                            "Writing…",
                            progress.bytes_done as u32,
                            progress.bytes_total as u32,
                            MUTED,
                        ));
                    }
                }

                if let Some(pct) = pct {
                    body = body.push(
                        iced::widget::progress_bar(0.0..=1.0, pct).girth(8),
                    );
                }

                body = body.push(
                    iced_text("Do not disconnect the cartridge or device.").color(MUTED),
                );

                column![
                    screen_header_no_back("Writing to Cartridge"),
                    container(body.max_width(600)).padding(l()),
                ]
                .into()
            }
            FlashState::Complete => {
                column![
                    screen_header_no_back("Write Complete"),
                    container(
                        column![
                            library::view::cartridge_tile(
                                &game_title,
                                &cart_subtitle(game_entry, "Written successfully"),
                                game_cover,
                            ),
                            buttons::primary("Done")
                                .on_press(Message::Cartridge(CartridgeMessage::FlashCancel)),
                        ]
                        .spacing(s())
                        .max_width(600),
                    )
                    .padding(l()),
                ]
                .into()
            }
            FlashState::Failed(error) => {
                column![
                    screen_header_no_back("Write Failed"),
                    container(
                        column![
                            iced_text(format!("Error: {error}")),
                            buttons::primary("Back").on_press(Message::Cartridge(CartridgeMessage::FlashCancel)),
                        ]
                        .spacing(s())
                        .max_width(600),
                    )
                    .padding(l()),
                ]
                .into()
            }
        };

        container(content).height(Fill).width(Fill).into()
    }

    fn library_view(&self) -> Element<'_, Message> {
        let hovered_game = match &self.screen {
            Screen::Library { hovered_game } => hovered_game.as_deref(),
            _ => None,
        };
        library::view::view(
            &self.store,
            hovered_game,
            self.inserted_cartridge(),
            self.cartridge_dump_progress.as_ref(),
            self.homebrew_enabled(),
        )
    }

    fn empty_detail_view(&self) -> Element<'_, Message> {
        self.library_view()
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

/// Build a cartridge tile subtitle combining library metadata with hardware info.
fn cart_subtitle(
    entry: Option<&library::GameEntry>,
    hardware: &str,
) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let publisher;
    let year;
    if let Some(e) = entry {
        if let Some(p) = &e.publisher {
            publisher = p.clone();
            parts.push(&publisher);
        }
        if let Some(y) = &e.year {
            year = library::activity::format_date_string(y);
            parts.push(&year);
        }
    }
    parts.push(hardware);
    parts.join(" · ")
}

fn menu_item<'a>(icon: Icon, label: &'a str, message: Message) -> Element<'a, Message> {
    buttons::subtle(
        row![icons::m(icon), label]
            .spacing(s())
            .align_y(Center),
    )
    .on_press(Message::MenuAction(Box::new(message)))
    .width(Fill)
    .into()
}

fn menu_item_danger<'a>(icon: Icon, label: &'a str, message: Message) -> Element<'a, Message> {
    buttons::danger(
        row![icons::m(icon), label]
            .spacing(s())
            .align_y(Center),
    )
    .on_press(Message::MenuAction(Box::new(message)))
    .width(Fill)
    .into()
}

/// Standard screen header: back button + title + horizontal rule.
fn screen_header<'a>(title: &'a str, back_message: Message) -> Element<'a, Message> {
    column![
        row![
            buttons::subtle(icons::m(Icon::Back)).on_press(back_message),
            text::heading(title),
        ]
        .spacing(s())
        .padding(m())
        .align_y(Center),
        horizontal_rule(),
    ]
    .into()
}

/// Screen header without a back button (for non-cancellable states like progress).
fn screen_header_no_back<'a>(title: &'a str) -> Element<'a, Message> {
    column![
        row![text::heading(title)]
            .spacing(s())
            .padding(m())
            .align_y(Center),
        horizontal_rule(),
    ]
    .into()
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
            border: iced::Border::default().rounded(border_m()),
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
