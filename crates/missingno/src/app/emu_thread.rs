//! The dedicated emulation thread and its command/event protocol.
//!
//! Emulation runs on a single `std::thread` that owns the payload (a console, or
//! a debugger core) while a game runs, so the Iced UI thread never blocks on a
//! frame. The UI drives it through [`EmuCommand`]s and observes it through
//! [`EmuEvent`]s (bridged into an Iced subscription) plus a latest-frame slot.

use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel},
};
use std::time::{Duration, Instant};

use missingno_core::inspect::{MemoryWindow, Watch};
use missingno_core::recording::{Recorder, Recording, frame_hash};
use missingno_core::system::StateError;

use super::audio_output::AudioOutput;
use super::debugger::inspect::DebugView;
use super::debugger::memory::MemoryInterest;
use super::library::activity::{CaptureOptions, FrameCapture};
use super::screen::Frame;
use super::system::{
    ControlId, ControlInput, FrameOutcome, StepOutcome, SystemConsole, SystemDebugger,
};

/// Backlog cap, in frames: falling further behind schedule than this drops
/// the deficit — degrades to slow-but-steady instead of spiralling.
const MAX_DEFICIT_FRAMES: u32 = 4;

/// Frames of quiet before a debounced SRAM save is emitted. Games write SRAM
/// across several consecutive frames during a save; we wait for writes to stop.
const SRAM_DEBOUNCE_FRAMES: u32 = 30;

/// How often a recording checkpoints a frame hash, for replay-divergence
/// detection — every few seconds of play at ~60 fps.
const RECORDING_CHECK_INTERVAL: u64 = 300;

/// The latest fully-rendered frame, overwritten each `new_screen`. A latest-value
/// handoff, not a queue: the UI reads whatever is current on redraw.
pub type FrameSlot = Arc<Mutex<Option<Frame>>>;

/// The emulatable payload the emu thread owns while running: the plain
/// console, or the debugger's core (console + breakpoints) in debugger mode.
/// The debugger's pane/layout state stays on the UI thread.
pub enum Payload {
    Console(Box<dyn SystemConsole>),
    Debugger(DebuggerPayload),
}

/// The debugger state that moves to the emu thread while running: the core
/// (console, breakpoints, counters) plus the UI's frame counter. Pane and
/// layout state stays behind on the UI thread.
pub struct DebuggerPayload {
    pub core: Box<dyn SystemDebugger>,
    pub frame: u64,
}

pub use missingno_core::system::RunningStatus;

/// Latest-value handoff for [`RunningStatus`], written alongside the frame slot.
pub type StatusSlot = Arc<Mutex<Option<RunningStatus>>>;

/// Latest-value handoff for the debugger's per-vblank inspection snapshot. Only
/// written while a debugger payload runs; `None` for plain-console payloads.
pub type SnapshotSlot = Arc<Mutex<Option<DebugView>>>;

/// Latest-value handoff for the memory viewers' interest windows: one window
/// per open memory pane, peeked at the vblank boundary. Only written while a
/// debugger payload runs with at least one interest set.
/// `None` when no window set has been published since the last take (keep the
/// last); `Some` for a fresh publish, empty when interest just went away so the
/// consumer clears rather than pinning a stale peek.
pub type MemoryWindowSlot = Arc<Mutex<Option<Vec<MemoryWindow>>>>;

/// The latest-value publish slots the emu thread writes each frame, grouped so
/// they thread through construction as one handle.
struct PublishSlots {
    frames: FrameSlot,
    status: StatusSlot,
    snapshot: SnapshotSlot,
    memory_window: MemoryWindowSlot,
}

/// Commands the UI sends to the emu thread. The thread terminates when the
/// command channel is dropped (the UI holds the only sender).
pub enum EmuCommand {
    /// Take ownership of a payload and start running it.
    Run(Payload),
    /// Stop running and return the payload on the sync return channel. The UI
    /// persists any final SRAM from the recovered payload synchronously.
    Pause,
    Reset,
    /// Drop the audio device and terminate the thread. The thread acknowledges
    /// on the shutdown-ack channel once the cpal stream is destroyed, so the UI
    /// can hold the process open until teardown completes.
    Shutdown,
    SetControl(ControlId, ControlInput),
    SetBreakpoint(u32),
    ClearBreakpoint(u32),
    AddWatchpoint(Watch),
    RemoveWatchpoint(Watch),
    /// The open memory viewers' view spans, peeked each vblank and published to
    /// the memory-window slot. Empty clears them (the last pane closed).
    SetMemoryInterest(Vec<MemoryInterest>),
    /// Enable or disable the debugger's per-channel waveform capture. Stored on
    /// the loop and re-applied to each payload it runs.
    SetWaveCapture(bool),
    /// Enable or disable the debugger's per-vblank graphics-surface capture.
    /// Stored on the loop and re-applied to each payload it runs.
    SetGraphicsCapture(bool),
    RequestScreenshot {
        options: CaptureOptions,
    },
    /// Write the running machine state to a save file.
    SaveState(std::path::PathBuf),
    /// Restore the running machine state from a save file.
    LoadState(std::path::PathBuf),
    /// Begin capturing an input recording to the given file (finalized on
    /// [`StopRecording`](Self::StopRecording)). Play-mode consoles only.
    StartRecording(std::path::PathBuf),
    /// Finish the active recording and write it out.
    StopRecording,
    /// Load a recording and replay it, driving the running console frame by
    /// frame so the playback is watchable.
    PlayRecording(std::path::PathBuf),
}

/// Events the emu thread sends to the UI (via the Iced subscription).
#[derive(Clone, Debug)]
pub enum EmuEvent {
    /// First event: hands the UI the channels it drives the thread with.
    Started(EmuHandle),
    /// A new frame is in the slot — a wake hint, no payload.
    FrameReady,
    /// Debounced SRAM contents, ready to persist.
    SramDirty(Vec<u8>),
    /// A requested screenshot capture (boxed — `FrameCapture` is large).
    Screenshot(Box<FrameCapture>),
    /// The running debugger hit a breakpoint; its payload is on the return
    /// channel and the UI should switch to the paused view.
    BreakpointHit,
    /// A user-facing status line the UI surfaces (save/load result, recording
    /// lifecycle note, replay outcome).
    Notice(String),
    /// Input-recording capture turned on or off. The UI mirrors this into its
    /// recording flag — the flag follows the event, never the click.
    RecordingChanged(bool),
    /// A replay checkpoint disagreed with the recorded timeline; playback
    /// stopped at this frame.
    ReplayDiverged {
        frame: u64,
        expected: u64,
        actual: u64,
    },
}

/// The UI-side handle to the emu thread. Cloneable so it can ride in a Message;
/// the return receiver is shared behind a mutex (single consumer in practice).
#[derive(Clone)]
pub struct EmuHandle {
    commands: Sender<EmuCommand>,
    frames: FrameSlot,
    status: StatusSlot,
    snapshot: SnapshotSlot,
    memory_window: MemoryWindowSlot,
    returns: Arc<Mutex<Receiver<Payload>>>,
    shutdown_ack: Arc<Mutex<Receiver<()>>>,
}

// The snapshot slot holds a `DebugView`, which isn't `Debug`; a hand-rolled
// impl keeps `EmuHandle` (and the `EmuEvent` that carries it) printable.
impl std::fmt::Debug for EmuHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmuHandle").finish_non_exhaustive()
    }
}

impl EmuHandle {
    pub fn frames(&self) -> &FrameSlot {
        &self.frames
    }

    pub fn status(&self) -> &StatusSlot {
        &self.status
    }

    pub fn snapshot(&self) -> &SnapshotSlot {
        &self.snapshot
    }

    pub fn memory_window(&self) -> &MemoryWindowSlot {
        &self.memory_window
    }

    pub fn send(&self, command: EmuCommand) {
        let _ = self.commands.send(command);
    }

    /// Send a payload to the thread and start running it.
    pub fn run(&self, payload: Payload) {
        self.send(EmuCommand::Run(payload));
    }

    /// Enable or disable the running debugger's per-channel waveform capture,
    /// driven by whether the audio scope pane is open.
    pub fn set_wave_capture(&self, on: bool) {
        self.send(EmuCommand::SetWaveCapture(on));
    }

    /// Enable or disable the running debugger's per-vblank graphics-surface
    /// capture, driven by whether a graphics pane is open.
    pub fn set_graphics_capture(&self, on: bool) {
        self.send(EmuCommand::SetGraphicsCapture(on));
    }

    /// Pause and recover the payload synchronously (bounded wait). Returns
    /// `None` if the thread was already idle or did not respond in time.
    pub fn pause_and_recover(&self) -> Option<Payload> {
        self.send(EmuCommand::Pause);
        self.recover()
    }

    /// Recover the payload the thread returned in response to `Pause` or a
    /// breakpoint stop.
    pub fn recover(&self) -> Option<Payload> {
        self.returns
            .lock()
            .ok()?
            .recv_timeout(Duration::from_millis(500))
            .ok()
    }

    /// Drop the thread's audio device and terminate it, blocking (bounded)
    /// until it confirms teardown. Called on app close so the cpal stream is
    /// destroyed on the emu thread before the process exits — otherwise the OS
    /// audio backend can invoke the stream callback into freed memory.
    pub fn shutdown(&self) {
        self.send(EmuCommand::Shutdown);
        if let Ok(ack) = self.shutdown_ack.lock() {
            let _ = ack.recv_timeout(Duration::from_millis(500));
        }
    }
}

/// The Iced subscription worker. A non-capturing `fn` (required by
/// `Subscription::run`): it creates the channels, spawns the emu thread, and
/// hands the UI its [`EmuHandle`] as the first stream item.
pub fn subscription_worker() -> impl iced::futures::Stream<Item = EmuEvent> {
    use iced::futures::channel::mpsc::unbounded;

    let (event_tx, event_rx) = unbounded::<EmuEvent>();
    let (command_tx, command_rx) = channel::<EmuCommand>();
    let (return_tx, return_rx) = channel::<Payload>();
    let (shutdown_ack_tx, shutdown_ack_rx) = channel::<()>();
    let slots = PublishSlots {
        frames: Arc::new(Mutex::new(None)),
        status: Arc::new(Mutex::new(None)),
        snapshot: Arc::new(Mutex::new(None)),
        memory_window: Arc::new(Mutex::new(None)),
    };

    let handle = EmuHandle {
        commands: command_tx,
        frames: slots.frames.clone(),
        status: slots.status.clone(),
        snapshot: slots.snapshot.clone(),
        memory_window: slots.memory_window.clone(),
        returns: Arc::new(Mutex::new(return_rx)),
        shutdown_ack: Arc::new(Mutex::new(shutdown_ack_rx)),
    };
    let _ = event_tx.unbounded_send(EmuEvent::Started(handle));

    let worker_events = event_tx;
    std::thread::Builder::new()
        .name("emu".into())
        .spawn(move || run_emu_thread(command_rx, return_tx, shutdown_ack_tx, slots, worker_events))
        .expect("spawn emu thread");

    event_rx
}

type EventSink = iced::futures::channel::mpsc::UnboundedSender<EmuEvent>;

fn run_emu_thread(
    commands: Receiver<EmuCommand>,
    returns: Sender<Payload>,
    shutdown_ack: Sender<()>,
    slots: PublishSlots,
    events: EventSink,
) {
    // Audio device lives on this thread (cpal's Stream is `!Send`).
    let mut audio = AudioOutput::new();
    let mut state = EmuLoop::new(slots, events, returns, shutdown_ack);

    'thread: loop {
        if state.running() {
            // Drain pending commands without blocking, then emulate one frame.
            loop {
                match commands.try_recv() {
                    Ok(command) => {
                        state.handle(command);
                        if state.shutdown_requested() {
                            break 'thread;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'thread,
                }
            }
            if state.running() {
                state.emulate_frame(&mut audio);
                let interval = state.frame_interval();
                state.pace(interval);
            }
        } else {
            // Idle: block until the next command (with a timeout so a paused
            // thread stays responsive if the UI drops the channel).
            match commands.recv_timeout(Duration::from_millis(200)) {
                Ok(command) => {
                    state.handle(command);
                    if state.shutdown_requested() {
                        break 'thread;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break 'thread,
            }
        }
    }

    // Destroy the cpal stream on this thread before acknowledging, so the OS
    // audio backend can't call the stream callback into freed memory once the
    // process starts tearing down. The UI's `shutdown()` blocks on this ack.
    drop(audio);
    state.confirm_shutdown();
}

struct EmuLoop {
    payload: Option<Payload>,
    frames: FrameSlot,
    status: StatusSlot,
    snapshot: SnapshotSlot,
    memory_window: MemoryWindowSlot,
    /// The open memory viewers' view spans, peeked each vblank into the
    /// memory-window slot. Empty when no memory pane is browsing.
    memory_interest: Vec<MemoryInterest>,
    /// Whether the audio scope wants per-channel waveform capture. Re-applied
    /// to each payload the loop starts running.
    wave_capture: bool,
    /// Whether a graphics pane wants per-vblank graphics-surface capture.
    /// Re-applied to each payload the loop starts running.
    graphics_capture: bool,
    events: EventSink,
    returns: Sender<Payload>,
    shutdown_ack: Sender<()>,
    shutdown_requested: bool,
    sram_countdown: Option<u32>,
    next_deadline: Instant,
    /// The active input recording and its destination file, while recording.
    recorder: Option<(Recorder, std::path::PathBuf)>,
    /// The recording being replayed frame by frame, while replaying.
    replay: Option<ReplayPlayback>,
    /// A save requested off an instruction boundary (a VCS frame boundary need
    /// not be one): retried at each frame until the console reaches a boundary
    /// and its state serializes.
    pending_save: Option<std::path::PathBuf>,
    /// A recording-start requested off an instruction boundary, retried likewise
    /// — its initial state is a save, so it has the same boundary requirement.
    pending_record: Option<std::path::PathBuf>,
}

/// A recording played back through the running console: applies the recorded
/// inputs at their frame boundaries and checks the frame-hash checkpoints,
/// reporting the frame a divergence first appears on.
struct ReplayPlayback {
    recording: Recording,
    frame: u64,
    input_cursor: usize,
    check_cursor: usize,
}

/// What one replayed frame concluded: keep going, stop at a divergence, or the
/// whole recording has been replayed.
enum ReplayStep {
    Continue,
    Diverged {
        frame: u64,
        expected: u64,
        actual: u64,
    },
    Finished,
}

/// The result of attempting to serialize the running state to a file.
enum SaveOutcome {
    /// Written to disk.
    Saved,
    /// The console has a backend but is off an instruction boundary; retry once
    /// a frame steps it forward.
    Retry,
    /// The system has no save-state backend at all.
    NoBackend,
    /// The state serialized but the file write failed.
    Failed(String),
}

impl ReplayPlayback {
    fn new(recording: Recording) -> Self {
        Self {
            recording,
            frame: 0,
            input_cursor: 0,
            check_cursor: 0,
        }
    }

    /// Apply the inputs scheduled for the current frame boundary.
    fn apply_inputs(&mut self, payload: &mut Payload) {
        while let Some(event) = self.recording.inputs.get(self.input_cursor) {
            if event.frame != self.frame {
                break;
            }
            payload.set_control(event.control, event.input);
            self.input_cursor += 1;
        }
    }

    /// Verify the checkpoint for the frame just produced, then advance. A
    /// disagreeing checkpoint stops playback at the divergent frame rather than
    /// continuing on a timeline the recording no longer describes.
    fn note_frame(&mut self, produced: Option<&Frame>) -> ReplayStep {
        if let Some(check) = self.recording.checks.get(self.check_cursor)
            && check.frame == self.frame
        {
            let hash = produced.map(frame_hash).unwrap_or(0);
            self.check_cursor += 1;
            if check.hash != hash {
                return ReplayStep::Diverged {
                    frame: self.frame,
                    expected: check.hash,
                    actual: hash,
                };
            }
        }
        self.frame += 1;
        if self.frame < self.recording.frames {
            ReplayStep::Continue
        } else {
            ReplayStep::Finished
        }
    }
}

impl EmuLoop {
    fn new(
        slots: PublishSlots,
        events: EventSink,
        returns: Sender<Payload>,
        shutdown_ack: Sender<()>,
    ) -> Self {
        Self {
            payload: None,
            frames: slots.frames,
            status: slots.status,
            snapshot: slots.snapshot,
            memory_window: slots.memory_window,
            memory_interest: Vec::new(),
            wave_capture: false,
            graphics_capture: false,
            events,
            returns,
            shutdown_ack,
            shutdown_requested: false,
            sram_countdown: None,
            next_deadline: Instant::now(),
            recorder: None,
            replay: None,
            pending_save: None,
            pending_record: None,
        }
    }

    fn running(&self) -> bool {
        self.payload.is_some()
    }

    fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Confirm to the UI that the thread has torn down and is exiting.
    fn confirm_shutdown(&self) {
        let _ = self.shutdown_ack.send(());
    }

    fn handle(&mut self, command: EmuCommand) {
        match command {
            EmuCommand::Run(mut payload) => {
                payload.set_wave_capture(self.wave_capture);
                payload.set_graphics_capture(self.graphics_capture);
                self.payload = Some(payload);
                self.sram_countdown = None;
                self.next_deadline = Instant::now();
                // A fresh session inherits no recording, replay, or pending
                // save/record from the last.
                self.recorder = None;
                self.replay = None;
                self.pending_save = None;
                self.pending_record = None;
            }
            EmuCommand::Pause => self.return_payload(),
            EmuCommand::Shutdown => self.shutdown_requested = true,
            EmuCommand::Reset => {
                // A reset invalidates any recording-in-progress and any pending
                // save; finalize the recording rather than corrupt it.
                self.finalize_recording("Recording saved before reset");
                self.replay = None;
                self.pending_save = None;
                if let Some(payload) = &mut self.payload {
                    payload.reset();
                }
            }
            EmuCommand::SetControl(control, input) => {
                if let Some(payload) = &mut self.payload {
                    payload.set_control(control, input);
                }
                if let Some((recorder, _)) = &mut self.recorder {
                    recorder.note_input(control, input);
                }
            }
            EmuCommand::SetBreakpoint(address) => {
                if let Some(payload) = &mut self.payload {
                    payload.set_breakpoint(address);
                }
            }
            EmuCommand::ClearBreakpoint(address) => {
                if let Some(payload) = &mut self.payload {
                    payload.clear_breakpoint(address);
                }
            }
            EmuCommand::AddWatchpoint(watch) => {
                if let Some(payload) = &mut self.payload {
                    payload.add_watch(watch);
                }
            }
            EmuCommand::RemoveWatchpoint(watch) => {
                if let Some(payload) = &mut self.payload {
                    payload.remove_watch(&watch);
                }
            }
            EmuCommand::SetMemoryInterest(interest) => {
                self.memory_interest = interest;
                // Interest gone empty (the last memory pane closed): publish an
                // empty set once so the consumer clears the pinned windows
                // rather than keeping the last peek shown.
                if self.memory_interest.is_empty()
                    && let Ok(mut slot) = self.memory_window.lock()
                {
                    *slot = Some(Vec::new());
                }
            }
            EmuCommand::SetWaveCapture(on) => {
                self.wave_capture = on;
                if let Some(payload) = &mut self.payload {
                    payload.set_wave_capture(on);
                }
            }
            EmuCommand::SetGraphicsCapture(on) => {
                self.graphics_capture = on;
                if let Some(payload) = &mut self.payload {
                    payload.set_graphics_capture(on);
                }
            }
            EmuCommand::RequestScreenshot { options } => {
                if let Some(payload) = &self.payload {
                    let capture = FrameCapture::from_frame(&payload.screen_display(), &options);
                    let _ = self
                        .events
                        .unbounded_send(EmuEvent::Screenshot(Box::new(capture)));
                }
            }
            EmuCommand::SaveState(path) => match self.attempt_save(&path) {
                SaveOutcome::Saved => self.notice("State saved"),
                // Off an instruction boundary: retry when a frame reaches one.
                SaveOutcome::Retry => self.pending_save = Some(path),
                SaveOutcome::NoBackend => self.notice("this system has no save-state backend"),
                SaveOutcome::Failed(error) => self.notice(error),
            },
            EmuCommand::LoadState(path) => {
                // A load replaces the state a recording continues from and the
                // state a pending save would capture; finalize the recording and
                // drop both rather than corrupt them.
                self.finalize_recording("Recording saved before load");
                self.replay = None;
                self.pending_save = None;
                if let Some(payload) = &mut self.payload {
                    match payload.load_state(&path) {
                        Ok(()) => self.notice("State loaded"),
                        Err(error) => self.notice(error),
                    }
                }
            }
            EmuCommand::StartRecording(path) => self.begin_recording(path),
            EmuCommand::StopRecording => self.finalize_recording("Recording saved"),
            EmuCommand::PlayRecording(path) => {
                // Replaying while recording would fold the replay's applied
                // inputs (which bypass the recorder) into the capture; refuse.
                if self.recorder.is_some() || self.pending_record.is_some() {
                    self.notice("Stop recording before replaying");
                    return;
                }
                match std::fs::read(&path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        Recording::from_bytes(&bytes).map_err(|error| error.to_string())
                    }) {
                    Ok(recording) => match &mut self.payload {
                        Some(Payload::Console(console)) => {
                            match console.load_state(&recording.initial_state) {
                                Ok(()) => self.replay = Some(ReplayPlayback::new(recording)),
                                Err(error) => {
                                    self.notice(format!("Replay could not restore: {error}"))
                                }
                            }
                        }
                        _ => self.notice("Replay is only available in play mode"),
                    },
                    Err(error) => self.notice(format!("Could not read recording: {error}")),
                }
            }
        }
    }

    /// Emit a user-facing status line to the UI.
    fn notice(&self, message: impl Into<String>) {
        let _ = self.events.unbounded_send(EmuEvent::Notice(message.into()));
    }

    /// Attempt to serialize the running state to `path`. `save_state` returns
    /// `None` both when a console has no backend and when it is momentarily off
    /// an instruction boundary; a console that authors a state schema has a
    /// backend, so `None` there means "retry at the next boundary".
    fn attempt_save(&self, path: &std::path::Path) -> SaveOutcome {
        let Some(payload) = &self.payload else {
            return SaveOutcome::NoBackend;
        };
        match payload.save_state_bytes() {
            Some(bytes) => match std::fs::write(path, bytes) {
                Ok(()) => SaveOutcome::Saved,
                Err(error) => SaveOutcome::Failed(format!("could not write save state: {error}")),
            },
            None if payload.has_state_backend() => SaveOutcome::Retry,
            None => SaveOutcome::NoBackend,
        }
    }

    /// Begin an input recording, first finalizing any recording already running
    /// (so its file is written, not dropped) and refusing during a replay.
    fn begin_recording(&mut self, path: std::path::PathBuf) {
        if self.replay.is_some() {
            self.notice("Cannot record during replay");
            return;
        }
        self.finalize_recording("Previous recording saved");
        match &mut self.payload {
            Some(Payload::Console(console)) => {
                match Recorder::start(console.as_mut(), RECORDING_CHECK_INTERVAL) {
                    Ok(recorder) => {
                        self.recorder = Some((recorder, path));
                        let _ = self.events.unbounded_send(EmuEvent::RecordingChanged(true));
                    }
                    // A schema-authoring console has a backend, so an
                    // `Unsupported` there is really "off an instruction boundary"
                    // (its initial save missed) — retry at the next frame. Any
                    // other error (e.g. a double-speed boundary) is genuine.
                    Err(StateError::Unsupported) if console.state_schema().is_some() => {
                        self.pending_record = Some(path)
                    }
                    Err(error) => self.notice(format!("recording failed: {error}")),
                }
            }
            _ => self.notice("recording is only available in play mode"),
        }
    }

    /// Finish and write any active recording, emitting the flag change and a
    /// note. A no-op (and silent) when nothing is recording.
    fn finalize_recording(&mut self, note: &str) {
        self.pending_record = None;
        let Some((recorder, path)) = self.recorder.take() else {
            return;
        };
        let _ = self
            .events
            .unbounded_send(EmuEvent::RecordingChanged(false));
        match recorder.finish().to_bytes() {
            Ok(bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => self.notice(note),
                Err(error) => self.notice(format!("could not write recording: {error}")),
            },
            Err(error) => self.notice(format!("could not encode recording: {error}")),
        }
    }

    /// Retry a save or recording-start that was requested off an instruction
    /// boundary, now that a frame has stepped the console forward.
    fn drive_pending_state(&mut self) {
        if let Some(path) = self.pending_save.take() {
            match self.attempt_save(&path) {
                SaveOutcome::Saved => self.notice("State saved"),
                SaveOutcome::Retry => self.pending_save = Some(path),
                SaveOutcome::NoBackend => self.notice("this system has no save-state backend"),
                SaveOutcome::Failed(error) => self.notice(error),
            }
        }
        if let Some(path) = self.pending_record.take() {
            self.begin_recording(path);
        }
    }

    fn return_payload(&mut self) {
        // Finalize any in-progress recording before the payload leaves the
        // thread, and drop a running replay and pending save — all belong to
        // this session.
        self.finalize_recording("Recording saved");
        self.replay = None;
        self.pending_save = None;
        if let Some(payload) = self.payload.take() {
            let _ = self.returns.send(payload);
        }
    }

    fn emulate_frame(&mut self, audio: &mut Option<AudioOutput>) {
        if self.payload.is_none() {
            return;
        }

        // Feed the running console the inputs the replay schedules for this
        // frame boundary, before it steps.
        if let (Some(replay), Some(payload)) = (&mut self.replay, &mut self.payload) {
            replay.apply_inputs(payload);
        }

        let (outcome, breakpoint_hit) = match &mut self.payload {
            Some(payload) => payload.step_frame(),
            None => return,
        };
        let new_frame = outcome.display.is_some();

        // Record or replay-check the produced frame before it moves into the
        // publish slot.
        if let Some((recorder, _)) = &mut self.recorder {
            recorder.note_frame(outcome.display.as_ref());
        }
        if let Some(step) = self
            .replay
            .as_mut()
            .map(|replay| replay.note_frame(outcome.display.as_ref()))
        {
            match step {
                ReplayStep::Continue => {}
                ReplayStep::Finished => {
                    self.replay = None;
                    self.notice("Replay finished");
                }
                ReplayStep::Diverged {
                    frame,
                    expected,
                    actual,
                } => {
                    self.replay = None;
                    let _ = self.events.unbounded_send(EmuEvent::ReplayDiverged {
                        frame,
                        expected,
                        actual,
                    });
                }
            }
        }

        // A save or recording-start deferred to an instruction boundary retries
        // now that this frame stepped the console.
        self.drive_pending_state();

        if let Some(display) = outcome.display
            && let Ok(mut slot) = self.frames.lock()
        {
            *slot = Some(display);
        }

        let Some(payload) = &mut self.payload else {
            return;
        };
        if let Ok(mut slot) = self.status.lock() {
            *slot = payload.running_status();
        }
        // Publish the inspection snapshot for the running debugger panes. Only
        // debugger payloads produce one; the copy is skipped for the console.
        if new_frame
            && let Some(view) = payload.debug_view()
            && let Ok(mut slot) = self.snapshot.lock()
        {
            *slot = Some(view);
        }
        // Peek each open memory viewer's view span at this vblank boundary, so
        // the running panes show the bytes as of the frame they publish.
        if new_frame
            && !self.memory_interest.is_empty()
            && let Ok(mut slot) = self.memory_window.lock()
        {
            *slot = Some(
                self.memory_interest
                    .iter()
                    .filter_map(|&interest| payload.peek_window(interest))
                    .collect(),
            );
        }
        if new_frame {
            let _ = self.events.unbounded_send(EmuEvent::FrameReady);
        }
        if let Some(audio) = audio {
            audio.push_samples(&payload.drain_audio_samples(), payload.audio_coupling());
        }

        // Debounce SRAM: reset countdown on a dirty frame, flush after quiet.
        if outcome.sram_dirty {
            self.sram_countdown = Some(0);
        } else if let Some(count) = &mut self.sram_countdown {
            *count += 1;
            if *count >= SRAM_DEBOUNCE_FRAMES {
                self.flush_sram();
            }
        }

        if breakpoint_hit {
            self.return_payload();
            let _ = self.events.unbounded_send(EmuEvent::BreakpointHit);
        }
    }

    fn flush_sram(&mut self) {
        self.sram_countdown = None;
        if let Some(payload) = &self.payload
            && let Some(ram) = payload.sram()
        {
            let _ = self.events.unbounded_send(EmuEvent::SramDirty(ram));
        }
    }

    fn frame_interval(&self) -> Duration {
        match &self.payload {
            Some(Payload::Console(console)) => console.frame_interval(),
            Some(Payload::Debugger(payload)) => payload.core.frame_interval(),
            None => Duration::ZERO,
        }
    }

    /// Fixed-timestep pacing against a wall clock: sleep when ahead, drop the
    /// backlog when it exceeds the deficit cap.
    fn pace(&mut self, interval: Duration) {
        self.next_deadline += interval;
        let now = Instant::now();
        if now < self.next_deadline {
            std::thread::sleep(self.next_deadline - now);
        } else if now - self.next_deadline > interval * MAX_DEFICIT_FRAMES {
            self.next_deadline = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_loop() -> EmuLoop {
        test_loop_with_events().0
    }

    fn test_loop_with_events() -> (
        EmuLoop,
        iced::futures::channel::mpsc::UnboundedReceiver<EmuEvent>,
    ) {
        let slots = PublishSlots {
            frames: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(None)),
            snapshot: Arc::new(Mutex::new(None)),
            memory_window: Arc::new(Mutex::new(None)),
        };
        let (events, events_rx) = iced::futures::channel::mpsc::unbounded();
        let (returns, _returns_rx) = channel();
        let (ack, _ack_rx) = channel();
        (EmuLoop::new(slots, events, returns, ack), events_rx)
    }

    fn drain(rx: &mut iced::futures::channel::mpsc::UnboundedReceiver<EmuEvent>) -> Vec<EmuEvent> {
        let mut out = Vec::new();
        while let Ok(Some(event)) = rx.try_next() {
            out.push(event);
        }
        out
    }

    fn recording_with(
        checks: Vec<missingno_core::recording::FrameCheck>,
        frames: u64,
    ) -> Recording {
        Recording {
            initial_state: Vec::new(),
            inputs: Vec::new(),
            checks,
            frames,
            check_interval: 0,
        }
    }

    // The audio scope pane drives this through the handle; here it exercises
    // the stored-flag path the emu loop re-applies to each payload it runs.
    #[test]
    fn set_wave_capture_stores_the_flag() {
        let mut state = test_loop();
        assert!(!state.wave_capture);
        state.handle(EmuCommand::SetWaveCapture(true));
        assert!(state.wave_capture);
        state.handle(EmuCommand::SetWaveCapture(false));
        assert!(!state.wave_capture);
    }

    // A graphics pane drives this through the handle; here it exercises the
    // stored-flag path the emu loop re-applies to each payload it runs.
    #[test]
    fn set_graphics_capture_stores_the_flag() {
        let mut state = test_loop();
        assert!(!state.graphics_capture);
        state.handle(EmuCommand::SetGraphicsCapture(true));
        assert!(state.graphics_capture);
        state.handle(EmuCommand::SetGraphicsCapture(false));
        assert!(!state.graphics_capture);
    }

    // A disagreeing checkpoint stops playback at the divergent frame and reports
    // the expected/actual hashes rather than continuing on a stale timeline.
    #[test]
    fn replay_stops_and_reports_a_divergent_checkpoint() {
        use missingno_core::recording::FrameCheck;
        let mut replay = ReplayPlayback::new(recording_with(
            vec![FrameCheck {
                frame: 0,
                hash: 0x1234,
            }],
            3,
        ));
        // A step that produced no frame hashes to 0, disagreeing with 0x1234.
        match replay.note_frame(None) {
            ReplayStep::Diverged {
                frame,
                expected,
                actual,
            } => {
                assert_eq!(frame, 0);
                assert_eq!(expected, 0x1234);
                assert_eq!(actual, 0);
            }
            _ => panic!("expected a divergence"),
        }
    }

    // A matching checkpoint continues; the run ends after the recorded count.
    #[test]
    fn replay_runs_to_the_recorded_frame_count() {
        use missingno_core::recording::FrameCheck;
        // A None frame hashes to 0; a checkpoint of 0 agrees.
        let mut replay =
            ReplayPlayback::new(recording_with(vec![FrameCheck { frame: 0, hash: 0 }], 2));
        assert!(matches!(replay.note_frame(None), ReplayStep::Continue));
        assert!(matches!(replay.note_frame(None), ReplayStep::Finished));
    }

    // Starting a recording during a replay would fold the replay's applied
    // inputs into the capture; the loop refuses and leaves no recorder.
    #[test]
    fn recording_is_refused_during_replay() {
        let (mut state, mut rx) = test_loop_with_events();
        state.replay = Some(ReplayPlayback::new(recording_with(Vec::new(), 2)));
        state.handle(EmuCommand::StartRecording("unused.mprc".into()));
        assert!(state.recorder.is_none());
        assert!(
            matches!(drain(&mut rx).as_slice(), [EmuEvent::Notice(_)]),
            "a refusal surfaces a notice, not a RecordingChanged"
        );
    }

    // Finalizing with nothing recording is a silent no-op — no stray event.
    #[test]
    fn finalize_without_a_recording_is_silent() {
        let (mut state, mut rx) = test_loop_with_events();
        state.handle(EmuCommand::StopRecording);
        assert!(drain(&mut rx).is_empty());
    }
}

impl Payload {
    fn reset(&mut self) {
        match self {
            Self::Console(console) => console.reset(),
            Self::Debugger(payload) => {
                payload.frame = 0;
                payload.core.reset();
            }
        }
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        match self {
            Self::Console(console) => console.set_control(control, input),
            Self::Debugger(payload) => payload.core.set_control(control, input),
        }
    }

    /// Serialize the running machine state. `None` when the system has no
    /// backend, or the console is momentarily off an instruction boundary;
    /// [`has_state_backend`](Self::has_state_backend) tells the two apart.
    fn save_state_bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Console(console) => console.save_state(),
            Self::Debugger(payload) => payload.core.save_state(),
        }
    }

    /// Whether the system has a save-state backend at all — it authors a state
    /// schema. A `None` from [`save_state_bytes`](Self::save_state_bytes) on a
    /// backend-having console is a transient off-boundary miss, not "no backend".
    fn has_state_backend(&self) -> bool {
        match self {
            Self::Console(console) => console.state_schema().is_some(),
            Self::Debugger(payload) => payload.core.state_schema().is_some(),
        }
    }

    /// Restore the running machine state from a save file.
    fn load_state(&mut self, path: &std::path::Path) -> Result<(), String> {
        let bytes =
            std::fs::read(path).map_err(|error| format!("could not read save state: {error}"))?;
        match self {
            Self::Console(console) => console.load_state(&bytes),
            Self::Debugger(payload) => payload.core.load_state(&bytes),
        }
        .map_err(|error| error.to_string())
    }

    fn set_breakpoint(&mut self, address: u32) {
        match self {
            Self::Console(_) => {}
            Self::Debugger(payload) => payload.core.set_breakpoint(address),
        }
    }

    fn clear_breakpoint(&mut self, address: u32) {
        match self {
            Self::Console(_) => {}
            Self::Debugger(payload) => payload.core.clear_breakpoint(address),
        }
    }

    fn add_watch(&mut self, watch: Watch) {
        match self {
            Self::Console(_) => {}
            Self::Debugger(payload) => payload.core.add_watch(watch),
        }
    }

    fn remove_watch(&mut self, watch: &Watch) {
        match self {
            Self::Console(_) => {}
            Self::Debugger(payload) => payload.core.remove_watch(watch),
        }
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match self {
            Self::Console(console) => console.drain_audio_samples(),
            Self::Debugger(payload) => payload.core.drain_audio_samples(),
        }
    }

    /// Toggle waveform capture. Only the debugger core captures; a plain
    /// console has no debugger surface, so the toggle is a no-op there.
    fn set_wave_capture(&mut self, on: bool) {
        if let Self::Debugger(payload) = self {
            payload.core.set_wave_capture(on);
        }
    }

    /// Toggle graphics-surface capture. Only the debugger core captures; a
    /// plain console has no debugger surface, so the toggle is a no-op there.
    fn set_graphics_capture(&mut self, on: bool) {
        if let Self::Debugger(payload) = self {
            payload.core.set_graphics_capture(on);
        }
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        match self {
            Self::Console(console) => console.audio_coupling(),
            Self::Debugger(payload) => payload.core.audio_coupling(),
        }
    }

    fn screen_display(&self) -> Frame {
        match self {
            Self::Console(console) => console.screen_display(),
            Self::Debugger(payload) => payload.core.screen_display(),
        }
    }

    fn sram(&self) -> Option<Vec<u8>> {
        match self {
            Self::Console(console) => console.battery_save(),
            Self::Debugger(payload) => payload.core.battery_save(),
        }
    }

    fn running_status(&self) -> Option<RunningStatus> {
        match self {
            Self::Console(_) => None,
            Self::Debugger(payload) => Some(payload.core.running_status(payload.frame)),
        }
    }

    fn debug_view(&self) -> Option<DebugView> {
        match self {
            Self::Console(_) => None,
            Self::Debugger(payload) => Some(payload.core.snapshot(payload.frame)),
        }
    }

    /// Peek the memory viewer's interest span through the seam. `None` for a
    /// plain console (no debugger, nothing to inspect).
    fn peek_window(&self, interest: MemoryInterest) -> Option<MemoryWindow> {
        match self {
            Self::Console(_) => None,
            Self::Debugger(payload) => Some(interest.read_through(payload.core.as_ref())),
        }
    }

    /// Emulate up to one frame; the flag reports a breakpoint stop. The
    /// console path never stops early; the debugger's frame step honours
    /// breakpoints.
    fn step_frame(&mut self) -> (FrameOutcome, bool) {
        match self {
            Self::Console(console) => (console.step_frame(), false),
            Self::Debugger(payload) => {
                payload.frame += 1;
                let outcome = payload.core.step_frame();
                let stopped = matches!(
                    outcome,
                    StepOutcome::Breakpoint { .. } | StepOutcome::WatchHit(_)
                );
                (
                    FrameOutcome {
                        display: outcome.into_frame(),
                        sram_dirty: false,
                    },
                    stopped,
                )
            }
        }
    }
}
