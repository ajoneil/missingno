use std::time::Instant;

use iced::Task;
use replace_with::replace_with_or_abort;

use super::emu_thread::{EmuCommand, EmuEvent, Payload};
use super::{App, Game, LoadedGame, Message, PendingAction, library};
use crate::app::library::activity::FrameCapture;
use crate::app::system::{ControlId, ControlInput};

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
            Message::TakeScreenshot => {
                let options = self.settings.capture_options();
                // While the game runs, the console is on the emu thread; ask
                // it to capture and record on the resulting `Screenshot` event.
                let capture = match &self.game {
                    Game::Loaded(LoadedGame::Emulator(emu)) if emu.running() => {
                        if let Some(handle) = &self.emu {
                            handle.send(EmuCommand::RequestScreenshot { options });
                        }
                        None
                    }
                    Game::Loaded(LoadedGame::Emulator(emu)) => emu.console().map(|console| {
                        FrameCapture::from_frame(&console.screen_display(), &options)
                    }),
                    Game::Loaded(LoadedGame::Debugger(dbg)) if dbg.is_detached() => {
                        if let Some(handle) = &self.emu {
                            handle.send(EmuCommand::RequestScreenshot { options });
                        }
                        None
                    }
                    Game::Loaded(LoadedGame::Debugger(dbg)) => dbg.capture_screenshot(&options),
                    _ => None,
                };
                if let Some(capture) = capture {
                    self.record_screenshot(capture);
                }
            }
            Message::ExportCapture(index) => {
                let default_name = self
                    .current_game
                    .as_ref()
                    .and_then(|g| g.session.as_ref())
                    .and_then(|s| s.events.get(index))
                    .map(|e| match e.kind {
                        library::activity::EventKind::Print { .. } => "print.png",
                        _ => "screenshot.png",
                    })
                    .unwrap_or("capture.png");
                let dialog = rfd::AsyncFileDialog::new()
                    .set_file_name(default_name)
                    .add_filter("PNG Image", &["png"]);
                return Task::perform(dialog.save_file(), move |handle| {
                    Message::ExportCaptureSaved(index, handle)
                });
            }
            Message::ExportCaptureSaved(index, Some(handle)) => {
                if let Some(session) = self.current_game.as_ref().and_then(|g| g.session.as_ref())
                    && let Some(event) = session.events.get(index)
                {
                    let image = match &event.kind {
                        library::activity::EventKind::Screenshot { frame } => {
                            let (width, height) = frame.dimensions();
                            let rgba = frame
                                .rgba
                                .as_ref()
                                .map(|r| r.data.clone())
                                .unwrap_or_else(|| frame.to_rgba());
                            image::RgbaImage::from_raw(width, height, rgba)
                        }
                        library::activity::EventKind::Print { print } => {
                            let rgba: Vec<u8> =
                                print.pixels.iter().flat_map(|&g| [g, g, g, 0xff]).collect();
                            image::RgbaImage::from_raw(print.width, print.height, rgba)
                        }
                        _ => None,
                    };
                    if let Some(image) = image {
                        let _ = image.save(handle.path());
                    }
                }
            }
            Message::ExportCaptureSaved(_, None) => {}
            Message::DismissScreenshotToast => {
                self.screenshot_toast = None;
            }
            Message::SetControl(control, pressed) => {
                self.set_control(ControlId(control), ControlInput::Digital(pressed))
            }
            Message::SetAxis(control, value) => {
                self.set_control(ControlId(control), ControlInput::Axis(value))
            }
            Message::ToggleDebugger(debugger_enabled) => {
                self.debugger_enabled = debugger_enabled;
                // Conversion needs the console on the UI thread.
                self.pause();

                if let Game::Loaded(game) = &mut self.game {
                    let palette = self.settings.palette;
                    let rom_path = self.current_game.as_ref().and_then(|current| {
                        current.entry.rom_paths.iter().find(|p| p.exists()).cloned()
                    });
                    replace_with_or_abort(game, |game| match game {
                        LoadedGame::Debugger(debugger) => {
                            if debugger_enabled {
                                LoadedGame::Debugger(debugger)
                            } else {
                                let mut emu = debugger.disable_debugger(
                                    self.settings.use_sgb_colors,
                                    self.settings.frame_blending,
                                );
                                emu.set_palette(palette);
                                LoadedGame::Emulator(emu)
                            }
                        }
                        LoadedGame::Emulator(emulator) => {
                            if debugger_enabled {
                                match emulator.enable_debugger() {
                                    Ok(mut dbg) => {
                                        if let Some(rom_path) = &rom_path {
                                            dbg.load_sidecars(rom_path);
                                        }
                                        dbg.set_palette(palette);
                                        LoadedGame::Debugger(dbg)
                                    }
                                    Err(emulator) => LoadedGame::Emulator(*emulator),
                                }
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

    /// Handle an event from the emulation thread.
    pub(super) fn handle_emu_event(&mut self, event: EmuEvent) -> Task<Message> {
        match event {
            EmuEvent::Started(handle) => {
                self.emu = Some(handle);
                // A game loaded before the thread was ready (e.g. a CLI ROM)
                // starts running now.
                self.start_running();
            }
            EmuEvent::FrameReady => {
                // The printer runs on the emu thread; drain any finished prints
                // into the session log.
                let prints: Vec<_> = self.print_rx.try_iter().collect();
                for print in prints {
                    self.record_print(print);
                }
                let display = self
                    .emu
                    .as_ref()
                    .and_then(|handle| handle.frames().lock().ok()?.take());
                let status = self
                    .emu
                    .as_ref()
                    .and_then(|handle| handle.status().lock().ok()?.clone());
                let snapshot = self
                    .emu
                    .as_ref()
                    .and_then(|handle| handle.snapshot().lock().ok()?.take());
                let memory_window = self
                    .emu
                    .as_ref()
                    .and_then(|handle| handle.memory_window().lock().ok()?.take());
                match &mut self.game {
                    Game::Loaded(LoadedGame::Emulator(emulator)) => {
                        if let Some(display) = display {
                            emulator.apply_frame(display);
                        }
                    }
                    Game::Loaded(LoadedGame::Debugger(debugger)) => {
                        if let Some(display) = display {
                            debugger.apply_frame(display);
                        }
                        if let Some(status) = status {
                            debugger.apply_status(status);
                        }
                        if let Some(snapshot) = snapshot {
                            debugger.apply_snapshot(snapshot);
                        }
                        if let Some(memory_window) = memory_window {
                            debugger.apply_memory_window(memory_window);
                        }
                    }
                    _ => {}
                }
            }
            EmuEvent::BreakpointHit => {
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    if debugger.is_detached()
                        && let Some(handle) = &self.emu
                        && let Some(Payload::Debugger(payload)) = handle.recover()
                    {
                        debugger.restore_payload(payload);
                    }
                    debugger.pause();
                }
                self.save();
            }
            EmuEvent::SramDirty(ram) => {
                if let Some(title) = self
                    .current_game
                    .as_ref()
                    .map(|c| c.cartridge_title.clone())
                {
                    self.persist_sram(&ram, &title);
                }
            }
            EmuEvent::Screenshot(capture) => self.record_screenshot(*capture),
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
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.run(),
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.set_running(true),
            _ => {}
        }
        self.start_running();
    }

    /// Hand the running payload (console or debugger core) to the emu thread
    /// if it should run. Idempotent.
    pub(super) fn start_running(&mut self) {
        let Some(handle) = self.emu.clone() else {
            return;
        };
        match &mut self.game {
            Game::Loaded(LoadedGame::Emulator(emulator)) if emulator.running() => {
                if let Some(console) = emulator.take_console() {
                    handle.run(Payload::Console(console));
                }
            }
            Game::Loaded(LoadedGame::Debugger(debugger)) if debugger.running() => {
                if let Some(payload) = debugger.take_payload() {
                    handle.run(Payload::Debugger(payload));
                    // Aim the vblank memory peek at the pane's current view so
                    // the running browser fills in from the first frame.
                    handle.send(EmuCommand::SetMemoryInterest(debugger.memory_interest()));
                }
            }
            _ => {}
        }
    }

    /// Terminate the emu thread and wait (bounded) for it to tear down its
    /// audio device. Run on every app-close path before `window::close` so the
    /// cpal stream is destroyed on the emu thread, not left to a teardown race.
    pub(super) fn shutdown_emu(&self) {
        if let Some(handle) = &self.emu {
            handle.shutdown();
        }
    }

    pub(super) fn pause(&mut self) {
        // Recover the payload from the emu thread so all inspection and saving
        // paths work synchronously while paused.
        if let Some(handle) = self.emu.clone() {
            let on_emu_thread = match &self.game {
                Game::Loaded(LoadedGame::Emulator(emu)) => emu.running() && emu.console().is_none(),
                Game::Loaded(LoadedGame::Debugger(dbg)) => dbg.running() && dbg.is_detached(),
                _ => false,
            };
            if on_emu_thread {
                match (handle.pause_and_recover(), &mut self.game) {
                    (
                        Some(Payload::Console(console)),
                        Game::Loaded(LoadedGame::Emulator(emulator)),
                    ) => emulator.restore_console(console),
                    (
                        Some(Payload::Debugger(payload)),
                        Game::Loaded(LoadedGame::Debugger(debugger)),
                    ) => debugger.restore_payload(payload),
                    _ => {}
                }
            }
        }
        match &mut self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.pause(),
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.set_running(false),
            _ => {}
        }
        // Persist any pending SRAM from the recovered payload.
        self.save();
    }

    pub(super) fn reset(&mut self) {
        let handle = self.emu.clone();
        match &mut self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                if debugger.is_detached()
                    && let Some(handle) = &handle
                {
                    handle.send(EmuCommand::Reset);
                } else {
                    debugger.reset();
                }
            }
            Game::Loaded(LoadedGame::Emulator(emulator)) => {
                if emulator.running()
                    && let Some(handle) = &handle
                {
                    handle.send(EmuCommand::Reset);
                } else {
                    emulator.reset();
                }
            }
            _ => {}
        }
    }

    pub(super) fn set_control(&mut self, control: ControlId, input: ControlInput) {
        let handle = self.emu.clone();
        match &mut self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                if debugger.is_detached()
                    && let Some(handle) = &handle
                {
                    handle.send(EmuCommand::SetControl(control, input));
                } else {
                    debugger.set_control(control, input);
                }
            }
            Game::Loaded(LoadedGame::Emulator(emulator)) => {
                if emulator.running()
                    && let Some(handle) = &handle
                {
                    handle.send(EmuCommand::SetControl(control, input));
                } else {
                    emulator.set_control(control, input);
                }
            }
            _ => {}
        }
    }

    fn record_screenshot(&mut self, capture: FrameCapture) {
        if let Some(current) = &mut self.current_game
            && let Some(session) = &mut current.session
        {
            session.events.push(library::activity::SessionEvent {
                at: jiff::Timestamp::now(),
                kind: library::activity::EventKind::Screenshot { frame: capture },
            });
            library::activity::write_session(&current.game_dir, session);
            self.store.update_live_screenshots(session);
        }
        self.screenshot_toast = Some(Instant::now());
    }

    fn record_print(&mut self, print: crate::printer::CompletedPrint) {
        if let Some(current) = &mut self.current_game
            && let Some(session) = &mut current.session
        {
            session.events.push(library::activity::SessionEvent {
                at: jiff::Timestamp::now(),
                kind: library::activity::EventKind::Print {
                    print: library::activity::PrintCapture {
                        width: print.width,
                        height: print.height,
                        pixels: print.pixels,
                    },
                },
            });
            library::activity::write_session(&current.game_dir, session);
            self.store.update_live_prints(session);
        }
    }

    pub(super) fn save(&mut self) {
        let (ram, cartridge_title) = match &self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                let Some(ram) = debugger.battery_save() else {
                    return;
                };
                let Some(title) = debugger.game_title() else {
                    return;
                };
                (ram, title)
            }
            Game::Loaded(LoadedGame::Emulator(emulator)) => {
                let Some(console) = emulator.console() else {
                    return;
                };
                let Some(ram) = console.battery_save() else {
                    return;
                };
                (ram, console.game_title())
            }
            _ => return,
        };
        self.persist_sram(&ram, &cartridge_title);
    }

    /// Drain audio from the on-thread debugger to the UI-side output device.
    /// The plain emulator pushes audio directly from the emu thread instead.
    pub(super) fn drain_audio(&mut self) {
        let (samples, coupling) = match &mut self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                (debugger.drain_audio_samples(), debugger.audio_coupling())
            }
            _ => return,
        };
        if let Some(audio) = &mut self.audio_output {
            audio.push_samples(&samples, coupling);
        }
    }

    /// Write an SRAM snapshot to the session if it meaningfully changed.
    fn persist_sram(&mut self, ram: &[u8], cartridge_title: &str) {
        let Some(current) = &mut self.current_game else {
            return;
        };
        let Some(session) = &mut current.session else {
            return;
        };

        // Compare the SRAM portion only — an RTC tail's save timestamp
        // differs every time without the game having saved anything.
        let previous = session.last_sram().or(current.initial_sram.as_deref());
        let changed = match previous {
            Some(prev) => {
                let (new_ram, _) = crate::sram::split_blob(ram.to_vec());
                let (prev_ram, _) = crate::sram::split_blob(prev.to_vec());
                library::game_db::sram_changed(cartridge_title, &new_ram, &prev_ram)
            }
            None => true,
        };

        if changed {
            session.events.push(library::activity::SessionEvent {
                at: jiff::Timestamp::now(),
                kind: library::activity::EventKind::Save { sram: ram.to_vec() },
            });
            library::activity::write_session(&current.game_dir, session);
        }
    }
}
