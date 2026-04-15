use iced::Task;

use missingno_gb::cartridge::Cartridge;

use crate::app::{self, DetailSubScreen, FlashState, Game, Screen, load};
use crate::cartridge_rw;

use super::{homebrew_browser, screenshot_gallery};

pub(in crate::app) fn handle_library_message(
    app: &mut app::App,
    message: super::view::Message,
) -> Task<app::Message> {
    match message {
        super::view::Message::SelectGame(sha1) => {
            return app.go_to_detail(&sha1);
        }
        super::view::Message::HoverGame(sha1) => {
            if let Screen::Library { hovered_game, .. } = &mut app.screen {
                *hovered_game = Some(sha1);
            }
        }
        super::view::Message::UnhoverGame => {
            if let Screen::Library { hovered_game, .. } = &mut app.screen {
                *hovered_game = None;
            }
        }
        super::view::Message::DumpCartridge => {
            // Find the first device with a cartridge
            if let Some(device) = app
                .detected_cartridge_devices
                .iter()
                .find(|d| d.cartridge.is_some())
            {
                let port_name = device.port_name.clone();
                let header = device.cartridge.clone().unwrap();
                app.cartridge_dump_progress = Some(cartridge_rw::DumpProgress {
                    bytes_done: 0,
                    bytes_total: header.rom_size as usize,
                });

                let (tx, rx) = smol::channel::bounded(32);
                // Progress subscription
                let progress_task = Task::run(
                    smol::stream::unfold(rx, |rx| async { rx.recv().await.ok().map(|p| (p, rx)) }),
                    app::Message::CartridgeRwDumpProgress,
                );

                let dump_task = Task::perform(
                    smol::unblock(move || {
                        let rom = cartridge_rw::dump_rom(&port_name, &header, &mut |p| {
                            let _ = tx.send_blocking(p);
                        })?;
                        // Also read SRAM if the cartridge has battery-backed save
                        let sram = if header.has_battery && header.ram_size > 0 {
                            match cartridge_rw::read_sram(&port_name, &header) {
                                Ok(data) => Some(data),
                                Err(_) => None,
                            }
                        } else {
                            None
                        };
                        Ok((rom, sram))
                    }),
                    app::Message::CartridgeRwDumpComplete,
                );

                return Task::batch([dump_task, progress_task]);
            }
        }
        super::view::Message::QuickPlay(sha1) => {
            let same_game = app
                .current_game
                .as_ref()
                .map(|c| c.entry.sha1 == sha1)
                .unwrap_or(false);

            if same_game {
                // Already loaded, just resume
                app.run();
                app.screen = Screen::Emulator;
            } else if matches!(app.game, Game::Loaded(_)) {
                // Different game loaded, confirm first
                app.pending_action = Some(app::PendingAction::SwitchGame(sha1));
            } else {
                // Nothing loaded, go ahead
                load::select_game(app, &sha1);
                return load::play_current_game(app);
            }
        }
    }

    Task::none()
}

pub(in crate::app) fn handle(app: &mut app::App, message: app::Message) -> Task<app::Message> {
    match message {
        // Detail screen messages
        app::Message::Detail(detail_msg) => {
            use app::DetailMessage::*;
            match detail_msg {
                OpenGameFolder => {
                    if let Some(sha1) = app.viewing_sha1() {
                        if let Some(dir) = super::find_by_sha1(sha1).map(|(d, _)| d) {
                            let _ = open::that(&dir);
                        }
                    }
                }
                RefreshMetadata => {
                    if let Some(sha1) = app.viewing_sha1().map(|s| s.to_string()) {
                        return Task::perform(
                            smol::unblock(move || super::hasheous::lookup(&sha1).ok().flatten()),
                            move |info| {
                                if let Some(info) = info {
                                    app::Message::Detail(GameMetadataRefreshed(info))
                                } else {
                                    app::Message::None
                                }
                            },
                        );
                    }
                }
                GameMetadataRefreshed(info) => {
                    if let Some(sha1) = app.viewing_sha1().map(str::to_owned) {
                        if let Some((game_dir, mut entry)) = super::find_by_sha1(&sha1) {
                            entry.title = info.name;
                            entry.platform = info.platform;
                            entry.publisher = info.publisher;
                            entry.year = info.year;
                            entry.description = info.description;
                            entry.wikipedia_url = info.wikipedia_url;
                            entry.igdb_url = info.igdb_url;
                            entry.enrichment_attempted = true;
                            super::save_entry(&game_dir, &entry);
                            if let Some(bytes) = &info.cover_art {
                                super::save_cover(&game_dir, bytes);
                            }
                            app.store.notify_metadata_changed(&sha1);
                        }
                    }
                }
                ImportSave => {
                    let dialog = rfd::AsyncFileDialog::new().add_filter("Game Boy Save", &["sav"]);
                    return Task::perform(dialog.pick_file(), |handle| {
                        app::Message::Detail(ImportSaveSelected(handle))
                    });
                }
                ImportSaveSelected(handle) => {
                    if let (Some(handle), Some(sha1)) = (handle, app.viewing_sha1()) {
                        if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                            if let Ok(data) = std::fs::read(handle.path()) {
                                super::activity::write_import(&game_dir, &data);
                            }
                        }
                    }
                }
                PlayWithSave(save_id) => {
                    // Launch the game with a specific save
                    if let Some(sha1) = app.viewing_sha1().map(|s| s.to_string()) {
                        let same_game = app
                            .current_game
                            .as_ref()
                            .map(|c| c.entry.sha1 == sha1)
                            .unwrap_or(false);

                        if matches!(app.game, Game::Loaded(_)) && !same_game {
                            // Different game loaded — would need confirmation
                            // For now, just go to the detail page
                        } else {
                            if !same_game || !matches!(app.game, Game::Loaded(_)) {
                                load::select_game(app, &sha1);
                            }
                            return load::play_with_save(app, &save_id);
                        }
                    }
                }
                ExportSave(save_id) => {
                    let dialog = rfd::AsyncFileDialog::new()
                        .set_file_name("save.sav")
                        .add_filter("Game Boy Save", &["sav"]);
                    return Task::perform(dialog.save_file(), move |handle| {
                        app::Message::Detail(ExportSaveSelected(save_id.clone(), handle))
                    });
                }
                ExportSaveSelected(save_id, handle) => {
                    if let (Some(handle), Some(sha1)) = (handle, app.viewing_sha1()) {
                        if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                            if let Some(data) = super::activity::load_sram_from(&game_dir, &save_id)
                            {
                                let _ = std::fs::write(handle.path(), data);
                            }
                        }
                    }
                }
                OpenScreenshotGallery(session_filename, screenshot_idx) => {
                    if let Screen::ViewingGame { sha1, .. } = &app.screen {
                        if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                            if let Some(mut state) =
                                screenshot_gallery::GalleryState::load(&game_dir, &session_filename)
                            {
                                state.select(screenshot_idx);
                                let sha1 = sha1.clone();
                                app.screen = Screen::ViewingGame {
                                    sha1,
                                    sub_screen: DetailSubScreen::ScreenshotGallery {
                                        gallery_state: state,
                                    },
                                };
                            }
                        }
                    }
                }
                HoverLogEntry(idx) => {
                    if let Screen::ViewingGame {
                        sub_screen:
                            DetailSubScreen::Detail {
                                hovered_log_entry, ..
                            },
                        ..
                    } = &mut app.screen
                    {
                        *hovered_log_entry = Some(idx);
                    }
                }
                UnhoverLogEntry => {
                    if let Screen::ViewingGame {
                        sub_screen:
                            DetailSubScreen::Detail {
                                hovered_log_entry, ..
                            },
                        ..
                    } = &mut app.screen
                    {
                        *hovered_log_entry = None;
                    }
                }
                HoverHeader => {
                    if let Screen::ViewingGame {
                        sub_screen: DetailSubScreen::Detail { header_hovered, .. },
                        ..
                    } = &mut app.screen
                    {
                        *header_hovered = true;
                    }
                }
                UnhoverHeader => {
                    if let Screen::ViewingGame {
                        sub_screen: DetailSubScreen::Detail { header_hovered, .. },
                        ..
                    } = &mut app.screen
                    {
                        *header_hovered = false;
                    }
                }
                RemoveGame => {
                    app.pending_action = Some(app::PendingAction::RemoveGameFromLibrary);
                }
            }
        }

        // Cartridge messages
        app::Message::Cartridge(cart_msg) => {
            use app::CartridgeMessage::*;
            match cart_msg {
                ShowActions(sha1) => {
                    let has_save = super::find_by_sha1(&sha1)
                        .and_then(|(game_dir, _)| super::activity::load_current_sram(&game_dir))
                        .is_some();
                    app.screen = Screen::ViewingGame {
                        sha1,
                        sub_screen: DetailSubScreen::CartridgeActions {
                            flash_write_save: has_save,
                            has_save,
                        },
                    };
                }
                Back => {
                    if let Screen::ViewingGame { sha1, .. } = &app.screen {
                        let sha1 = sha1.clone();
                        return app.go_to_detail(&sha1);
                    }
                    app.screen = Screen::Library { hovered_game: None };
                }
                ImportSave => {
                    if let Some(device) = app.detected_cartridge_devices.iter().find(|d| {
                        d.cartridge
                            .as_ref()
                            .is_some_and(|c| c.has_battery && c.ram_size > 0)
                    }) {
                        let port_name = device.port_name.clone();
                        let header = device.cartridge.clone().unwrap();
                        return Task::perform(
                            smol::unblock(move || cartridge_rw::read_sram(&port_name, &header)),
                            |result| app::Message::Cartridge(ImportSaveComplete(result)),
                        );
                    }
                }
                ImportSaveComplete(result) => match result {
                    Ok(sram) => {
                        if let Some(sha1) = app.viewing_sha1().map(str::to_owned) {
                            if let Some((game_dir, _)) = super::find_by_sha1(&sha1) {
                                super::activity::write_cartridge_import(&game_dir, &sram);
                                app.store.notify_activity_changed(&sha1);
                            }
                        }
                    }
                    Err(_) => {}
                },
                WriteSave => {
                    if let Some(sha1) = app.viewing_sha1() {
                        if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                            if let Some(sram) = super::activity::load_current_sram(&game_dir) {
                                if let Some(device) =
                                    app.detected_cartridge_devices.iter().find(|d| {
                                        d.cartridge
                                            .as_ref()
                                            .is_some_and(|c| c.has_battery && c.ram_size > 0)
                                    })
                                {
                                    let port_name = device.port_name.clone();
                                    let header = device.cartridge.clone().unwrap();
                                    return Task::perform(
                                        smol::unblock(move || {
                                            cartridge_rw::write_sram(&port_name, &header, &sram)?;
                                            Ok(sram)
                                        }),
                                        |result| app::Message::Cartridge(WriteSaveComplete(result)),
                                    );
                                }
                            }
                        }
                    }
                }
                WriteSaveComplete(result) => match result {
                    Ok(sram) => {
                        if let Some(sha1) = app.viewing_sha1().map(str::to_owned) {
                            if let Some((game_dir, _)) = super::find_by_sha1(&sha1) {
                                super::activity::write_cart_write(&game_dir, &sram);
                                app.store.notify_activity_changed(&sha1);
                            }
                        }
                    }
                    Err(_) => {}
                },
                Flash(sha1) => {
                    // Read write_save preference from the CartridgeActions screen
                    let write_save = matches!(
                        &app.screen,
                        Screen::ViewingGame {
                            sub_screen: DetailSubScreen::CartridgeActions {
                                flash_write_save: true,
                                ..
                            },
                            ..
                        }
                    );

                    let entry = app.store.entry(&sha1).cloned();
                    let device = app.detected_cartridge_devices.iter().find(|d| {
                        d.cartridge
                            .as_ref()
                            .and_then(|c| c.flash.as_ref())
                            .is_some()
                    });

                    if let (Some(entry), Some(device)) = (entry, device) {
                        let flash = device.cartridge.as_ref().unwrap().flash.clone().unwrap();
                        let port_name = device.port_name.clone();

                        let rom_path = match entry.rom_paths.first() {
                            Some(p) => p.clone(),
                            None => return Task::none(),
                        };
                        let rom_data = match std::fs::read(&rom_path) {
                            Ok(data) => data,
                            Err(e) => {
                                app.screen = Screen::ViewingGame {
                                    sha1,
                                    sub_screen: DetailSubScreen::FlashCartridge {
                                        flash_state: FlashState::Failed(format!(
                                            "Failed to read ROM: {e}"
                                        )),
                                    },
                                };
                                return Task::none();
                            }
                        };

                        let sram_data = if write_save {
                            super::find_by_sha1(&sha1).and_then(|(game_dir, _)| {
                                super::activity::load_current_sram(&game_dir)
                            })
                        } else {
                            None
                        };
                        let cart_header = device.cartridge.clone().unwrap();

                        app.screen = Screen::ViewingGame {
                            sha1,
                            sub_screen: DetailSubScreen::FlashCartridge {
                                flash_state: FlashState::InProgress(cartridge_rw::FlashProgress {
                                    phase: cartridge_rw::FlashPhase::Erasing,
                                    bytes_done: 0,
                                    bytes_total: rom_data.len(),
                                }),
                            },
                        };

                        let (tx, rx) = smol::channel::bounded(32);
                        let progress_task = Task::run(
                            smol::stream::unfold(rx, |rx| async {
                                rx.recv().await.ok().map(|p| (p, rx))
                            }),
                            |progress| app::Message::Cartridge(FlashProgress(progress)),
                        );

                        let port_name_for_sram = port_name.clone();
                        let flash_task = Task::perform(
                            smol::unblock(move || {
                                cartridge_rw::flash_rom(&port_name, &flash, &rom_data, &mut |p| {
                                    let _ = tx.send_blocking(p);
                                })?;
                                if let Some(sram) = sram_data {
                                    cartridge_rw::write_sram(
                                        &port_name_for_sram,
                                        &cart_header,
                                        &sram,
                                    )?;
                                    Ok(Some(sram))
                                } else {
                                    Ok(None)
                                }
                            }),
                            |result| app::Message::Cartridge(FlashComplete(result)),
                        );

                        return Task::batch([flash_task, progress_task]);
                    }
                }
                FlashCancel => {
                    // Go back to the detail page if we have a viewing SHA1
                    if let Screen::ViewingGame { sha1, .. } = &app.screen {
                        let sha1 = sha1.clone();
                        return app.go_to_detail(&sha1);
                    }
                    app.screen = Screen::Library { hovered_game: None };
                }
                FlashToggleSave(enabled) => {
                    if let Screen::ViewingGame {
                        sub_screen:
                            DetailSubScreen::CartridgeActions {
                                flash_write_save, ..
                            },
                        ..
                    } = &mut app.screen
                    {
                        *flash_write_save = enabled;
                    }
                }
                FlashProgress(progress) => {
                    if let Screen::ViewingGame {
                        sub_screen: DetailSubScreen::FlashCartridge { flash_state },
                        ..
                    } = &mut app.screen
                    {
                        *flash_state = FlashState::InProgress(progress);
                    }
                }
                FlashComplete(result) => {
                    match result {
                        Ok(written_sram) => {
                            // Record cartridge sync if save was written
                            if let Some(sram) = &written_sram {
                                if let Some(sha1) = app.viewing_sha1() {
                                    if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                                        super::activity::write_cart_write(&game_dir, sram);
                                    }
                                }
                            }
                            if let Screen::ViewingGame {
                                sub_screen: DetailSubScreen::FlashCartridge { flash_state },
                                ..
                            } = &mut app.screen
                            {
                                *flash_state = FlashState::Complete;
                            }
                            // Force full re-detection to read the new cartridge header
                            app.detected_cartridge_devices.clear();
                            app.cartridge_rw_known_ports.clear();
                        }
                        Err(e) => {
                            if let Screen::ViewingGame {
                                sub_screen: DetailSubScreen::FlashCartridge { flash_state },
                                ..
                            } = &mut app.screen
                            {
                                *flash_state = FlashState::Failed(e);
                            }
                        }
                    }
                }
            }
        }

        // Screenshot gallery
        app::Message::ScreenshotGallery(msg) => {
            use screenshot_gallery::Message as G;
            match msg {
                G::SelectScreenshot(idx) => {
                    if let Screen::ViewingGame {
                        sub_screen: DetailSubScreen::ScreenshotGallery { gallery_state },
                        ..
                    } = &mut app.screen
                    {
                        gallery_state.select(idx);
                    }
                }
                G::SetPalette(pal) => {
                    if let Screen::ViewingGame {
                        sub_screen: DetailSubScreen::ScreenshotGallery { gallery_state },
                        ..
                    } = &mut app.screen
                    {
                        gallery_state.palette = pal;
                    }
                }
                G::SetScale(scale) => {
                    if let Screen::ViewingGame {
                        sub_screen: DetailSubScreen::ScreenshotGallery { gallery_state },
                        ..
                    } = &mut app.screen
                    {
                        gallery_state.scale = scale;
                    }
                }
                G::Export => {
                    let dialog = rfd::AsyncFileDialog::new()
                        .set_file_name("screenshot.png")
                        .add_filter("PNG Image", &["png"]);
                    return Task::perform(dialog.save_file(), |handle| {
                        app::Message::ScreenshotGallery(G::ExportSelected(handle))
                    });
                }
                G::ExportSelected(handle) => {
                    if let Screen::ViewingGame {
                        sub_screen: DetailSubScreen::ScreenshotGallery { gallery_state },
                        ..
                    } = &app.screen
                    {
                        if let Some(handle) = handle {
                            let rgba = gallery_state.selected_rgba();
                            let width = 160 * gallery_state.scale;
                            let height = 144 * gallery_state.scale;
                            let scaled = screenshot_gallery::scale_nearest_neighbour(
                                &rgba,
                                160,
                                144,
                                gallery_state.scale,
                            );
                            if let Some(img) = image::RgbaImage::from_raw(width, height, scaled) {
                                let _ = img.save(handle.path());
                            }
                        }
                    }
                }
                G::Back => {
                    if let Screen::ViewingGame { sha1, .. } = &app.screen {
                        let sha1 = sha1.clone();
                        return app.go_to_detail(&sha1);
                    }
                }
            }
        }

        // Homebrew
        app::Message::HomebrewDownloaded(title, rom_bytes, manifest) => {
            let sha1 = super::hasheous::rom_sha1(&rom_bytes);

            // Check if already in library
            if app.store.entry(&sha1).is_some() {
                return Task::none();
            }

            let Some(game_dir) = super::game_dir_for(&title, &sha1) else {
                return Task::none();
            };
            let _ = std::fs::create_dir_all(&game_dir);

            // Get filename from source
            let filename = match &manifest.source {
                Some(super::catalogue::GameSource::HomebrewHub { filename, .. }) => {
                    filename.clone()
                }
                _ => format!("{}.gb", title.to_lowercase().replace(' ', "-")),
            };
            let filename = std::path::Path::new(&filename)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or(filename);
            let rom_path = game_dir.join(&filename);
            let _ = std::fs::write(&rom_path, &rom_bytes);

            // Create library entry
            let header_title = Cartridge::peek_title(&rom_bytes);
            let mut entry = super::GameEntry::new(sha1.clone(), title, rom_path);
            entry.header_title = if header_title.is_empty() {
                None
            } else {
                Some(header_title)
            };
            entry.platform = Some("Nintendo Game Boy".to_string());
            entry.year = manifest.date.clone();
            entry.description = manifest.description.clone();
            entry.publisher = manifest.developer.clone();
            super::save_entry(&game_dir, &entry);
            // Use cached cover bytes from the browser if available
            let slug = match &manifest.source {
                Some(super::catalogue::GameSource::HomebrewHub { slug, .. }) => Some(slug.clone()),
                _ => None,
            };
            let cached_cover = slug.as_ref().and_then(|s| {
                if let Screen::HomebrewBrowser { state } = &app.screen {
                    state.cover_bytes.get(s).cloned()
                } else {
                    None
                }
            });

            if let Some(bytes) = &cached_cover {
                super::save_cover(&game_dir, bytes);
            }

            app.store.notify_game_added(&sha1, game_dir.clone());
            let detail_task = app.go_to_detail(&sha1);

            // If no cached cover, download in background
            if cached_cover.is_none() {
                if let Some(slug) = slug {
                    let cover_url = format!(
                        "https://raw.githubusercontent.com/gbdev/database/master/entries/{slug}/cover.png"
                    );
                    let client = app.homebrew_client.clone();
                    let gd = game_dir;
                    let sha1_clone = sha1;
                    let cover_task = Task::perform(
                        smol::unblock(move || {
                            if let Ok(bytes) = client.download_image(&cover_url) {
                                super::save_cover(&gd, &bytes);
                            }
                            sha1_clone
                        }),
                        |sha1| {
                            app::Message::EnrichComplete(super::scanner::EnrichResult {
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
        app::Message::OpenHomebrewBrowser => {
            app.screen = Screen::HomebrewBrowser {
                state: homebrew_browser::BrowserState::new(),
            };
            // Load covers for the initial results
            return load_homebrew_covers(app);
        }
        app::Message::HomebrewBrowser(msg) => {
            use homebrew_browser::Message as H;
            match msg {
                H::SearchTextChanged(text) => {
                    if let Screen::HomebrewBrowser { state } = &mut app.screen {
                        state.search_text = text;
                        state.visible_count = homebrew_browser::PAGE_SIZE;
                        state.error = None;
                    }
                    return load_homebrew_covers(app);
                }
                H::ShowMore => {
                    if let Screen::HomebrewBrowser { state } = &mut app.screen {
                        state.visible_count += homebrew_browser::PAGE_SIZE;
                    }
                    return load_homebrew_covers(app);
                }
                H::DownloadFailed(error) => {
                    if let Screen::HomebrewBrowser { state } = &mut app.screen {
                        state.error = Some(error);
                    }
                }
                H::DismissError => {
                    if let Screen::HomebrewBrowser { state } = &mut app.screen {
                        state.error = None;
                    }
                }
                H::CoverLoaded(slug, bytes) => {
                    if let Screen::HomebrewBrowser { state } = &mut app.screen {
                        state.covers.insert(
                            slug.clone(),
                            iced::widget::image::Handle::from_bytes(bytes.clone()),
                        );
                        state.cover_bytes.insert(slug, bytes);
                    }
                }
                H::SelectEntry(slug) => {
                    if let Screen::HomebrewBrowser { state } = &mut app.screen {
                        state.selected_slug = Some(slug.clone());

                        // Load cover image if not cached
                        if !state.covers.contains_key(&slug) {
                            if let Some(entry) = app.catalogue.lookup_slug(&slug) {
                                if let Some(url) = entry.download_cover_url() {
                                    let client = app.homebrew_client.clone();
                                    let s = slug;
                                    return Task::perform(
                                        smol::unblock(move || {
                                            client.download_image(&url).ok().map(|bytes| (s, bytes))
                                        }),
                                        |result| match result {
                                            Some((slug, bytes)) => app::Message::HomebrewBrowser(
                                                H::CoverLoaded(slug, bytes),
                                            ),
                                            None => app::Message::None,
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                H::Download(slug) => {
                    if let Some(entry) = app.catalogue.lookup_slug(&slug) {
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
                                        app::Message::HomebrewDownloaded(title, rom_bytes, manifest)
                                    }
                                    Err(e) => app::Message::HomebrewBrowser(
                                        homebrew_browser::Message::DownloadFailed(format!(
                                            "Download failed: {e}"
                                        )),
                                    ),
                                },
                            );
                        }
                    }
                }
                H::Back => {
                    if let Screen::HomebrewBrowser { state } = &mut app.screen {
                        if state.selected_slug.is_some() {
                            // Back from detail to results
                            state.selected_slug = None;
                        } else {
                            // Back from results to library
                            app.screen = Screen::Library { hovered_game: None };
                        }
                    }
                }
            }
        }
        app::Message::ActivityLoaded(raw) => {
            // Only apply if we're still viewing the same game
            if app.viewing_sha1() == Some(&raw.sha1) {
                app.store.set_raw_activity_detail(raw);
            }
        }
        app::Message::ScanComplete(changed) => {
            if changed {
                app.store.rebuild_index();
            }
            if app.settings.internet_enabled && app.settings.hasheous_enabled {
                return Task::perform(smol::unblock(|| super::scanner::enrich_next()), |result| {
                    app::Message::EnrichComplete(result)
                });
            }
        }
        app::Message::EnrichComplete(result) => {
            if let Some(sha1) = &result.sha1 {
                if result.data_changed {
                    app.store.notify_metadata_changed(sha1);
                }
            }

            // Sync recent game titles with enriched library entries
            for summary in app.store.all_summaries() {
                app.recent_games
                    .update_title(&summary.entry.sha1, &summary.entry.display_title());
            }
            app.recent_games.save();

            // Also update the current game if loaded
            if let Some(current) = &mut app.current_game {
                if let Some((_dir, entry)) = super::find_by_sha1(&current.entry.sha1) {
                    current.entry = entry;
                    current.cover = super::load_cover(&current.game_dir)
                        .map(|bytes| iced::widget::image::Handle::from_bytes(bytes));
                }
            }

            // Chain: enrich next game if there are more
            if result.has_more && app.settings.internet_enabled && app.settings.hasheous_enabled {
                return Task::perform(smol::unblock(|| super::scanner::enrich_next()), |result| {
                    app::Message::EnrichComplete(result)
                });
            }
        }
        app::Message::OpenUrl(url) => {
            let _ = open::that(url);
        }
        app::Message::CartridgeRwDumpProgress(progress) => {
            app.cartridge_dump_progress = Some(progress);
        }
        app::Message::CartridgeRwDumpComplete(result) => {
            app.cartridge_dump_progress = None;
            match result {
                Ok((rom, sram)) => {
                    let header_title = Cartridge::peek_title(&rom);
                    let title = if header_title.is_empty() {
                        "Unknown".to_string()
                    } else {
                        header_title.clone()
                    };
                    let sha1 = super::hasheous::rom_sha1(&rom);

                    // Save ROM file to library
                    let game_dir = match super::game_dir_for(&title, &sha1) {
                        Some(dir) => dir,
                        None => return Task::none(),
                    };
                    let _ = std::fs::create_dir_all(&game_dir);
                    let rom_path = game_dir.join(format!("{title}.gb"));
                    if std::fs::write(&rom_path, &rom).is_err() {
                        return Task::none();
                    }

                    // Create library entry
                    let mut entry = super::GameEntry::new(sha1, title, rom_path);
                    entry.header_title = if header_title.is_empty() {
                        None
                    } else {
                        Some(header_title)
                    };
                    super::save_entry(&game_dir, &entry);

                    // Import SRAM if we read it from the cartridge
                    if let Some(sram) = &sram {
                        super::activity::write_cartridge_import(&game_dir, sram);
                    }

                    // Refresh the library view
                    app.store.rebuild_index();

                    // Trigger enrichment for cover art etc.
                    if app.settings.internet_enabled && app.settings.hasheous_enabled {
                        return Task::perform(
                            smol::unblock(|| super::scanner::enrich_next()),
                            |result| app::Message::EnrichComplete(result),
                        );
                    }
                }
                Err(_) => {}
            }
        }

        _ => {}
    }

    Task::none()
}

/// Load cover images for visible homebrew entries (first batch only).
fn load_homebrew_covers(app: &app::App) -> Task<app::Message> {
    use homebrew_browser::Message as H;
    let Screen::HomebrewBrowser { state } = &app.screen else {
        return Task::none();
    };

    let results = if state.search_text.is_empty() {
        app.catalogue.homebrew()
    } else {
        app.catalogue.search_homebrew(&state.search_text)
    };

    let visible = state.visible_count.min(results.len());

    let tasks: Vec<Task<app::Message>> = results[..visible]
        .iter()
        .filter(|e| !state.covers.contains_key(&e.slug))
        .filter_map(|e| {
            let url = e.download_cover_url()?;
            let slug = e.slug.clone();
            let client = app.homebrew_client.clone();
            Some(Task::perform(
                smol::unblock(move || client.download_image(&url).ok().map(|bytes| (slug, bytes))),
                |result| match result {
                    Some((slug, bytes)) => {
                        app::Message::HomebrewBrowser(H::CoverLoaded(slug, bytes))
                    }
                    None => app::Message::None,
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
