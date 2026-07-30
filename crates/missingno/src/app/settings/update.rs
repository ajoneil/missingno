use iced::Task;

use crate::app::{self, Game, LoadedGame, library};

pub(in crate::app) fn handle(
    app: &mut app::App,
    message: super::view::Message,
) -> Task<app::Message> {
    match message {
        super::view::Message::SelectSection(section) => {
            if let app::Screen::Settings {
                section: ref mut s, ..
            } = app.screen
            {
                *s = section;
            }
        }
        super::view::Message::Back => {
            if let app::Screen::Settings {
                previous_screen,
                was_running,
                ..
            } = std::mem::replace(&mut app.screen, app::Screen::Library { hovered_game: None })
            {
                app.screen = *previous_screen;
                if was_running {
                    app.run();
                }
            }
        }
        super::view::Message::SetInternetEnabled(enabled) => {
            app.settings.internet_enabled = enabled;
            app.settings.save();
        }
        super::view::Message::SetHasheousEnabled(enabled) => {
            app.settings.hasheous_enabled = enabled;
            app.settings.save();
        }
        super::view::Message::SetHomebrewHubEnabled(enabled) => {
            app.settings.homebrew_hub_enabled = enabled;
            app.settings.save();
        }
        super::view::Message::SetAllowExternalClients(enabled) => {
            app.settings.allow_external_clients = enabled;
            app.settings.save();
            // Takes effect on the running game immediately: turning it off
            // unpublishes the socket rather than waiting for the next load.
            app.publish_session();
        }
        super::view::Message::SetAllowUiAutomation(enabled) => {
            app.settings.allow_ui_automation = enabled;
            app.settings.save();
            app.reconcile_automation();
        }
        super::view::Message::PickRomDirectory => {
            let dialog = rfd::AsyncFileDialog::new();
            return Task::perform(dialog.pick_folder(), |folder| match folder {
                Some(handle) => {
                    let path = handle.path().to_path_buf();
                    super::view::Message::AddRomDirectory(path).into()
                }
                None => app::Message::None,
            });
        }
        super::view::Message::AddRomDirectory(path) => {
            if !app.settings.rom_directories.contains(&path) {
                app.settings.rom_directories.push(path.clone());
                app.settings.save();
                let dirs = vec![path];
                let cat = app.catalogue.clone();
                return Task::perform(
                    smol::unblock(move || library::scanner::scan_directories(&dirs, &cat)),
                    |entries| app::Message::ScanComplete(!entries.is_empty()),
                );
            }
        }
        super::view::Message::RemoveRomDirectory(index) => {
            if index < app.settings.rom_directories.len() {
                app.settings.rom_directories.remove(index);
                app.settings.save();
            }
        }
        super::view::Message::SelectPalette(palette) => {
            app.settings.palette = palette;
            app.settings.save();
            match &mut app.game {
                Game::Loaded(LoadedGame::Emulator(emulator)) => {
                    emulator.set_palette(palette);
                }
                Game::Loaded(LoadedGame::Debugger(debugger)) => {
                    debugger.set_palette(palette);
                }
                _ => {}
            }
        }
        super::view::Message::SetUseSgbColors(enabled) => {
            app.settings.use_sgb_colors = enabled;
            app.settings.save();
            if let Game::Loaded(LoadedGame::Emulator(emu)) = &mut app.game {
                emu.set_use_sgb_colors(enabled);
            }
        }
        super::view::Message::SetPersistence(enabled) => {
            app.settings.persistence = enabled;
            app.settings.save();
            // Persistence is the main display's control; the debugger screen
            // pane has its own per-pane device/raw toggle instead.
            if let Game::Loaded(LoadedGame::Emulator(emu)) = &mut app.game {
                emu.set_persistence(enabled);
            }
        }
        super::view::Message::SetPixelGrid(enabled) => {
            app.settings.pixel_grid = enabled;
            app.settings.save();
            // Like persistence, this is the main display's control; the
            // debugger pane bundles the overlay into its device/raw toggle.
            if let Game::Loaded(LoadedGame::Emulator(emu)) = &mut app.game {
                emu.set_pixel_grid(enabled);
            }
        }
        super::view::Message::SetScanlines(enabled) => {
            app.settings.scanlines = enabled;
            app.settings.save();
            if let Game::Loaded(LoadedGame::Emulator(emu)) = &mut app.game {
                emu.set_scanlines(enabled);
            }
        }
        super::view::Message::SetCartridgeRwEnabled(enabled) => {
            app.settings.cartridge_rw_enabled = enabled;
            app.settings.save();
            if !enabled {
                app.cartridge_rw.detected_devices.clear();
                app.cartridge_rw.known_ports.clear();
            }
        }
        super::view::Message::SelectControlsPage(page) => {
            if let app::Screen::Settings {
                ref mut controls,
                ref mut listening_for,
                ..
            } = app.screen
            {
                controls.page = page;
                *listening_for = None;
            }
        }
        super::view::Message::SelectControllerTab(platform, peripheral) => {
            if let app::Screen::Settings {
                ref mut controls,
                ref mut listening_for,
                ..
            } = app.screen
            {
                controls.controller_tabs.insert(platform, peripheral);
                *listening_for = None;
            }
        }
        super::view::Message::SetPointerKnob(platform, drives) => {
            app.settings.controls.set_pointer_knob(platform, drives);
            app.settings.save();
            app.push_routing();
        }
        super::view::Message::StartListening(target) => {
            if let app::Screen::Settings {
                ref mut listening_for,
                ..
            } = app.screen
            {
                *listening_for = Some(target);
            }
        }
        super::view::Message::CaptureBinding(key_str) => {
            if let Some(listening) = take_listening(app) {
                match listening.target {
                    super::view::BindingTarget::Emulator(action) => app
                        .settings
                        .controls
                        .set_emulator(listening.surface, action, key_str),
                    super::view::BindingTarget::System(platform, slot) => app
                        .settings
                        .controls
                        .set_system(platform, listening.surface, slot, key_str),
                }
                app.settings.save();
                app.push_routing();
            }
        }
        super::view::Message::ClearBinding => {
            if let Some(listening) = take_listening(app) {
                match listening.target {
                    super::view::BindingTarget::Emulator(action) => app
                        .settings
                        .controls
                        .clear_emulator(listening.surface, action),
                    super::view::BindingTarget::System(platform, slot) => app
                        .settings
                        .controls
                        .clear_system(platform, listening.surface, slot),
                }
                app.settings.save();
                app.push_routing();
            }
        }
        super::view::Message::CancelCapture => {
            if let app::Screen::Settings {
                ref mut listening_for,
                ..
            } = app.screen
            {
                *listening_for = None;
            }
        }
        super::view::Message::ResetBindings(page) => {
            match page {
                super::view::ControlsPage::Emulator => app.settings.controls.reset_emulator(),
                super::view::ControlsPage::System(platform) => {
                    app.settings.controls.reset_system(platform)
                }
            }
            app.settings.save();
            if let app::Screen::Settings {
                ref mut listening_for,
                ..
            } = app.screen
            {
                *listening_for = None;
            }
            app.push_routing();
        }
    }

    Task::none()
}

/// Take the binding the settings screen is waiting on, ending the capture.
fn take_listening(app: &mut app::App) -> Option<super::view::ListeningFor> {
    match app.screen {
        app::Screen::Settings {
            ref mut listening_for,
            ..
        } => listening_for.take(),
        _ => None,
    }
}
