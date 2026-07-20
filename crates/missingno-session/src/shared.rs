//! The shared session component: one owner of the emulated machine, living on a
//! dedicated thread, that every consumer drives as a client.
//!
//! [`SharedSession`] owns the machine permanently — either a debugger-hosting
//! [`Session`] (breakpoints, inspection, disassembly) or a plain
//! [`SystemConsole`] on the console fast path. [`SessionHandle`] is the
//! cloneable client through which all access flows — commands and readouts alike
//! travel the request channel as blocking request/response, so commands from any
//! client serialize in arrival order. The handle also carries the run loop's
//! latest-value publish slots (frame, running status, per-vblank snapshot, memory
//! windows), read directly while the machine free-runs, and a subscriber channel
//! ([`SessionEvent`]) carrying the events a client cannot poll for (stops, SRAM,
//! recording/replay lifecycle, notices, and per-frame redraw ticks).
//!
//! Access is uniform for a debugger session: [`SessionHandle::with_session`]
//! hands a closure to the session thread to run against the owned [`Session`],
//! and the HTTP and MCP transports are nothing more than clients that route each
//! request through it. A console-only session hosts no [`Session`]; it answers
//! only the loop, control, state, recording and screenshot commands, and drops a
//! `with_session` job unanswered (`is_debugger` reports which kind it is).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use missingno_core::HighPass;
use missingno_core::inspect::MemoryWindow;
use missingno_core::recording::{FrameCheck, InputRecord, Recording, frame_hash};
use missingno_core::system::{
    ControlId, ControlInput, DebugView, RunningStatus, SystemConsole, SystemDebugger,
};
use missingno_core::video::Frame;

use crate::session::{Session, StopReason};

/// How far behind schedule the run loop lets itself fall before dropping the
/// deficit — degrades to slow-but-steady instead of spiralling.
const MAX_DEFICIT_FRAMES: u32 = 4;

/// The idle poll interval while paused, so a dropped request channel is noticed
/// promptly.
const IDLE_POLL: Duration = Duration::from_millis(200);

/// The frame cadence at which a recording checkpoints a frame hash.
const RECORDING_CHECK_INTERVAL: u64 = 300;

/// Frames of quiet before a debounced SRAM save is emitted. A game writes SRAM
/// across several consecutive frames during a save; we wait for writes to stop.
const SRAM_DEBOUNCE_FRAMES: u32 = 30;

/// A per-frame audio drain point a frontend attaches to consume the free-run
/// samples; `None` (the headless default) drains and drops them.
pub type AudioSink = Box<dyn FnMut(Vec<(f32, f32)>, Option<HighPass>) + Send>;

/// An event pushed from the session thread to a client that subscribed. Carries
/// what a client cannot learn by polling the slots: run-loop stops, debounced
/// SRAM, recording/replay lifecycle, notices, and the per-frame redraw tick.
#[derive(Clone, Debug)]
pub enum SessionEvent {
    /// A new frame is in the frame slot — a client's redraw hint.
    FrameReady,
    /// The free-run loop stopped on a breakpoint or watch; a client reads the
    /// paused surfaces and persists any battery save.
    Stopped,
    /// Debounced battery-backed RAM contents, ready to persist (console mode).
    SramDirty(Vec<u8>),
    /// Input-recording capture turned on or off. A client mirrors this into its
    /// recording flag — the flag follows the event, never the request.
    RecordingChanged(bool),
    /// A replay checkpoint disagreed with the recorded timeline; playback stopped
    /// at this frame.
    ReplayDiverged {
        frame: u64,
        expected: u64,
        actual: u64,
    },
    /// A user-facing status line (save/load result, recording note, replay note).
    Notice(String),
}

/// The subscriber senders an emitted event fans out to. Dead senders (a client
/// dropped its receiver) are pruned on the next emit.
type Subscribers = Arc<Mutex<Vec<Sender<SessionEvent>>>>;

/// A span the running panes want peeked each vblank into the memory-window slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryInterest {
    pub start: u32,
    pub len: u32,
}

/// The largest span a single interest peeks, so a bad length cannot allocate
/// unbounded.
const MAX_INTEREST_LEN: u32 = 0x1000;

impl MemoryInterest {
    fn read_through(self, session: &Session) -> MemoryWindow {
        let len = self.len.min(MAX_INTEREST_LEN);
        let bytes = (0..len)
            .map(|i| session.peek(self.start.wrapping_add(i)))
            .collect();
        MemoryWindow {
            base: self.start,
            bytes,
        }
    }
}

/// Returned when a readout cannot be answered honestly while the machine
/// free-runs: the published snapshot does not cover it, and touching the live
/// core mid-run is not allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningReadout;

impl std::fmt::Display for RunningReadout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("readout unavailable while the session is running; pause first")
    }
}

impl std::error::Error for RunningReadout {}

/// The latest-value publish slots the run loop writes each frame, shared by the
/// engine and every handle.
#[derive(Clone)]
struct Slots {
    frame: Arc<Mutex<Option<Frame>>>,
    status: Arc<Mutex<Option<RunningStatus>>>,
    snapshot: Arc<Mutex<Option<DebugView>>>,
    memory_windows: Arc<Mutex<Vec<MemoryWindow>>>,
    running: Arc<AtomicBool>,
    recording: Arc<AtomicBool>,
    subscribers: Subscribers,
}

impl Slots {
    fn new() -> Self {
        Slots {
            frame: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(None)),
            snapshot: Arc::new(Mutex::new(None)),
            memory_windows: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            recording: Arc::new(AtomicBool::new(false)),
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn emit(&self, event: SessionEvent) {
        // The pollable flag is set from the announcement itself, so it cannot
        // drift from what subscribers were told.
        if let SessionEvent::RecordingChanged(active) = event {
            self.recording.store(active, Ordering::SeqCst);
        }
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|tx| tx.send(event.clone()).is_ok());
        }
    }
}

/// A closure run against the owned [`Session`] at a command-drain boundary.
type Job = Box<dyn FnOnce(&mut Session) + Send>;

/// A request from a client to the session thread. Every debugger readout and
/// Session-method command rides as a [`Job`]; the loop, control, state,
/// recording, replay and screenshot commands the engine must observe get their
/// own variants.
enum Request {
    Job(Job),
    Run(Sender<()>),
    Pause(Sender<()>),
    Reset,
    SetControl(ControlId, ControlInput),
    SetMemoryInterest(Vec<MemoryInterest>),
    SaveState(PathBuf, Sender<Result<(), String>>),
    LoadState(PathBuf, Sender<Result<(), String>>),
    StartRecording(PathBuf, Sender<Result<(), String>>),
    StopRecording(Sender<Result<(), String>>),
    PlayRecording(PathBuf, Sender<Result<(), String>>),
    Screenshot(Sender<Frame>),
    BatterySave(Sender<Option<Vec<u8>>>),
    /// Finalize any recording and hand the owned machine back, then exit — the
    /// path a frontend takes to re-host the same live console in a session of the
    /// other kind (a debugger↔emulator toggle).
    Extract(Sender<ExtractedMachine>),
    Shutdown(Sender<()>),
}

/// The machine handed back by [`SharedSession::into_machine`]: the plain console,
/// or the debugger the session hosted, whichever kind it was.
pub enum ExtractedMachine {
    Console(Box<dyn SystemConsole>),
    Debugger(Box<dyn SystemDebugger>),
}

/// The cloneable client handle. All access to the machine flows through it.
#[derive(Clone)]
pub struct SessionHandle {
    requests: Sender<Request>,
    slots: Slots,
    is_debugger: bool,
}

impl SessionHandle {
    /// Whether this session hosts a debugger (inspection, breakpoints,
    /// [`with_session`](Self::with_session)) rather than a plain console.
    pub fn is_debugger(&self) -> bool {
        self.is_debugger
    }

    /// Run `f` against the owned [`Session`] on the session thread and block for
    /// its result. This is the universal debugger readout/command path: while
    /// paused the closure sees the live core immediately, and while running it
    /// runs at the next frame boundary — never mid-frame. On a console-only
    /// session ([`is_debugger`](Self::is_debugger) is false) the job is dropped
    /// unanswered, so callers must gate on the session kind.
    pub fn with_session<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut Session) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = channel();
        let job: Job = Box::new(move |session| {
            let _ = tx.send(f(session));
        });
        self.requests
            .send(Request::Job(job))
            .expect("session thread alive");
        rx.recv().expect("session thread answered")
    }

    /// Subscribe to the session's event stream. Each subscriber gets its own
    /// receiver; dropping it unsubscribes on the next emit.
    pub fn subscribe(&self) -> Receiver<SessionEvent> {
        let (tx, rx) = channel();
        if let Ok(mut subscribers) = self.slots.subscribers.lock() {
            subscribers.push(tx);
        }
        rx
    }

    /// Start free-running: the loop paces frame stepping and publishes the slots
    /// until [`pause`](Self::pause) or a breakpoint/watch stop. Blocks until the
    /// loop has started, so [`is_running`](Self::is_running) is settled on
    /// return — symmetric with [`pause`](Self::pause).
    pub fn run(&self) {
        let (tx, rx) = channel();
        if self.requests.send(Request::Run(tx)).is_ok() {
            let _ = rx.recv();
        }
    }

    /// Stop free-running and block until the loop has halted, so a following
    /// readout sees the paused core.
    pub fn pause(&self) {
        let (tx, rx) = channel();
        if self.requests.send(Request::Pause(tx)).is_ok() {
            let _ = rx.recv();
        }
    }

    /// Whether the machine is free-running right now.
    pub fn is_running(&self) -> bool {
        self.slots.running.load(Ordering::SeqCst)
    }

    /// Whether an input recording is being captured right now. Tracks the same
    /// truth [`SessionEvent::RecordingChanged`] announces, for a client that
    /// polls rather than subscribes.
    pub fn is_recording(&self) -> bool {
        self.slots.recording.load(Ordering::SeqCst)
    }

    /// Reset the machine, finalizing any recording and dropping any replay.
    pub fn reset(&self) {
        let _ = self.requests.send(Request::Reset);
    }

    /// Drive a console control. Applied in arrival order; noted into an active
    /// recording so a recorded input lands at the frame boundary it happened on.
    pub fn set_control(&self, control: ControlId, input: ControlInput) {
        let _ = self.requests.send(Request::SetControl(control, input));
    }

    /// Set the spans the run loop peeks into the memory-window slot each vblank.
    pub fn set_memory_interest(&self, interest: Vec<MemoryInterest>) {
        let _ = self.requests.send(Request::SetMemoryInterest(interest));
    }

    /// Save the machine state to `path`. A request that misses an instruction
    /// boundary waits for the next frame while running, and errors while paused
    /// — no frame is coming to reach a boundary on. The outcome also reaches
    /// every client as a [`SessionEvent::Notice`].
    pub fn save_state(&self, path: PathBuf) -> Result<(), String> {
        self.round_trip(|ack| Request::SaveState(path, ack))
    }

    /// Load the machine state from `path`, finalizing any recording and dropping
    /// any replay. The outcome also reaches every client as a
    /// [`SessionEvent::Notice`].
    pub fn load_state(&self, path: PathBuf) -> Result<(), String> {
        self.round_trip(|ack| Request::LoadState(path, ack))
    }

    /// Begin capturing an input recording to `path`, finalized by
    /// [`stop_recording`](Self::stop_recording). Errors when the system has no
    /// save-state backend; a request off an instruction boundary defers and
    /// starts at the next frame (reported by [`SessionEvent::RecordingChanged`]).
    pub fn start_recording(&self, path: PathBuf) -> Result<(), String> {
        self.round_trip(|tx| Request::StartRecording(path, tx))
    }

    /// Finish and write the active recording. A no-op when nothing is recording.
    pub fn stop_recording(&self) -> Result<(), String> {
        self.round_trip(Request::StopRecording)
    }

    /// Load a recording and replay it, driving the machine frame by frame so the
    /// playback is watchable. Errors on an unreadable/corrupt file, a state the
    /// machine cannot restore, or an active capture; a divergence stops playback
    /// and arrives as [`SessionEvent::ReplayDiverged`].
    pub fn play_recording(&self, path: PathBuf) -> Result<(), String> {
        self.round_trip(|tx| Request::PlayRecording(path, tx))
    }

    /// The current display frame, captured at the next frame boundary while
    /// running or immediately while paused.
    pub fn screenshot(&self) -> Option<Frame> {
        let (tx, rx) = channel();
        self.requests.send(Request::Screenshot(tx)).ok()?;
        rx.recv().ok()
    }

    fn round_trip(
        &self,
        build: impl FnOnce(Sender<Result<(), String>>) -> Request,
    ) -> Result<(), String> {
        let (tx, rx) = channel();
        self.requests
            .send(build(tx))
            .map_err(|_| "session thread gone".to_string())?;
        rx.recv().map_err(|_| "session thread gone".to_string())?
    }

    /// Take the latest published frame, or `None` when none is pending. A
    /// frontend resolves colour at draw time (palette, SGB, index-frame detection
    /// are its policy), so the seam carries the pre-resolution [`Frame`]; it is a
    /// take, not a clone, since `Frame` is a large move-only surface.
    pub fn latest_frame(&self) -> Option<Frame> {
        self.slots
            .frame
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// The latest published running status.
    pub fn latest_status(&self) -> Option<RunningStatus> {
        self.slots.status.lock().ok().and_then(|slot| slot.clone())
    }

    /// Read the latest published inspection snapshot through `f` (the snapshot is
    /// not clonable, so it is borrowed in place). `f` sees `None` before the
    /// first free-run frame publishes one.
    pub fn with_snapshot<R>(&self, f: impl FnOnce(Option<&DebugView>) -> R) -> R {
        match self.slots.snapshot.lock() {
            Ok(slot) => f(slot.as_ref()),
            Err(_) => f(None),
        }
    }

    /// Take the latest published inspection snapshot, or `None` when none is
    /// pending. A take, like [`latest_frame`](Self::latest_frame): the frontend
    /// stores the owned snapshot and the run loop republishes one each frame.
    pub fn take_snapshot(&self) -> Option<DebugView> {
        self.slots
            .snapshot
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// The battery-backed RAM to persist, captured at the next command boundary.
    /// Works for both machine kinds, so a plain-console client can persist on a
    /// pause without a debugger surface. `None` when the cart has no battery.
    pub fn battery_save(&self) -> Option<Vec<u8>> {
        let (tx, rx) = channel();
        self.requests.send(Request::BatterySave(tx)).ok()?;
        rx.recv().ok().flatten()
    }

    /// The latest published memory windows (one per set interest).
    pub fn latest_memory_windows(&self) -> Vec<MemoryWindow> {
        self.slots
            .memory_windows
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    /// Read `len` bytes honestly across the run boundary: from the live core
    /// while paused, from the published memory windows while running, and
    /// [`RunningReadout`] when running with no window covering the span.
    pub fn read_memory(&self, address: u32, len: u32) -> Result<Vec<u8>, RunningReadout> {
        if !self.is_running() && self.is_debugger {
            return Ok(self.with_session(move |session| session.memory(address, len)));
        }
        let windows = self.latest_memory_windows();
        (0..len)
            .map(|i| {
                let at = address.wrapping_add(i);
                windows
                    .iter()
                    .find_map(|window| window.read(at))
                    .ok_or(RunningReadout)
            })
            .collect()
    }
}

/// The owning component: it holds the session thread and hands out clients.
/// Dropping it shuts the thread down (finalizing any recording first).
pub struct SharedSession {
    handle: SessionHandle,
    thread: Option<JoinHandle<()>>,
}

impl SharedSession {
    /// Spawn a session thread owning `debugger`, with no audio sink.
    pub fn spawn(debugger: Box<dyn SystemDebugger>) -> Self {
        Self::spawn_with_audio(debugger, None)
    }

    /// Spawn a session thread owning `debugger`, draining each free-run frame's
    /// audio into `sink` when one is attached.
    pub fn spawn_with_audio(debugger: Box<dyn SystemDebugger>, sink: Option<AudioSink>) -> Self {
        Self::spawn_machine(Machine::Debugger(Session::new(debugger)), sink, true)
    }

    /// Spawn a session thread owning a plain `console` on the fast path, with no
    /// audio sink. The console mode answers the loop, control, state, recording,
    /// replay and screenshot commands but has no debugger inspection surface.
    pub fn spawn_console(console: Box<dyn SystemConsole>) -> Self {
        Self::spawn_console_with_audio(console, None)
    }

    /// Spawn a console-only session, draining each free-run frame's audio into
    /// `sink` when one is attached.
    pub fn spawn_console_with_audio(
        console: Box<dyn SystemConsole>,
        sink: Option<AudioSink>,
    ) -> Self {
        Self::spawn_machine(Machine::Console(console), sink, false)
    }

    fn spawn_machine(machine: Machine, sink: Option<AudioSink>, is_debugger: bool) -> Self {
        let slots = Slots::new();
        let (requests_tx, requests_rx) = channel();
        let handle = SessionHandle {
            requests: requests_tx,
            slots: slots.clone(),
            is_debugger,
        };
        let engine_slots = slots;
        let thread = std::thread::Builder::new()
            .name("session".into())
            .spawn(move || {
                SessionEngine::new(machine, engine_slots, sink).serve(requests_rx);
            })
            .expect("spawn session thread");
        SharedSession {
            handle,
            thread: Some(thread),
        }
    }

    /// A fresh client handle onto this session.
    pub fn handle(&self) -> SessionHandle {
        self.handle.clone()
    }

    /// Consume the session and hand back the owned machine — the console, or the
    /// debugger it hosted — finalizing any recording first. `None` only when the
    /// thread has already gone. The frontend re-hosts the returned machine in a
    /// session of the other kind to toggle the debugger while keeping the live
    /// console (its serial link, printer, and cartridge state) intact.
    pub fn into_machine(mut self) -> Option<ExtractedMachine> {
        let (tx, rx) = channel();
        self.handle.requests.send(Request::Extract(tx)).ok()?;
        let extracted = rx.recv().ok();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        extracted
    }
}

impl From<Session> for SharedSession {
    /// Promote a bare session to a shared one by moving its debugger onto a
    /// session thread — the path a caller holding a plain [`Session`] takes to
    /// serve it through a client transport.
    fn from(session: Session) -> Self {
        SharedSession::spawn(session.into_debugger())
    }
}

impl Drop for SharedSession {
    fn drop(&mut self) {
        let (tx, rx) = channel();
        if self.handle.requests.send(Request::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The machine the session owns: a debugger-hosting [`Session`], or a plain
/// console on the fast path. The console path never pays the debugger wrapper's
/// per-step cost; it simply has no inspection surface.
enum Machine {
    Debugger(Session),
    Console(Box<dyn SystemConsole>),
}

/// One stepped frame's outcome, uniform across the two machine kinds.
struct MachineStep {
    display: Option<Frame>,
    sram_dirty: bool,
    stopped: bool,
}

impl Machine {
    fn step(&mut self) -> MachineStep {
        match self {
            Machine::Debugger(session) => {
                let (reason, display) = session.advance_frame();
                MachineStep {
                    display,
                    sram_dirty: false,
                    stopped: matches!(reason, StopReason::Breakpoint | StopReason::Watch(_)),
                }
            }
            Machine::Console(console) => {
                let outcome = console.step_frame();
                MachineStep {
                    display: outcome.display,
                    sram_dirty: outcome.sram_dirty,
                    stopped: false,
                }
            }
        }
    }

    /// The machine as a plain console — every debugger is one, so the
    /// non-inspection surface needs no per-kind arm.
    fn console(&self) -> &dyn SystemConsole {
        match self {
            Machine::Debugger(session) => session.console(),
            Machine::Console(console) => console.as_ref(),
        }
    }

    fn console_mut(&mut self) -> &mut dyn SystemConsole {
        match self {
            Machine::Debugger(session) => session.console_mut(),
            Machine::Console(console) => console.as_mut(),
        }
    }

    /// Power-cycle, clearing the hosting session's run bookkeeping with it.
    fn reset(&mut self) {
        match self {
            Machine::Debugger(session) => session.reset(),
            Machine::Console(console) => console.reset(),
        }
    }

    fn running_status(&self) -> Option<RunningStatus> {
        match self {
            Machine::Debugger(session) => Some(session.running_status()),
            Machine::Console(_) => None,
        }
    }

    fn snapshot(&self) -> Option<DebugView> {
        match self {
            Machine::Debugger(session) => Some(session.snapshot()),
            Machine::Console(_) => None,
        }
    }

    fn into_extracted(self) -> ExtractedMachine {
        match self {
            Machine::Console(console) => ExtractedMachine::Console(console),
            Machine::Debugger(session) => ExtractedMachine::Debugger(session.into_debugger()),
        }
    }
}

/// An input recording being captured from the owned machine as it steps frames.
struct Capture {
    initial_state: Vec<u8>,
    inputs: Vec<InputRecord>,
    checks: Vec<FrameCheck>,
    frame: u64,
    check_interval: u64,
    path: PathBuf,
}

impl Capture {
    fn note_input(&mut self, control: ControlId, input: ControlInput) {
        self.inputs.push(InputRecord {
            frame: self.frame,
            control,
            input,
        });
    }

    fn note_frame(&mut self, frame: Option<&Frame>) {
        if self.check_interval != 0 && self.frame.is_multiple_of(self.check_interval) {
            self.checks.push(FrameCheck {
                frame: self.frame,
                hash: frame.map(frame_hash).unwrap_or(0),
            });
        }
        self.frame += 1;
    }

    fn finish(mut self) -> Result<(), String> {
        // An input noted after the last frame stepped (a press while paused, or
        // between the final frame and the stop) is stamped on a frame replay
        // never reaches, so the timeline ends without it.
        self.inputs.retain(|input| input.frame < self.frame);
        let recording = Recording {
            initial_state: self.initial_state,
            inputs: self.inputs,
            checks: self.checks,
            frames: self.frame,
            check_interval: self.check_interval,
        };
        let bytes = recording.to_bytes().map_err(|error| error.to_string())?;
        std::fs::write(&self.path, bytes)
            .map_err(|error| format!("could not write {:?}: {error}", self.path))
    }
}

/// A recording played back through the running machine: applies the recorded
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

impl ReplayPlayback {
    fn new(recording: Recording) -> Self {
        Self {
            recording,
            frame: 0,
            input_cursor: 0,
            check_cursor: 0,
        }
    }

    fn apply_inputs(&mut self, machine: &mut Machine) {
        while let Some(event) = self.recording.inputs.get(self.input_cursor) {
            if event.frame != self.frame {
                break;
            }
            machine
                .console_mut()
                .set_control(event.control, event.input);
            self.input_cursor += 1;
        }
    }

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

/// The result of attempting to serialize the running state to a file.
enum SaveOutcome {
    Saved,
    /// The machine has a backend but is off an instruction boundary; retry once
    /// a frame steps it forward.
    Retry,
    NoBackend,
    Failed(String),
}

/// The session thread's engine: the owned [`Machine`] plus the run-loop state.
struct SessionEngine {
    machine: Machine,
    slots: Slots,
    running: bool,
    memory_interest: Vec<MemoryInterest>,
    capture: Option<Capture>,
    replay: Option<ReplayPlayback>,
    /// A save requested off an instruction boundary (a frame boundary need not be
    /// one): retried at each frame until the machine reaches a boundary, with the
    /// requester still waiting on its ack.
    pending_save: Option<(PathBuf, Sender<Result<(), String>>)>,
    /// A recording-start requested off an instruction boundary, retried likewise
    /// — its initial state is a save, so it has the same boundary requirement.
    pending_record: Option<PathBuf>,
    /// Frames of quiet since the last SRAM write, or `None` when settled.
    sram_countdown: Option<u32>,
    /// A pending machine-extraction reply, set by [`Request::Extract`] and
    /// answered by `finish_extract` once the loop releases the machine.
    extract: Option<Sender<ExtractedMachine>>,
    audio: Option<AudioSink>,
    next_deadline: Instant,
}

impl SessionEngine {
    fn new(machine: Machine, slots: Slots, audio: Option<AudioSink>) -> Self {
        SessionEngine {
            machine,
            slots,
            running: false,
            memory_interest: Vec::new(),
            capture: None,
            replay: None,
            pending_save: None,
            pending_record: None,
            sram_countdown: None,
            extract: None,
            audio,
            next_deadline: Instant::now(),
        }
    }

    fn emit(&self, event: SessionEvent) {
        self.slots.emit(event);
    }

    fn notice(&self, message: impl Into<String>) {
        self.emit(SessionEvent::Notice(message.into()));
    }

    /// The command/run loop: step paced frames while running (draining pending
    /// requests between whole frames), else block for the next request.
    fn serve(mut self, requests: Receiver<Request>) {
        loop {
            if self.running {
                loop {
                    match requests.try_recv() {
                        Ok(request) => {
                            if self.handle(request) {
                                return self.finish_extract();
                            }
                            if !self.running {
                                break;
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
                if self.running {
                    self.step_and_publish();
                    self.pace();
                }
            } else {
                match requests.recv_timeout(IDLE_POLL) {
                    Ok(request) => {
                        if self.handle(request) {
                            return self.finish_extract();
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    }

    /// Apply one request. Returns `true` when the thread should exit.
    fn handle(&mut self, request: Request) -> bool {
        match request {
            Request::Job(job) => {
                // A console-only session has no `Session` to run the job against;
                // dropping it lets the client's readout gate on `is_debugger`.
                if let Machine::Debugger(session) = &mut self.machine {
                    job(session);
                }
            }
            Request::Run(ack) => {
                self.start_running();
                let _ = ack.send(());
            }
            Request::Pause(ack) => {
                self.stop_running();
                let _ = ack.send(());
            }
            Request::Reset => {
                let _ = self.finish_recording();
                self.replay = None;
                self.cancel_pending_save("a reset replaced the state it would have captured");
                self.pending_record = None;
                self.machine.reset();
            }
            Request::SetControl(control, input) => {
                self.machine.console_mut().set_control(control, input);
                if let Some(capture) = &mut self.capture {
                    capture.note_input(control, input);
                }
            }
            Request::SetMemoryInterest(interest) => {
                self.memory_interest = interest;
                self.publish_memory_windows();
            }
            Request::SaveState(path, ack) => self.save_state(path, ack),
            Request::LoadState(path, ack) => self.load_state(path, ack),
            Request::StartRecording(path, ack) => {
                let _ = ack.send(self.begin_recording(path));
            }
            Request::StopRecording(ack) => {
                let _ = ack.send(self.finish_recording());
            }
            Request::PlayRecording(path, ack) => {
                let _ = ack.send(self.start_replay(path));
            }
            Request::Screenshot(ack) => {
                let _ = ack.send(self.machine.console().screen_display());
            }
            Request::BatterySave(ack) => {
                let _ = ack.send(self.machine.console().battery_save());
            }
            // Extract and Shutdown both exit the thread; Extract stashes the
            // reply channel so `finish_extract` can hand the machine back after
            // the loop releases it.
            Request::Extract(ack) => {
                self.extract = Some(ack);
                return true;
            }
            Request::Shutdown(ack) => {
                let _ = self.finish_recording();
                let _ = ack.send(());
                return true;
            }
        }
        false
    }

    /// The thread is exiting: if it was an [`Request::Extract`], finalize any
    /// recording and hand the owned machine back; a plain shutdown drops it.
    fn finish_extract(mut self) {
        if let Some(ack) = self.extract.take() {
            let _ = self.finish_recording();
            let SessionEngine { machine, .. } = self;
            let _ = ack.send(machine.into_extracted());
        }
    }

    fn start_running(&mut self) {
        self.running = true;
        self.slots.running.store(true, Ordering::SeqCst);
        self.next_deadline = Instant::now();
    }

    fn stop_running(&mut self) {
        self.running = false;
        self.slots.running.store(false, Ordering::SeqCst);
    }

    fn step_and_publish(&mut self) {
        // Feed the machine the inputs the replay schedules for this frame
        // boundary before it steps.
        if let Some(replay) = self.replay.as_mut() {
            replay.apply_inputs(&mut self.machine);
        }

        let step = self.machine.step();
        let display = step.display;

        if let Some(capture) = &mut self.capture {
            capture.note_frame(display.as_ref());
        }
        let replay_step = self
            .replay
            .as_mut()
            .map(|replay| replay.note_frame(display.as_ref()));
        if let Some(replay_step) = replay_step {
            match replay_step {
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
                    self.emit(SessionEvent::ReplayDiverged {
                        frame,
                        expected,
                        actual,
                    });
                }
            }
        }

        // A save or recording-start deferred to an instruction boundary retries
        // now that this frame stepped the machine.
        self.drive_pending_state();

        let produced = display.is_some();
        if let Some(frame) = display
            && let Ok(mut slot) = self.slots.frame.lock()
        {
            *slot = Some(frame);
        }
        if let Ok(mut slot) = self.slots.status.lock() {
            *slot = self.machine.running_status();
        }
        if produced {
            if let Some(view) = self.machine.snapshot()
                && let Ok(mut slot) = self.slots.snapshot.lock()
            {
                *slot = Some(view);
            }
            self.publish_memory_windows();
        }

        // Drain the frame's audio so the buffer can't grow unbounded; a frontend
        // sink consumes it, the headless default drops it.
        let samples = self.machine.console_mut().drain_audio_samples();
        if let Some(sink) = &mut self.audio {
            let coupling = self.machine.console().audio_coupling();
            sink(samples, coupling);
        }

        self.debounce_sram(step.sram_dirty);

        if produced {
            self.emit(SessionEvent::FrameReady);
        }
        if step.stopped {
            self.stop_running();
            self.emit(SessionEvent::Stopped);
        }
    }

    /// Emit the battery RAM once writes have been quiet for a debounce window.
    /// Only the console path marks frames dirty; the debugger path persists SRAM
    /// on a stop instead.
    fn debounce_sram(&mut self, dirty: bool) {
        if dirty {
            self.sram_countdown = Some(0);
        } else if let Some(count) = self.sram_countdown {
            let count = count + 1;
            if count >= SRAM_DEBOUNCE_FRAMES {
                self.sram_countdown = None;
                if let Some(ram) = self.machine.console().battery_save() {
                    self.emit(SessionEvent::SramDirty(ram));
                }
            } else {
                self.sram_countdown = Some(count);
            }
        }
    }

    fn publish_memory_windows(&self) {
        let windows: Vec<MemoryWindow> = match &self.machine {
            Machine::Debugger(session) => self
                .memory_interest
                .iter()
                .map(|interest| interest.read_through(session))
                .collect(),
            Machine::Console(_) => Vec::new(),
        };
        if let Ok(mut slot) = self.slots.memory_windows.lock() {
            *slot = windows;
        }
    }

    /// Fixed-timestep pacing against a wall clock: sleep when ahead, drop the
    /// backlog when it exceeds the deficit cap.
    fn pace(&mut self) {
        let interval = self.machine.console().frame_interval();
        self.next_deadline += interval;
        let now = Instant::now();
        if now < self.next_deadline {
            std::thread::sleep(self.next_deadline - now);
        } else if now - self.next_deadline > interval * MAX_DEFICIT_FRAMES {
            self.next_deadline = now;
        }
    }

    fn attempt_save(&self, path: &std::path::Path) -> SaveOutcome {
        match self.machine.console().save_state() {
            Some(bytes) => match std::fs::write(path, bytes) {
                Ok(()) => SaveOutcome::Saved,
                Err(error) => SaveOutcome::Failed(format!("could not write save state: {error}")),
            },
            None if self.machine.console().state_schema().is_some() => SaveOutcome::Retry,
            None => SaveOutcome::NoBackend,
        }
    }

    fn save_state(&mut self, path: PathBuf, ack: Sender<Result<(), String>>) {
        match self.attempt_save(&path) {
            SaveOutcome::Saved => self.settle(ack, Ok("State saved")),
            // Only a stepping machine reaches a boundary, so deferring while
            // paused would leave the requester waiting on a frame that never
            // comes.
            SaveOutcome::Retry if self.running => self.pending_save = Some((path, ack)),
            SaveOutcome::Retry => self.settle(
                ack,
                Err("the machine is between instructions; step it to a boundary first".to_string()),
            ),
            SaveOutcome::NoBackend => self.settle(
                ack,
                Err("this system has no save-state backend".to_string()),
            ),
            SaveOutcome::Failed(error) => self.settle(ack, Err(error)),
        }
    }

    fn load_state(&mut self, path: PathBuf, ack: Sender<Result<(), String>>) {
        // A load replaces the state a recording continues from and the state a
        // pending save would capture; finalize the recording and drop both.
        let _ = self.finish_recording();
        self.replay = None;
        self.cancel_pending_save("a state load replaced the state it would have captured");
        match std::fs::read(&path)
            .map_err(|error| format!("could not read save state: {error}"))
            .and_then(|bytes| {
                self.machine
                    .console_mut()
                    .load_state(&bytes)
                    .map_err(|error| error.to_string())
            }) {
            Ok(()) => self.settle(ack, Ok("State loaded")),
            Err(error) => self.settle(ack, Err(error)),
        }
    }

    /// Answer a state request on both channels a client may be listening on: the
    /// notice every client sees, and the ack the requester is blocked on.
    fn settle(&self, ack: Sender<Result<(), String>>, outcome: Result<&str, String>) {
        let (message, result) = match outcome {
            Ok(note) => (note.to_string(), Ok(())),
            Err(error) => (error.clone(), Err(error)),
        };
        self.notice(message);
        let _ = ack.send(result);
    }

    fn cancel_pending_save(&mut self, why: &str) {
        if let Some((_, ack)) = self.pending_save.take() {
            let _ = ack.send(Err(why.to_string()));
        }
    }

    /// Retry a save or recording-start that was requested off an instruction
    /// boundary, now that a frame has stepped the machine forward.
    fn drive_pending_state(&mut self) {
        // A still-off-boundary save re-defers itself through `save_state`.
        if let Some((path, ack)) = self.pending_save.take() {
            self.save_state(path, ack);
        }
        if let Some(path) = self.pending_record.take() {
            let _ = self.begin_recording(path);
        }
    }

    fn begin_recording(&mut self, path: PathBuf) -> Result<(), String> {
        // A replay's applied inputs bypass the capture, so recording one would
        // write a timeline its own inputs cannot reproduce.
        if self.replay.is_some() {
            return Err("stop the replay before recording".to_string());
        }
        // Finalize any recording already running before starting a fresh one so
        // its file is written rather than dropped.
        self.finish_recording()?;
        if self.machine.console().state_schema().is_none() {
            return Err("this system has no save-state backend".to_string());
        }
        match self.machine.console().save_state() {
            Some(initial_state) => {
                // Re-seat from the captured state so the recorded timeline is the
                // exact continuation replay reproduces.
                self.machine
                    .console_mut()
                    .load_state(&initial_state)
                    .map_err(|error| error.to_string())?;
                self.capture = Some(Capture {
                    initial_state,
                    inputs: Vec::new(),
                    checks: Vec::new(),
                    frame: 0,
                    check_interval: RECORDING_CHECK_INTERVAL,
                    path,
                });
                self.emit(SessionEvent::RecordingChanged(true));
                Ok(())
            }
            // Off an instruction boundary: its initial save missed. Defer and
            // retry at the next frame, matching the pending-save path.
            None => {
                self.pending_record = Some(path);
                Ok(())
            }
        }
    }

    fn finish_recording(&mut self) -> Result<(), String> {
        self.pending_record = None;
        match self.capture.take() {
            Some(capture) => {
                self.emit(SessionEvent::RecordingChanged(false));
                capture.finish()
            }
            None => Ok(()),
        }
    }

    fn start_replay(&mut self, path: PathBuf) -> Result<(), String> {
        // Replaying while recording would fold the replay's applied inputs (which
        // bypass the capture) into it; refuse.
        if self.capture.is_some() || self.pending_record.is_some() {
            return Err("stop recording before replaying".to_string());
        }
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        let recording = Recording::from_bytes(&bytes).map_err(|error| error.to_string())?;
        self.machine
            .console_mut()
            .load_state(&recording.initial_state)
            .map_err(|error| format!("replay could not restore: {error}"))?;
        self.replay = Some(ReplayPlayback::new(recording));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::recording::FrameCheck;

    fn recording_with(checks: Vec<FrameCheck>, frames: u64) -> Recording {
        Recording {
            initial_state: Vec::new(),
            inputs: Vec::new(),
            checks,
            frames,
            check_interval: 0,
        }
    }

    #[test]
    fn replay_reports_a_divergent_checkpoint() {
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
                assert_eq!((frame, expected, actual), (0, 0x1234, 0));
            }
            _ => panic!("expected a divergence"),
        }
    }

    #[test]
    fn replay_runs_to_the_recorded_frame_count() {
        // A None frame hashes to 0; a checkpoint of 0 agrees.
        let mut replay =
            ReplayPlayback::new(recording_with(vec![FrameCheck { frame: 0, hash: 0 }], 2));
        assert!(matches!(replay.note_frame(None), ReplayStep::Continue));
        assert!(matches!(replay.note_frame(None), ReplayStep::Finished));
    }

    #[test]
    fn replay_checkpoints_only_the_recorded_frames() {
        // A checkpoint on a later frame leaves earlier frames unchecked, so a
        // mismatching hash before it cannot diverge the replay.
        let mut replay = ReplayPlayback::new(recording_with(
            vec![FrameCheck {
                frame: 1,
                hash: 0x99,
            }],
            3,
        ));
        assert!(matches!(replay.note_frame(None), ReplayStep::Continue));
        assert!(matches!(
            replay.note_frame(None),
            ReplayStep::Diverged { frame: 1, .. }
        ));
    }
}
