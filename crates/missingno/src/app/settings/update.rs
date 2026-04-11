use iced::Task;

use crate::app::{self, controls, library, Game, LoadedGame};

pub(in crate::app) fn handle(
    app: &mut app::App,
    message: super::view::Message,
) -> Task<app::Message> {
    match message {
        super::view::Message::SelectSection(section) => {
            if let app::Screen::Settings { section: ref mut s, .. } = app.screen {
                *s = section;
            }
        }
        super::view::Message::Back => {
            if let app::Screen::Settings { previous_screen, was_running, .. } = std::mem::replace(&mut app.screen, app::Screen::Library { hovered_game: None }) {
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
            if let app::Screen::Settings { ref mut listening_for, .. } = app.screen {
                *listening_for = Some(target);
            }
        }
        super::view::Message::CaptureBinding(key_str) => {
            let target = if let app::Screen::Settings { ref mut listening_for, .. } = app.screen {
                listening_for.take()
            } else {
                None
            };
            if let Some(target) = target {
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
            let target = if let app::Screen::Settings { ref mut listening_for, .. } = app.screen {
                listening_for.take()
            } else {
                None
            };
            if let Some(target) = target {
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
            if let app::Screen::Settings { ref mut listening_for, .. } = app.screen {
                *listening_for = None;
            }
        }
        super::view::Message::ResetBindings => {
            app.settings.keyboard_bindings = super::Bindings::default_keyboard();
            app.settings.gamepad_bindings = super::Bindings::default_gamepad();
            app.settings.save();
            if let app::Screen::Settings { ref mut listening_for, .. } = app.screen {
                *listening_for = None;
            }
            controls::update_bindings(
                &app.settings.keyboard_bindings,
                &app.settings.gamepad_bindings,
            );
        }
    }

    Task::none()
}
