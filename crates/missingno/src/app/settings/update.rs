use iced::Task;

use crate::app::{self, controls, library, Game, LoadedGame};

pub(in crate::app) fn handle(
    app: &mut app::App,
    message: super::view::Message,
) -> Task<app::Message> {
    match message {
        super::view::Message::SelectSection(section) => {
            app.settings_section = section;
        }
        super::view::Message::Back => {
            app.screen = app.previous_screen.take().unwrap_or(app::Screen::Library);
            app.listening_for = None;
            if app.was_running_before_settings {
                app.run();
                app.was_running_before_settings = false;
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
        super::view::Message::SetCartridgeRwEnabled(enabled) => {
            app.settings.cartridge_rw_enabled = enabled;
            app.settings.save();
            if !enabled {
                app.detected_cartridge_devices.clear();
                app.cartridge_rw_known_ports.clear();
            }
        }
        super::view::Message::StartListening(target) => {
            app.listening_for = Some(target);
        }
        super::view::Message::CaptureBinding(key_str) => {
            if let Some(target) = app.listening_for.take() {
                match target {
                    super::view::ListeningFor::Keyboard(action) => {
                        app.settings.keyboard_bindings.set(action, key_str);
                    }
                    super::view::ListeningFor::Gamepad(action) => {
                        app.settings.gamepad_bindings.set(action, key_str);
                    }
                }
                app.settings.save();
                controls::update_bindings(
                    &app.settings.keyboard_bindings,
                    &app.settings.gamepad_bindings,
                );
            }
        }
        super::view::Message::ClearBinding => {
            if let Some(target) = app.listening_for.take() {
                match target {
                    super::view::ListeningFor::Keyboard(action) => {
                        app.settings.keyboard_bindings.0.remove(&action);
                    }
                    super::view::ListeningFor::Gamepad(action) => {
                        app.settings.gamepad_bindings.0.remove(&action);
                    }
                }
                app.settings.save();
                controls::update_bindings(
                    &app.settings.keyboard_bindings,
                    &app.settings.gamepad_bindings,
                );
            }
        }
        super::view::Message::CancelCapture => {
            app.listening_for = None;
        }
        super::view::Message::ResetBindings => {
            app.settings.keyboard_bindings = super::Bindings::default_keyboard();
            app.settings.gamepad_bindings = super::Bindings::default_gamepad();
            app.settings.save();
            app.listening_for = None;
            controls::update_bindings(
                &app.settings.keyboard_bindings,
                &app.settings.gamepad_bindings,
            );
        }
    }

    Task::none()
}
