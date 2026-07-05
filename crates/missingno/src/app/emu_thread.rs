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

use missingno_gb::joypad::Button;

use super::audio_output::AudioOutput;
use super::console::AnyConsole;
use super::library::activity::FrameCapture;
use super::screen::ScreenDisplay;

/// One emulated frame at the DMG dot rate (~59.7 Hz).
const FRAME_INTERVAL: Duration = Duration::from_micros(16_740);

/// Cap on how far behind schedule the loop is allowed to fall before the
/// backlog is dropped — degrades to slow-but-steady instead of spiralling.
const MAX_DEFICIT: Duration = Duration::from_micros(16_740 * 4);

/// Frames of quiet before a debounced SRAM save is emitted. Games write SRAM
/// across several consecutive frames during a save; we wait for writes to stop.
const SRAM_DEBOUNCE_FRAMES: u32 = 30;

/// The latest fully-rendered frame, overwritten each `new_screen`. A latest-value
/// handoff, not a queue: the UI reads whatever is current on redraw.
pub type FrameSlot = Arc<Mutex<Option<ScreenDisplay>>>;

/// The emulatable payload the emu thread owns while running. Only the plain
/// console runs off-thread today; the debugger still steps on the UI thread.
pub enum Payload {
    Console(Box<AnyConsole>),
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
    Press(Button),
    Release(Button),
    RequestScreenshot {
        use_sgb_colors: bool,
        palette: String,
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
}

/// The UI-side handle to the emu thread. Cloneable so it can ride in a Message;
/// the return receiver is shared behind a mutex (single consumer in practice).
#[derive(Clone, Debug)]
pub struct EmuHandle {
    commands: Sender<EmuCommand>,
    frames: FrameSlot,
    returns: Arc<Mutex<Receiver<Payload>>>,
}

impl EmuHandle {
    pub fn frames(&self) -> &FrameSlot {
        &self.frames
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

    /// Recover the payload the thread returned in response to `Pause`,
    /// `Shutdown`, or a breakpoint stop.
    pub fn recover(&self) -> Option<Payload> {
        self.returns
            .lock()
            .ok()?
            .recv_timeout(Duration::from_millis(500))
            .ok()
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
    let frames: FrameSlot = Arc::new(Mutex::new(None));

    let handle = EmuHandle {
        commands: command_tx,
        frames: frames.clone(),
        returns: Arc::new(Mutex::new(return_rx)),
    };
    let _ = event_tx.unbounded_send(EmuEvent::Started(handle));

    let worker_events = event_tx;
    std::thread::Builder::new()
        .name("emu".into())
        .spawn(move || run_emu_thread(command_rx, return_tx, frames, worker_events))
        .expect("spawn emu thread");

    event_rx
}

type EventSink = iced::futures::channel::mpsc::UnboundedSender<EmuEvent>;

fn run_emu_thread(
    commands: Receiver<EmuCommand>,
    returns: Sender<Payload>,
    frames: FrameSlot,
    events: EventSink,
) {
    // Audio device lives on this thread (cpal's Stream is `!Send`).
    let mut audio = AudioOutput::new();
    let mut state = EmuLoop::new(frames, events, returns);

    loop {
        if state.running() {
            // Drain pending commands without blocking, then emulate one frame.
            loop {
                match commands.try_recv() {
                    Ok(command) => state.handle(command),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if state.running() {
                state.emulate_frame(&mut audio);
                state.pace();
            }
        } else {
            // Idle: block until the next command (with a timeout so a paused
            // thread stays responsive if the UI drops the channel).
            match commands.recv_timeout(Duration::from_millis(200)) {
                Ok(command) => state.handle(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

struct EmuLoop {
    payload: Option<Payload>,
    frames: FrameSlot,
    events: EventSink,
    returns: Sender<Payload>,
    sram_countdown: Option<u32>,
    next_deadline: Instant,
}

impl EmuLoop {
    fn new(frames: FrameSlot, events: EventSink, returns: Sender<Payload>) -> Self {
        Self {
            payload: None,
            frames,
            events,
            returns,
            sram_countdown: None,
            next_deadline: Instant::now(),
        }
    }

    fn running(&self) -> bool {
        self.payload.is_some()
    }

    fn handle(&mut self, command: EmuCommand) {
        match command {
            EmuCommand::Run(payload) => {
                self.payload = Some(payload);
                self.sram_countdown = None;
                self.next_deadline = Instant::now();
            }
            EmuCommand::Pause => self.return_payload(),
            EmuCommand::Reset => {
                if let Some(payload) = &mut self.payload {
                    payload.reset();
                }
            }
            EmuCommand::Press(button) => {
                if let Some(payload) = &mut self.payload {
                    payload.press_button(button);
                }
            }
            EmuCommand::Release(button) => {
                if let Some(payload) = &mut self.payload {
                    payload.release_button(button);
                }
            }
            EmuCommand::RequestScreenshot {
                use_sgb_colors,
                palette,
            } => {
                if let Some(payload) = &self.payload {
                    let capture = payload.capture_frame(use_sgb_colors, &palette);
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
        let mut sram_dirty = false;
        if let Some(display) = payload.step_frame(&mut sram_dirty) {
            if let Ok(mut slot) = self.frames.lock() {
                *slot = Some(display);
            }
            let _ = self.events.unbounded_send(EmuEvent::FrameReady);
        }
        if let Some(audio) = audio {
            audio.push_samples(&payload.drain_audio_samples());
        }

        // Debounce SRAM: reset countdown on a dirty frame, flush after quiet.
        if sram_dirty {
            self.sram_countdown = Some(0);
        } else if let Some(count) = &mut self.sram_countdown {
            *count += 1;
            if *count >= SRAM_DEBOUNCE_FRAMES {
                self.flush_sram();
            }
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

    /// Fixed-timestep pacing against a wall clock: sleep when ahead, drop the
    /// backlog when it exceeds the deficit cap.
    fn pace(&mut self) {
        self.next_deadline += FRAME_INTERVAL;
        let now = Instant::now();
        if now < self.next_deadline {
            std::thread::sleep(self.next_deadline - now);
        } else if now - self.next_deadline > MAX_DEFICIT {
            self.next_deadline = now;
        }
    }
}

impl Payload {
    fn reset(&mut self) {
        match self {
            Self::Console(console) => console.reset(),
        }
    }

    fn press_button(&mut self, button: Button) {
        match self {
            Self::Console(console) => console.press_button(button),
        }
    }

    fn release_button(&mut self, button: Button) {
        match self {
            Self::Console(console) => console.release_button(button),
        }
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match self {
            Self::Console(console) => console.drain_audio_samples(),
        }
    }

    fn capture_frame(&self, use_sgb_colors: bool, palette: &str) -> FrameCapture {
        match self {
            Self::Console(console) => console.capture_frame(use_sgb_colors, palette),
        }
    }

    fn sram(&self) -> Option<Vec<u8>> {
        let cartridge = match self {
            Self::Console(console) => console.cartridge(),
        };
        if !cartridge.has_battery() {
            return None;
        }
        cartridge.ram().map(|ram| ram.to_vec())
    }

    /// Emulate up to one frame, returning the completed display if any. The
    /// LCD-off guard caps the step budget so an idle PPU can't stall.
    fn step_frame(&mut self, sram_dirty: &mut bool) -> Option<ScreenDisplay> {
        match self {
            Self::Console(console) => {
                let max = 70224 * 2 * console.cpu_tcycles_per_dot() as u32;
                let mut tcycles = 0;
                loop {
                    let result = console.step();
                    tcycles += result.tcycles;
                    *sram_dirty |= result.sram_dirty;
                    if result.new_screen || tcycles >= max {
                        break;
                    }
                }
                Some(console.screen_display())
            }
        }
    }
}
