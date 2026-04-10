use iced::Task;

use missingno_gb::cartridge::Cartridge;

use crate::app::{self, load, Game, Screen};
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
            app.hovered_library_game = Some(sha1);
        }
        super::view::Message::UnhoverGame => {
            app.hovered_library_game = None;
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
                    smol::stream::unfold(rx, |rx| async {
                        rx.recv().await.ok().map(|p| (p, rx))
                    }),
                    app::Message::CartridgeRwDumpProgress,
                );

                let dump_task = Task::perform(
                    smol::unblock(move || {
                        cartridge_rw::dump_rom(&port_name, &header, &mut |p| {
                            let _ = tx.send_blocking(p);
                        })
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

pub(in crate::app) fn handle(
    app: &mut app::App,
    message: app::Message,
) -> Task<app::Message> {
    match message {
        // Game management (detail page actions)
        app::Message::OpenGameFolder => {
            if let Some(sha1) = &app.viewing_sha1 {
                if let Some(dir) = super::find_by_sha1(sha1).map(|(d, _)| d) {
                    let _ = open::that(&dir);
                }
            }
        }
        app::Message::RefreshMetadata => {
            if let Some(sha1) = app.viewing_sha1.clone() {
                return Task::perform(
                    smol::unblock(move || super::hasheous::lookup(&sha1).ok().flatten()),
                    move |info| {
                        if let Some(info) = info {
                            app::Message::GameMetadataRefreshed(info)
                        } else {
                            app::Message::None
                        }
                    },
                );
            }
        }
        app::Message::GameMetadataRefreshed(info) => {
            if let Some(sha1) = &app.viewing_sha1 {
                if let Some((game_dir, mut entry)) = super::find_by_sha1(sha1) {
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
                    app.store.notify_metadata_changed(sha1);
                }
            }
        }
        app::Message::ImportSave => {
            let dialog = rfd::AsyncFileDialog::new().add_filter("Game Boy Save", &["sav"]);
            return Task::perform(dialog.pick_file(), |handle| {
                app::Message::ImportSaveSelected(handle)
            });
        }
        app::Message::ImportSaveSelected(handle) => {
            if let (Some(handle), Some(sha1)) = (handle, &app.viewing_sha1) {
                if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                    if let Ok(data) = std::fs::read(handle.path()) {
                        super::activity::write_import(&game_dir, &data);
                    }
                }
            }
        }
        app::Message::PlayWithSave(save_id) => {
            // Launch the game with a specific save
            if let Some(sha1) = app.viewing_sha1.clone() {
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
        app::Message::ExportSave(save_id) => {
            let dialog = rfd::AsyncFileDialog::new()
                .set_file_name("save.sav")
                .add_filter("Game Boy Save", &["sav"]);
            return Task::perform(dialog.save_file(), move |handle| {
                app::Message::ExportSaveSelected(save_id.clone(), handle)
            });
        }
        app::Message::ExportSaveSelected(save_id, handle) => {
            if let (Some(handle), Some(sha1)) = (handle, &app.viewing_sha1) {
                if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                    if let Some(data) = super::activity::load_sram_from(&game_dir, &save_id) {
                        let _ = std::fs::write(handle.path(), data);
                    }
                }
            }
        }
        app::Message::OpenScreenshotGallery(session_filename, screenshot_idx) => {
            if let Some(sha1) = &app.viewing_sha1 {
                if let Some((game_dir, _)) = super::find_by_sha1(sha1) {
                    if let Some(mut state) = screenshot_gallery::GalleryState::load(
                        &game_dir,
                        &session_filename,
                    ) {
                        state.select(screenshot_idx);
                        app.gallery_state = Some(state);
                        app.screen = Screen::ScreenshotGallery;
                    }
                }
            }
        }
        app::Message::ScreenshotGallery(msg) => {
            use screenshot_gallery::Message as G;
            match msg {
                G::SelectScreenshot(idx) => {
                    if let Some(state) = &mut app.gallery_state {
                        state.select(idx);
                    }
                }
                G::SetPalette(pal) => {
                    if let Some(state) = &mut app.gallery_state {
                        state.palette = pal;
                    }
                }
                G::SetScale(scale) => {
                    if let Some(state) = &mut app.gallery_state {
                        state.scale = scale;
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
                    if let (Some(handle), Some(state)) = (handle, &app.gallery_state) {
                        let rgba = state.selected_rgba();
                        let width = 160 * state.scale;
                        let height = 144 * state.scale;
                        let scaled = screenshot_gallery::scale_nearest_neighbour(
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
                    app.gallery_state = None;
                    if let Some(sha1) = app.viewing_sha1.clone() {
                        return app.go_to_detail(&sha1);
                    }
                }
            }
        }
        app::Message::HoverLogEntry(idx) => {
            app.hovered_log_entry = Some(idx);
        }
        app::Message::UnhoverLogEntry => {
            app.hovered_log_entry = None;
        }
        app::Message::HoverHeader => {
            app.header_hovered = true;
        }
        app::Message::UnhoverHeader => {
            app.header_hovered = false;
        }
        app::Message::RemoveGame => {
            app.pending_action = Some(app::PendingAction::RemoveGameFromLibrary);
        }
        app::Message::HomebrewDownloaded(title, rom_bytes, manifest) => {
            let sha1 = super::hasheous::rom_sha1(&rom_bytes);

            // Check if already in library
            if app.store.entry(&sha1).is_some() {
                eprintln!("[homebrew] {title} already in library");
                return Task::none();
            }

            let Some(game_dir) = super::game_dir_for(&title, &sha1) else {
                return Task::none();
            };
            if let Err(e) = std::fs::create_dir_all(&game_dir) {
                eprintln!("[homebrew] Failed to create game dir: {e}");
            }

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
            eprintln!(
                "[homebrew] Saving {} bytes to {}",
                rom_bytes.len(),
                rom_path.display()
            );
            if let Err(e) = std::fs::write(&rom_path, &rom_bytes) {
                eprintln!("[homebrew] Failed to write ROM: {e}");
            }

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
                Some(super::catalogue::GameSource::HomebrewHub { slug, .. }) => {
                    Some(slug.clone())
                }
                _ => None,
            };
            let cached_cover = slug.as_ref().and_then(|s| {
                app.homebrew_browser
                    .as_ref()
                    .and_then(|b| b.cover_bytes.get(s).cloned())
            });

            if let Some(bytes) = &cached_cover {
                super::save_cover(&game_dir, bytes);
            }

            app.store.notify_game_added(&sha1, game_dir.clone());
            app.homebrew_browser = None;
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
            app.homebrew_browser = Some(homebrew_browser::BrowserState::new());
            app.screen = Screen::HomebrewBrowser;
            // Load covers for the initial results
            return load_homebrew_covers(app);
        }
        app::Message::HomebrewBrowser(msg) => {
            use homebrew_browser::Message as H;
            match msg {
                H::SearchTextChanged(text) => {
                    if let Some(state) = &mut app.homebrew_browser {
                        state.search_text = text;
                        state.visible_count = homebrew_browser::PAGE_SIZE;
                        state.error = None;
                    }
                }
                H::ShowMore => {
                    if let Some(state) = &mut app.homebrew_browser {
                        state.visible_count += homebrew_browser::PAGE_SIZE;
                        return load_homebrew_covers(app);
                    }
                }
                H::DownloadFailed(error) => {
                    if let Some(state) = &mut app.homebrew_browser {
                        state.error = Some(error);
                    }
                }
                H::DismissError => {
                    if let Some(state) = &mut app.homebrew_browser {
                        state.error = None;
                    }
                }
                H::CoverLoaded(slug, bytes) => {
                    if let Some(state) = &mut app.homebrew_browser {
                        state.covers.insert(
                            slug.clone(),
                            iced::widget::image::Handle::from_bytes(bytes.clone()),
                        );
                        state.cover_bytes.insert(slug, bytes);
                    }
                }
                H::SelectEntry(slug) => {
                    if let Some(state) = &mut app.homebrew_browser {
                        state.selected_slug = Some(slug.clone());

                        // Load cover image if not cached
                        if !state.covers.contains_key(&slug) {
                            if let Some(entry) = app.catalogue.lookup_slug(&slug) {
                                if let Some(url) = entry.download_cover_url() {
                                    let client = app.homebrew_client.clone();
                                    let s = slug;
                                    return Task::perform(
                                        smol::unblock(move || {
                                            client
                                                .download_image(&url)
                                                .ok()
                                                .map(|bytes| (s, bytes))
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
                                    Err(e) => {
                                        eprintln!("[homebrew] Download failed: {e}");
                                        app::Message::HomebrewBrowser(
                                            homebrew_browser::Message::DownloadFailed(
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
                    if let Some(state) = &mut app.homebrew_browser {
                        if state.selected_slug.is_some() {
                            // Back from detail to results
                            state.selected_slug = None;
                        } else {
                            // Back from results to library
                            app.homebrew_browser = None;
                            app.screen = Screen::Library;
                        }
                    }
                }
            }
        }
        app::Message::ActivityLoaded(raw) => {
            // Only apply if we're still viewing the same game
            if app.viewing_sha1.as_deref() == Some(&raw.sha1) {
                app.store.set_raw_activity_detail(raw);
            }
        }
        app::Message::ScanComplete(changed) => {
            if changed {
                app.store.rebuild_index();
            }
            if app.settings.internet_enabled && app.settings.hasheous_enabled {
                return Task::perform(
                    smol::unblock(|| super::scanner::enrich_next()),
                    |result| app::Message::EnrichComplete(result),
                );
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
            if result.has_more
                && app.settings.internet_enabled
                && app.settings.hasheous_enabled
            {
                return Task::perform(
                    smol::unblock(|| super::scanner::enrich_next()),
                    |result| app::Message::EnrichComplete(result),
                );
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
                Ok(rom) => {
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
                    if let Err(e) = std::fs::write(&rom_path, &rom) {
                        eprintln!("[cartridge_rw] failed to save ROM: {e}");
                        return Task::none();
                    }

                    // Create library entry
                    let mut entry =
                        super::GameEntry::new(sha1, title, rom_path);
                    entry.header_title = if header_title.is_empty() {
                        None
                    } else {
                        Some(header_title)
                    };
                    super::save_entry(&game_dir, &entry);

                    // Refresh the library view
                    app.store.rebuild_index();

                    // Trigger enrichment for cover art etc.
                    if app.settings.internet_enabled
                        && app.settings.hasheous_enabled
                    {
                        return Task::perform(
                            smol::unblock(|| super::scanner::enrich_next()),
                            |result| app::Message::EnrichComplete(result),
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[cartridge_rw] dump failed: {e}");
                }
            }
        }
        _ => {}
    }

    Task::none()
}

/// Load cover images for visible homebrew entries (first batch only).
fn load_homebrew_covers(app: &app::App) -> Task<app::Message> {
    use homebrew_browser::Message as H;
    let Some(state) = &app.homebrew_browser else {
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
                smol::unblock(move || {
                    client.download_image(&url).ok().map(|bytes| (slug, bytes))
                }),
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
