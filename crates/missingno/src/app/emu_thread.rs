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

use missingno_core::inspect::Watch;

use super::audio_output::AudioOutput;
use super::debugger::inspect::DebugView;
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
    SetBreakpoint(u16),
    ClearBreakpoint(u16),
    AddWatchpoint(Watch),
    RemoveWatchpoint(Watch),
    RequestScreenshot {
        options: CaptureOptions,
    },
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
}

/// The UI-side handle to the emu thread. Cloneable so it can ride in a Message;
/// the return receiver is shared behind a mutex (single consumer in practice).
#[derive(Clone)]
pub struct EmuHandle {
    commands: Sender<EmuCommand>,
    frames: FrameSlot,
    status: StatusSlot,
    snapshot: SnapshotSlot,
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

    pub fn send(&self, command: EmuCommand) {
        let _ = self.commands.send(command);
    }

    /// Send a payload to the thread and start running it.
    pub fn run(&self, payload: Payload) {
        self.send(EmuCommand::Run(payload));
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
    let frames: FrameSlot = Arc::new(Mutex::new(None));
    let status: StatusSlot = Arc::new(Mutex::new(None));
    let snapshot: SnapshotSlot = Arc::new(Mutex::new(None));

    let handle = EmuHandle {
        commands: command_tx,
        frames: frames.clone(),
        status: status.clone(),
        snapshot: snapshot.clone(),
        returns: Arc::new(Mutex::new(return_rx)),
        shutdown_ack: Arc::new(Mutex::new(shutdown_ack_rx)),
    };
    let _ = event_tx.unbounded_send(EmuEvent::Started(handle));

    let worker_events = event_tx;
    std::thread::Builder::new()
        .name("emu".into())
        .spawn(move || {
            run_emu_thread(
                command_rx,
                return_tx,
                shutdown_ack_tx,
                frames,
                status,
                snapshot,
                worker_events,
            )
        })
        .expect("spawn emu thread");

    event_rx
}

type EventSink = iced::futures::channel::mpsc::UnboundedSender<EmuEvent>;

fn run_emu_thread(
    commands: Receiver<EmuCommand>,
    returns: Sender<Payload>,
    shutdown_ack: Sender<()>,
    frames: FrameSlot,
    status: StatusSlot,
    snapshot: SnapshotSlot,
    events: EventSink,
) {
    // Audio device lives on this thread (cpal's Stream is `!Send`).
    let mut audio = AudioOutput::new();
    let mut state = EmuLoop::new(frames, status, snapshot, events, returns, shutdown_ack);

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
    events: EventSink,
    returns: Sender<Payload>,
    shutdown_ack: Sender<()>,
    shutdown_requested: bool,
    sram_countdown: Option<u32>,
    next_deadline: Instant,
}

impl EmuLoop {
    fn new(
        frames: FrameSlot,
        status: StatusSlot,
        snapshot: SnapshotSlot,
        events: EventSink,
        returns: Sender<Payload>,
        shutdown_ack: Sender<()>,
    ) -> Self {
        Self {
            payload: None,
            frames,
            status,
            snapshot,
            events,
            returns,
            shutdown_ack,
            shutdown_requested: false,
            sram_countdown: None,
            next_deadline: Instant::now(),
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
            EmuCommand::Run(payload) => {
                self.payload = Some(payload);
                self.sram_countdown = None;
                self.next_deadline = Instant::now();
            }
            EmuCommand::Pause => self.return_payload(),
            EmuCommand::Shutdown => self.shutdown_requested = true,
            EmuCommand::Reset => {
                if let Some(payload) = &mut self.payload {
                    payload.reset();
                }
            }
            EmuCommand::SetControl(control, input) => {
                if let Some(payload) = &mut self.payload {
                    payload.set_control(control, input);
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
            EmuCommand::RequestScreenshot { options } => {
                if let Some(payload) = &self.payload {
                    let capture = FrameCapture::from_frame(&payload.screen_display(), &options);
                    let _ = self
                        .events
                        .unbounded_send(EmuEvent::Screenshot(Box::new(capture)));
                }
            }
        }
    }

    fn return_payload(&mut self) {
        if let Some(payload) = self.payload.take() {
            let _ = self.returns.send(payload);
        }
    }

    fn emulate_frame(&mut self, audio: &mut Option<AudioOutput>) {
        let Some(payload) = &mut self.payload else {
            return;
        };
        let (outcome, breakpoint_hit) = payload.step_frame();
        let new_frame = outcome.display.is_some();
        if let Some(display) = outcome.display
            && let Ok(mut slot) = self.frames.lock()
        {
            *slot = Some(display);
        }
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

    fn set_breakpoint(&mut self, address: u16) {
        match self {
            Self::Console(_) => {}
            Self::Debugger(payload) => payload.core.set_breakpoint(address as u32),
        }
    }

    fn clear_breakpoint(&mut self, address: u16) {
        match self {
            Self::Console(_) => {}
            Self::Debugger(payload) => payload.core.clear_breakpoint(address as u32),
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
