use std::time::Instant;

use iced::Task;
use replace_with::replace_with_or_abort;

use missingno_gb::joypad::Button;

use super::{App, Game, LoadedGame, Message, PendingAction, library};

impl App {
    pub(super) fn handle_emulation_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Run => self.run(),
            Message::Pause => self.pause(),
            Message::TogglePause => {
                if self.running() {
                    self.pause();
                } else {
                    self.run();
                }
            }
            Message::Reset => {
                self.pending_action = Some(PendingAction::ResetEmulator);
            }
            Message::SaveBattery => {
                self.save();
            }
            Message::TakeScreenshot => {
                // Grab the current framebuffer and SGB state from whichever game mode is active
                let capture_data = match &self.game {
                    Game::Loaded(LoadedGame::Emulator(emu)) => {
                        let gb = emu.game_boy();
                        let sgb_data = gb
                            .sgb()
                            .map(|sgb| sgb.render_data(gb.ppu().control().video_enabled()));
                        Some((gb.screen().clone(), sgb_data))
                    }
                    Game::Loaded(LoadedGame::Debugger(dbg)) => {
                        let gb = dbg.game_boy();
                        let sgb_data = gb
                            .sgb()
                            .map(|sgb| sgb.render_data(gb.ppu().control().video_enabled()));
                        Some((gb.screen().clone(), sgb_data))
                    }
                    _ => None,
                };
                if let Some((screen, sgb_render_data)) = capture_data {
                    let capture = library::activity::FrameCapture::capture(
                        screen.front(),
                        sgb_render_data.as_ref(),
                        self.settings.use_sgb_colors,
                        &self.settings.palette.to_string(),
                    );
                    if let Some(current) = &mut self.current_game {
                        if let Some(session) = &mut current.session {
                            session.events.push(library::activity::SessionEvent {
                                at: jiff::Timestamp::now(),
                                kind: library::activity::EventKind::Screenshot { frame: capture },
                            });
                            library::activity::write_session(&current.game_dir, session);
                            self.store.update_live_screenshots(session);
                        }
                    }
                    self.screenshot_toast = Some(Instant::now());
                }
            }
            Message::DismissScreenshotToast => {
                self.screenshot_toast = None;
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
                                let mut emu =
                                    debugger.disable_debugger(self.settings.use_sgb_colors);
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
            _ => {}
        }

        Task::none()
    }

    pub(super) fn running(&self) -> bool {
        match &self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.running(),
                LoadedGame::Emulator(emulator) => emulator.running(),
            },
            _ => false,
        }
    }

    pub(super) fn run(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.run(),
                LoadedGame::Emulator(emulator) => emulator.run(),
            },
            _ => {}
        }
    }

    pub(super) fn pause(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.pause(),
                LoadedGame::Emulator(emulator) => emulator.pause(),
            },
            _ => {}
        }
    }

    pub(super) fn reset(&mut self) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.reset(),
                LoadedGame::Emulator(emulator) => emulator.reset(),
            },
            _ => {}
        }
    }

    pub(super) fn press_button(&mut self, button: Button) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.press_button(button),
                LoadedGame::Emulator(emulator) => emulator.press_button(button),
            },
            _ => {}
        }
    }

    pub(super) fn release_button(&mut self, button: Button) {
        match &mut self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.release_button(button),
                LoadedGame::Emulator(emulator) => emulator.release_button(button),
            },
            _ => {}
        }
    }

    /// Flush any debounced SRAM save from the emulator.
    pub(super) fn flush_pending_save(&mut self) {
        let flushed = match &mut self.game {
            Game::Loaded(LoadedGame::Emulator(emu)) => emu.flush_pending_save(),
            _ => false,
        };
        if flushed {
            self.save();
        }
    }

    pub(super) fn save(&mut self) {
        let (ram, cartridge_title) = match &self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                if !debugger.game_boy().cartridge().has_battery() {
                    return;
                }
                (
                    debugger.game_boy().cartridge().ram(),
                    debugger.game_boy().cartridge().title().to_string(),
                )
            }
            Game::Loaded(LoadedGame::Emulator(emulator)) => {
                if !emulator.game_boy().cartridge().has_battery() {
                    return;
                }
                (
                    emulator.game_boy().cartridge().ram(),
                    emulator.game_boy().cartridge().title().to_string(),
                )
            }
            _ => return,
        };
        let Some(ram) = ram else { return };
        let Some(current) = &mut self.current_game else {
            return;
        };

        if let Some(session) = &mut current.session {
            // Check if SRAM has meaningfully changed, ignoring scratch regions
            let previous = session.last_sram().or(current.initial_sram.as_deref());
            let changed = match previous {
                Some(prev) => library::game_db::sram_changed(&cartridge_title, &ram, prev),
                None => true, // No previous data at all — always record
            };

            if changed {
                session.events.push(library::activity::SessionEvent {
                    at: jiff::Timestamp::now(),
                    kind: library::activity::EventKind::Save { sram: ram.to_vec() },
                });
                // Write incrementally for crash safety
                library::activity::write_session(&current.game_dir, session);
            }
        }
    }

    pub(super) fn drain_audio(&mut self) {
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
}
