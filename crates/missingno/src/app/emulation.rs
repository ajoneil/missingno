use std::time::Instant;

use iced::Task;
use missingno_session::{ExtractedMachine, SessionEvent, SessionHandle, SharedSession};

use super::audio_output::AudioOutput;
use super::emulator::{ConsoleFacts, Emulator};
use super::session_bridge::{self, SessionBridge};
use super::{App, Game, LoadedGame, Message, PendingAction, library};
use crate::app::library::activity::FrameCapture;
use crate::app::system::{ControlId, ControlInput};

impl App {
    /// A fresh client handle onto the current game's session, if one is loaded.
    pub(super) fn handle(&self) -> Option<SessionHandle> {
        self.session.as_ref().map(|session| session.handle())
    }

    /// Wire the current session's events into the UI: spawn a per-game bridge
    /// thread forwarding them into the app's Iced sink. A no-op before the
    /// subscription has handed over the sink (a CLI ROM loaded at startup — the
    /// bridge is spawned when the sink arrives).
    pub(super) fn attach_session_bridge(&self) {
        if let (Some(session), Some(sink)) = (&self.session, &self.event_sink) {
            session_bridge::spawn_bridge(&session.handle(), sink.clone());
        }
    }

    /// Open an audio device for a freshly spawned session: the UI-thread stream
    /// holder to keep alive, and the Send sink the session drains into. `None`
    /// silences the game when no device is available.
    pub(super) fn open_audio() -> (Option<AudioOutput>, Option<missingno_session::AudioSink>) {
        match AudioOutput::open() {
            Some((output, sink)) => (Some(output), Some(sink)),
            None => (None, None),
        }
    }

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
                // The session captures the current frame whether running or
                // paused; one path for both shells.
                if let Some(handle) = self.handle()
                    && let Some(frame) = handle.screenshot()
                {
                    let capture = FrameCapture::from_frame(&frame, &options);
                    self.record_screenshot(capture);
                }
            }
            Message::SaveState | Message::LoadState => {
                // The state slot lives beside the game's library folder.
                let Some(path) = self
                    .current_game
                    .as_ref()
                    .map(|current| current.game_dir.join("state.mpsv"))
                else {
                    return Task::none();
                };
                if let Some(handle) = self.handle() {
                    // The session announces the outcome as a notice either way.
                    let _ = match message {
                        Message::SaveState => handle.save_state(path),
                        Message::LoadState => {
                            // The refresh serializes after the load on the session's
                            // one request channel, so it reads the loaded state.
                            let outcome = handle.load_state(path);
                            if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game
                                && !debugger.running()
                            {
                                debugger.sync_paused();
                            }
                            outcome
                        }
                        _ => unreachable!(),
                    };
                }
            }
            Message::ToggleRecording | Message::Replay => {
                // Recordings live beside the game's library folder, like the
                // save-state slot.
                let Some(path) = self
                    .current_game
                    .as_ref()
                    .map(|current| current.game_dir.join("recording.mprc"))
                else {
                    return Task::none();
                };
                if let Some(handle) = self.handle() {
                    // The recording flag follows the session's `RecordingChanged`
                    // event, never this click — a start can fail or defer.
                    let result = match message {
                        Message::ToggleRecording if self.recording => handle.stop_recording(),
                        Message::ToggleRecording => handle.start_recording(path),
                        Message::Replay => handle.play_recording(path),
                        _ => unreachable!(),
                    };
                    if let Err(error) = result {
                        self.show_notice(error);
                    }
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
            Message::DismissNotice => {
                self.notice = None;
            }
            Message::SetControl(control, pressed) => {
                self.set_control(ControlId(control), ControlInput::Digital(pressed))
            }
            Message::SetAxis(control, value) => {
                self.set_control(ControlId(control), ControlInput::Axis(value))
            }
            Message::ToggleDebugger(debugger_enabled) => {
                self.debugger_enabled = debugger_enabled;
                self.toggle_debugger_mode(debugger_enabled);
            }
            _ => {}
        }

        Task::none()
    }

    /// Swap the current game between debugger and emulator shells, keeping the
    /// live console (its serial link, printer, cartridge state) by handing it
    /// from the old session to a fresh one of the other kind.
    fn toggle_debugger_mode(&mut self, want_debugger: bool) {
        // Toggle on a stopped machine.
        let was_running = self.running();
        self.pause();

        let currently_debugger = matches!(&self.game, Game::Loaded(LoadedGame::Debugger(_)));
        if currently_debugger == want_debugger {
            return;
        }

        let palette = self.settings.palette;
        let presentation = self.settings.presentation();
        let rom_path = self
            .current_game
            .as_ref()
            .and_then(|current| current.entry.rom_paths.iter().find(|p| p.exists()).cloned());

        // Take the session while its thread is still alive so the old shell's
        // handle keeps working for the handoff prep below. The socket points at
        // that thread, so it closes before the handoff consumes it.
        self.unpublish_session();
        let Some(session) = self.session.take() else {
            return;
        };
        let old_game = std::mem::replace(&mut self.game, Game::Unloaded);
        let (audio, sink) = Self::open_audio();

        match old_game {
            Game::Loaded(LoadedGame::Debugger(mut debugger)) if !want_debugger => {
                debugger.prepare_handoff();
                let screen_view = debugger.take_screen_view();
                let platform = debugger.platform();
                drop(debugger);
                let Some(ExtractedMachine::Debugger(core)) = session.into_machine() else {
                    return;
                };
                let console = core.into_console();
                let facts = ConsoleFacts::of(console.as_ref());
                let new_session = SharedSession::spawn_console_with_audio(console, sink);
                let handle = new_session.handle();
                let mut emulator =
                    Emulator::from_debugger(handle, screen_view, facts, platform, presentation);
                emulator.set_palette(palette);
                self.game = Game::Loaded(LoadedGame::Emulator(emulator));
                self.install_session(new_session, audio);
            }
            Game::Loaded(LoadedGame::Emulator(mut emulator)) if want_debugger => {
                let mut screen_view = emulator.take_screen_view();
                let platform = emulator.platform();
                drop(emulator);
                let Some(ExtractedMachine::Console(console)) = session.into_machine() else {
                    return;
                };
                let technology = console.video_out();
                match console.into_debugger() {
                    Ok(core) => {
                        let regions = core.memory_regions();
                        let new_session = SharedSession::spawn_with_audio(core, sink);
                        let handle = new_session.handle();
                        screen_view.set_technology(technology);
                        let mut debugger = crate::app::debugger::Debugger::new(
                            handle,
                            platform,
                            regions,
                            screen_view,
                        );
                        if let Some(rom_path) = &rom_path {
                            debugger.load_sidecars(rom_path);
                        }
                        debugger.set_palette(palette);
                        self.game = Game::Loaded(LoadedGame::Debugger(debugger));
                        self.install_session(new_session, audio);
                    }
                    // No debugger backend: re-host as a plain console, staying in
                    // emulator mode.
                    Err(console) => {
                        let facts = ConsoleFacts::of(console.as_ref());
                        let new_session = SharedSession::spawn_console_with_audio(console, sink);
                        let handle = new_session.handle();
                        let emulator = Emulator::from_debugger(
                            handle,
                            screen_view,
                            facts,
                            platform,
                            presentation,
                        );
                        self.game = Game::Loaded(LoadedGame::Emulator(emulator));
                        self.install_session(new_session, audio);
                    }
                }
            }
            // The shell already matches the wanted mode (the early return above
            // usually catches this): put everything back.
            other => {
                drop(sink);
                self.session = Some(session);
                self.game = other;
                return;
            }
        }

        if was_running {
            self.run();
        }
    }

    /// Install a freshly spawned session as the current one: keep its audio
    /// stream alive and wire its event bridge.
    pub(super) fn install_session(&mut self, session: SharedSession, audio: Option<AudioOutput>) {
        self.unpublish_session();
        self.session = Some(session);
        self.audio_output = audio;
        self.attach_session_bridge();
        self.publish_session();
    }

    /// Stop publishing. An endpoint holds a handle onto one session, so it must
    /// close before that session ends — a client reaching a session whose thread
    /// has gone gets no answer at all.
    #[cfg(unix)]
    pub(super) fn unpublish_session(&mut self) {
        self.attach_endpoint = None;
    }

    #[cfg(not(unix))]
    pub(super) fn unpublish_session(&mut self) {}

    /// Reconcile the attach socket with the user's permission and the current
    /// session.
    #[cfg(unix)]
    pub(super) fn publish_session(&mut self) {
        self.unpublish_session();
        if !self.settings.allow_external_clients {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let publication = missingno_session::Publication {
            title: self
                .current_game
                .as_ref()
                .map(|current| current.entry.display_title().to_string())
                .unwrap_or_else(|| "Missingno".into()),
            core: self
                .platform()
                .map(|platform| platform.name().to_string())
                .unwrap_or_default(),
        };
        match missingno_session::AttachEndpoint::open(session.handle(), publication) {
            Ok(endpoint) => self.attach_endpoint = Some(endpoint),
            Err(error) => self.show_notice(format!("Could not allow external clients: {error}")),
        }
    }

    #[cfg(not(unix))]
    pub(super) fn publish_session(&mut self) {}

    /// The platform of the loaded game, if one is loaded.
    fn platform(&self) -> Option<crate::app::system::Platform> {
        match &self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => Some(debugger.platform()),
            Game::Loaded(LoadedGame::Emulator(emulator)) => Some(emulator.platform()),
            _ => None,
        }
    }

    /// Handle an item from the app-lifetime session subscription.
    pub(super) fn handle_session_bridge(&mut self, bridge: SessionBridge) -> Task<Message> {
        match bridge {
            SessionBridge::Ready(sink) => {
                self.event_sink = Some(sink);
                // A game loaded before the sink arrived (a CLI ROM) has a session
                // but no bridge yet — wire it now.
                self.attach_session_bridge();
            }
            SessionBridge::Event(event) => return self.handle_session_event(event),
        }
        Task::none()
    }

    /// Handle a single session event forwarded through the bridge.
    fn handle_session_event(&mut self, event: SessionEvent) -> Task<Message> {
        match event {
            SessionEvent::FrameReady => {
                // The printer runs against the console on the session thread;
                // drain any finished prints into the session log.
                let prints: Vec<_> = self.print_rx.try_iter().collect();
                for print in prints {
                    self.record_print(print);
                }
                let handle = self.handle();
                let display = handle.as_ref().and_then(|handle| handle.latest_frame());
                let status = handle.as_ref().and_then(|handle| handle.latest_status());
                let snapshot = handle.as_ref().and_then(|handle| handle.take_snapshot());
                let memory_windows = handle
                    .as_ref()
                    .map(|handle| handle.latest_memory_windows())
                    .unwrap_or_default();
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
                        debugger.apply_memory_windows(memory_windows);
                    }
                    _ => {}
                }
            }
            SessionEvent::Stopped => {
                // A breakpoint/watch stopped the free-run; the session is already
                // paused. Rebuild the paused view and persist any battery save.
                if let Game::Loaded(LoadedGame::Debugger(debugger)) = &mut self.game {
                    debugger.sync_paused();
                }
                self.save();
            }
            SessionEvent::SramDirty(ram) => {
                if let Some(title) = self
                    .current_game
                    .as_ref()
                    .map(|c| c.cartridge_title.clone())
                {
                    self.persist_sram(&ram, &title);
                }
            }
            // The flag follows the session's truth, not the toggle click.
            SessionEvent::RecordingChanged(active) => self.recording = active,
            SessionEvent::ReplayDiverged {
                frame,
                expected,
                actual,
            } => self.show_notice(format!(
                "Replay diverged at frame {frame} (expected {expected:#x}, got {actual:#x})"
            )),
            SessionEvent::Notice(message) => self.show_notice(message),
        }
        Task::none()
    }

    /// Show a transient status-line toast.
    fn show_notice(&mut self, message: String) {
        self.notice = Some((message, Instant::now()));
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
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.run(),
            _ => {}
        }
    }

    /// Terminate the current session (finalizing any recording, joining its
    /// thread) and drop the cpal stream — both on the UI thread so teardown never
    /// races the audio backend. Run on every app-close path before `window::close`.
    pub(super) fn shutdown_emu(&mut self) {
        self.unpublish_session();
        self.session = None;
        self.audio_output = None;
    }

    pub(super) fn pause(&mut self) {
        match &mut self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.pause(),
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.pause(),
            _ => {}
        }
        // Persist any pending SRAM now that the machine is stopped.
        self.save();
    }

    pub(super) fn reset(&mut self) {
        match &mut self.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => debugger.reset(),
            Game::Loaded(LoadedGame::Emulator(emulator)) => emulator.reset(),
            _ => {}
        }
    }

    pub(super) fn set_control(&mut self, control: ControlId, input: ControlInput) {
        if let Some(handle) = self.handle() {
            handle.set_control(control, input);
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
        let Some(handle) = self.handle() else {
            return;
        };
        let Some(ram) = handle.battery_save() else {
            return;
        };
        let Some(cartridge_title) = self
            .current_game
            .as_ref()
            .map(|current| current.cartridge_title.clone())
        else {
            return;
        };
        self.persist_sram(&ram, &cartridge_title);
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
