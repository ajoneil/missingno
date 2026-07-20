//! The shared session component: one owner of the boxed [`SystemDebugger`],
//! living on a dedicated thread, that every consumer drives as a client.
//!
//! [`SharedSession`] owns the machine permanently. [`SessionHandle`] is the
//! cloneable client through which all access flows — commands and readouts alike
//! travel the request channel as blocking request/response, so commands from any
//! client serialize in arrival order. The handle also carries the run loop's
//! latest-value publish slots (frame, running status, per-vblank snapshot, memory
//! windows), read directly while the machine free-runs.
//!
//! Access is uniform: [`SessionHandle::with_session`] hands a closure to the
//! session thread to run against the owned [`Session`], and the HTTP and MCP
//! transports are nothing more than clients that route each request through it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use missingno_core::HighPass;
use missingno_core::inspect::MemoryWindow;
use missingno_core::recording::{FrameCheck, InputRecord, Recording, frame_hash};
use missingno_core::system::{ControlId, ControlInput, DebugView, RunningStatus, SystemDebugger};
use missingno_core::video::{Frame, RgbaFrame};

use crate::session::{Session, StopReason};

/// How far behind schedule the run loop lets itself fall before dropping the
/// deficit — degrades to slow-but-steady instead of spiralling.
const MAX_DEFICIT_FRAMES: u32 = 4;

/// The idle poll interval while paused, so a dropped request channel is noticed
/// promptly.
const IDLE_POLL: Duration = Duration::from_millis(200);

/// The frame cadence at which a recording checkpoints a frame hash.
const RECORDING_CHECK_INTERVAL: u64 = 300;

/// A per-frame audio drain point a frontend attaches to consume the free-run
/// samples; `None` (the headless default) drains and drops them.
pub type AudioSink = Box<dyn FnMut(Vec<(f32, f32)>, Option<HighPass>) + Send>;

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
    frame: Arc<Mutex<Option<RgbaFrame>>>,
    status: Arc<Mutex<Option<RunningStatus>>>,
    snapshot: Arc<Mutex<Option<DebugView>>>,
    memory_windows: Arc<Mutex<Vec<MemoryWindow>>>,
    running: Arc<AtomicBool>,
}

impl Slots {
    fn new() -> Self {
        Slots {
            frame: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(None)),
            snapshot: Arc::new(Mutex::new(None)),
            memory_windows: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// A closure run against the owned [`Session`] at a command-drain boundary.
type Job = Box<dyn FnOnce(&mut Session) + Send>;

/// A request from a client to the session thread. Every readout and every
/// Session-method command rides as a [`Job`]; the run loop, recording, and input
/// commands the engine must observe get their own variants.
enum Request {
    Job(Job),
    Run,
    Pause(Sender<()>),
    SetControl(ControlId, ControlInput),
    SetMemoryInterest(Vec<MemoryInterest>),
    StartRecording(PathBuf, Sender<Result<(), String>>),
    StopRecording(Sender<Result<(), String>>),
    Shutdown(Sender<()>),
}

/// The cloneable client handle. All access to the machine flows through it.
#[derive(Clone)]
pub struct SessionHandle {
    requests: Sender<Request>,
    slots: Slots,
}

impl SessionHandle {
    /// Run `f` against the owned [`Session`] on the session thread and block for
    /// its result. This is the universal readout/command path: while paused the
    /// closure sees the live core immediately, and while running it runs at the
    /// next frame boundary — never mid-frame.
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

    /// Start free-running: the loop paces frame stepping and publishes the slots
    /// until [`pause`](Self::pause) or a breakpoint/watch stop.
    pub fn run(&self) {
        let _ = self.requests.send(Request::Run);
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

    /// Drive a console control. Applied in arrival order; noted into an active
    /// recording so a recorded input lands at the frame boundary it happened on.
    pub fn set_control(&self, control: ControlId, input: ControlInput) {
        let _ = self.requests.send(Request::SetControl(control, input));
    }

    /// Set the spans the run loop peeks into the memory-window slot each vblank.
    pub fn set_memory_interest(&self, interest: Vec<MemoryInterest>) {
        let _ = self.requests.send(Request::SetMemoryInterest(interest));
    }

    /// Begin capturing an input recording to `path`, finalized by
    /// [`stop_recording`](Self::stop_recording). Errors when the system has no
    /// save-state backend or its boundary state cannot be captured.
    pub fn start_recording(&self, path: PathBuf) -> Result<(), String> {
        let (tx, rx) = channel();
        self.requests
            .send(Request::StartRecording(path, tx))
            .map_err(|_| "session thread gone".to_string())?;
        rx.recv().map_err(|_| "session thread gone".to_string())?
    }

    /// Finish and write the active recording. A no-op when nothing is recording.
    pub fn stop_recording(&self) -> Result<(), String> {
        let (tx, rx) = channel();
        self.requests
            .send(Request::StopRecording(tx))
            .map_err(|_| "session thread gone".to_string())?;
        rx.recv().map_err(|_| "session thread gone".to_string())?
    }

    /// The latest published frame, or `None` before the first frame runs.
    pub fn latest_frame(&self) -> Option<RgbaFrame> {
        self.slots.frame.lock().ok().and_then(|slot| slot.clone())
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
        if !self.is_running() {
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
        let slots = Slots::new();
        let (requests_tx, requests_rx) = channel();
        let handle = SessionHandle {
            requests: requests_tx,
            slots: slots.clone(),
        };
        let engine_slots = slots;
        let thread = std::thread::Builder::new()
            .name("session".into())
            .spawn(move || {
                SessionEngine::new(Session::new(debugger), engine_slots, sink).serve(requests_rx);
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

/// An input recording being captured from the owned session as it steps frames.
/// Reuses the recording container directly — the core's `Recorder` is typed to
/// `&mut dyn SystemConsole`, which the debugger-owning session does not hold.
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

    fn finish(self) -> Result<(), String> {
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

/// The session thread's engine: the owned [`Session`] plus the run-loop state.
struct SessionEngine {
    session: Session,
    slots: Slots,
    running: bool,
    memory_interest: Vec<MemoryInterest>,
    capture: Option<Capture>,
    audio: Option<AudioSink>,
    next_deadline: Instant,
}

impl SessionEngine {
    fn new(session: Session, slots: Slots, audio: Option<AudioSink>) -> Self {
        SessionEngine {
            session,
            slots,
            running: false,
            memory_interest: Vec::new(),
            capture: None,
            audio,
            next_deadline: Instant::now(),
        }
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
                                return;
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
                            return;
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
            Request::Job(job) => job(&mut self.session),
            Request::Run => self.start_running(),
            Request::Pause(ack) => {
                self.stop_running();
                let _ = ack.send(());
            }
            Request::SetControl(control, input) => {
                self.session.set_control(control, input);
                if let Some(capture) = &mut self.capture {
                    capture.note_input(control, input);
                }
            }
            Request::SetMemoryInterest(interest) => {
                self.memory_interest = interest;
                self.publish_memory_windows();
            }
            Request::StartRecording(path, ack) => {
                let _ = ack.send(self.begin_recording(path));
            }
            Request::StopRecording(ack) => {
                let _ = ack.send(self.finish_recording());
            }
            Request::Shutdown(ack) => {
                let _ = self.finish_recording();
                let _ = ack.send(());
                return true;
            }
        }
        false
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
        let (reason, frame) = self.session.advance_frame();
        if let Some(capture) = &mut self.capture {
            capture.note_frame(frame.as_ref());
        }
        let produced = frame.is_some();
        if let Some(frame) = &frame
            && let Ok(mut slot) = self.slots.frame.lock()
        {
            *slot = Some(frame.resolve_rgba());
        }
        if let Ok(mut slot) = self.slots.status.lock() {
            *slot = Some(self.session.running_status());
        }
        if produced {
            if let Ok(mut slot) = self.slots.snapshot.lock() {
                *slot = Some(self.session.snapshot());
            }
            self.publish_memory_windows();
        }

        // Drain the frame's audio so the buffer can't grow unbounded; a frontend
        // sink consumes it, the headless default drops it.
        let samples = self.session.drain_audio_samples();
        if let Some(sink) = &mut self.audio {
            let coupling = self.session.audio_coupling();
            sink(samples, coupling);
        }

        if matches!(reason, StopReason::Breakpoint | StopReason::Watch(_)) {
            self.stop_running();
        }
    }

    fn publish_memory_windows(&self) {
        let windows: Vec<MemoryWindow> = self
            .memory_interest
            .iter()
            .map(|interest| interest.read_through(&self.session))
            .collect();
        if let Ok(mut slot) = self.slots.memory_windows.lock() {
            *slot = windows;
        }
    }

    /// Fixed-timestep pacing against a wall clock: sleep when ahead, drop the
    /// backlog when it exceeds the deficit cap.
    fn pace(&mut self) {
        let interval = self.session.frame_interval();
        self.next_deadline += interval;
        let now = Instant::now();
        if now < self.next_deadline {
            std::thread::sleep(self.next_deadline - now);
        } else if now - self.next_deadline > interval * MAX_DEFICIT_FRAMES {
            self.next_deadline = now;
        }
    }

    fn begin_recording(&mut self, path: PathBuf) -> Result<(), String> {
        // Finalize any recording already running before starting a fresh one, so
        // its file is written rather than dropped.
        self.finish_recording()?;
        let initial_state = self
            .session
            .save_state_bytes()
            .ok_or("this system has no save-state backend")?;
        // Re-seat from the captured state so the recorded timeline is the exact
        // continuation replay reproduces.
        self.session
            .load_state_bytes(&initial_state)
            .map_err(|error| error.to_string())?;
        self.capture = Some(Capture {
            initial_state,
            inputs: Vec::new(),
            checks: Vec::new(),
            frame: 0,
            check_interval: RECORDING_CHECK_INTERVAL,
            path,
        });
        Ok(())
    }

    fn finish_recording(&mut self) -> Result<(), String> {
        match self.capture.take() {
            Some(capture) => capture.finish(),
            None => Ok(()),
        }
    }
}
