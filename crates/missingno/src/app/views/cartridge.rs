use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    widget::{column, container, row, scrollable, svg, text as iced_text},
};

use crate::app::ui::{
    buttons, horizontal_rule,
    icons::{self, Icon},
    palette::MUTED,
    sizes::{l, m, s},
    text,
};
use crate::app::{App, CartridgeMessage, DetailSubScreen, FlashState, Message, Screen, library};
use crate::cartridge_rw;
use super::friendly_ago;

impl App {
    /// Get the cartridge header from the first connected device with a cartridge inserted.
    pub(super) fn inserted_cartridge(&self) -> Option<&cartridge_rw::CartridgeHeader> {
        self.detected_cartridge_devices
            .iter()
            .find_map(|d| d.cartridge.as_ref())
    }

    pub(super) fn cartridge_actions_view(&self) -> Element<'_, Message> {
        let cart = match self.inserted_cartridge() {
            Some(c) => c,
            None => {
                // Cart was disconnected — go back
                return container(column![
                    screen_header("Cartridge", Message::Cartridge(CartridgeMessage::Back)),
                    container(iced_text("No cartridge inserted.").color(MUTED)).padding(l()),
                ])
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
                sub_screen:
                    DetailSubScreen::CartridgeActions {
                        flash_write_save,
                        has_save,
                    },
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
                    hw.push_str(&format!(
                        " · RAM {}",
                        cartridge_rw::format_size(cart.ram_size)
                    ));
                }
                hw
            } else {
                format!(
                    "{} · {}",
                    cart.mapper_name,
                    cartridge_rw::format_size(cart.rom_size)
                )
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
                                .on_toggle(|v| {
                                    Message::Cartridge(CartridgeMessage::FlashToggleSave(v))
                                })
                                .size(m()),
                        );
                    }

                    reflash_col =
                        reflash_col.push(buttons::subtle("Reflash ROM to Cartridge").on_press(
                            Message::Cartridge(CartridgeMessage::Flash(sha1.to_string())),
                        ));

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
                let game_tile = library::view::game_tile(&flash_title, &rom_size, flash_cover);
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
                        iced_text(
                            "This will erase the cartridge and replace it with this game's ROM."
                        )
                        .color(MUTED),
                        buttons::danger("Erase and Write to Cartridge").on_press(
                            Message::Cartridge(CartridgeMessage::Flash(sha1.to_string()))
                        ),
                    ]
                    .spacing(s()),
                );
            }
        }

        container(column![
            screen_header("Cartridge", Message::Cartridge(CartridgeMessage::Back)),
            container(scrollable(container(body).padding(l())).height(Fill)).center_x(Fill),
        ])
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
            iced_text("Never synced with this cartridge.")
                .color(MUTED)
                .into()
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

    pub(super) fn flash_cartridge_view(&self, flash_state: &FlashState) -> Element<'_, Message> {
        use crate::cartridge_rw;

        // Look up the game being flashed for the tile
        let sha1 = self.viewing_sha1();
        let game_entry = sha1.and_then(|s| self.store.entry(s));
        let game_title = game_entry.map(|e| e.display_title()).unwrap_or_default();
        let game_cover = sha1
            .and_then(|s| self.store.summary(s))
            .and_then(|s| s.thumbnail.as_ref());

        let content: Element<'_, Message> = match flash_state {
            FlashState::InProgress(progress) => {
                let pct = match progress.phase {
                    cartridge_rw::FlashPhase::Erasing => None,
                    cartridge_rw::FlashPhase::Writing => Some(if progress.bytes_total > 0 {
                        progress.bytes_done as f32 / progress.bytes_total as f32
                    } else {
                        0.0
                    }),
                };

                let mut body = column![library::view::cartridge_tile(
                    &game_title,
                    &cart_subtitle(game_entry, "Writing to cartridge…"),
                    game_cover,
                ),]
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
                    body = body.push(iced::widget::progress_bar(0.0..=1.0, pct).girth(8));
                }

                body =
                    body.push(iced_text("Do not disconnect the cartridge or device.").color(MUTED));

                column![
                    screen_header_no_back("Writing to Cartridge"),
                    container(body.max_width(600)).padding(l()),
                ]
                .into()
            }
            FlashState::Complete => column![
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
            .into(),
            FlashState::Failed(error) => column![
                screen_header_no_back("Write Failed"),
                container(
                    column![
                        iced_text(format!("Error: {error}")),
                        buttons::primary("Back")
                            .on_press(Message::Cartridge(CartridgeMessage::FlashCancel)),
                    ]
                    .spacing(s())
                    .max_width(600),
                )
                .padding(l()),
            ]
            .into(),
        };

        container(content).height(Fill).width(Fill).into()
    }
}

/// Build a cartridge tile subtitle combining library metadata with hardware info.
fn cart_subtitle(entry: Option<&library::GameEntry>, hardware: &str) -> String {
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
