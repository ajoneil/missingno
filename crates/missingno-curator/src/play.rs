//! The playtest pane: host a real free-running session (with sound) for the
//! selected entry and feed it gamepad input.

use std::sync::{Arc, Mutex, mpsc::Receiver};
use std::time::Duration;

use iced::futures::SinkExt;
use missingno_core::system::{ConsoleSwitch, ControlId, ControlInput};
use missingno_core::video::DisplayTechnology;
use missingno_session::{
    SessionEvent, SessionHandle, SharedSession, audio_output::AudioOutput, factory,
};

pub struct PlaySession {
    /// Owns the session thread; dropping stops the machine.
    _shared: SharedSession,
    pub handle: SessionHandle,
    /// The display the console states, driving the screen renderer.
    pub technology: DisplayTechnology,
    /// The family's latching console switches, captured before the console
    /// moves into the session, with the level the UI last set for each.
    pub switches: &'static [ConsoleSwitch],
    pub switch_levels: Vec<bool>,
    /// The `!Send` cpal stream stays on the UI thread, as in the emulator.
    _audio: Option<AudioOutput>,
    pub events: Arc<Mutex<Receiver<SessionEvent>>>,
}

pub fn start(
    filename_hint: &str,
    rom: &[u8],
    tv_standard: Option<String>,
    cart_type: Option<String>,
) -> Result<PlaySession, String> {
    let options = factory::LoadOptions {
        tv_standard,
        boot_rom: None,
        cart_type,
    };
    let console = factory::create_console_with(std::path::Path::new(filename_hint), rom, &options)
        .map_err(|e| format!("core rejected ROM: {e}"))?
        .ok_or("no core recognizes this ROM")?;
    let technology = console.video_out();
    let switches = console.console_switches();
    let switch_levels = switches.iter().map(|s| s.default_high).collect();
    let (audio, sink) = match AudioOutput::open() {
        Some((audio, sink)) => (Some(audio), Some(sink)),
        None => (None, None),
    };
    let shared = SharedSession::spawn_console_with_audio(console, sink);
    let handle = shared.handle();
    let events = handle.subscribe();
    handle.run();
    Ok(PlaySession {
        _shared: shared,
        handle,
        technology,
        switches,
        switch_levels,
        _audio: audio,
        events: Arc::new(Mutex::new(events)),
    })
}

impl PlaySession {
    pub fn set_control(&self, id: u8, pressed: bool) {
        self.handle
            .set_control(ControlId(id), ControlInput::Digital(pressed));
    }
}

/// Block until the session produces a frame (or dies); coalesces a backlog.
pub fn await_frame(events: &Arc<Mutex<Receiver<SessionEvent>>>) -> bool {
    let events = events.lock().unwrap();
    loop {
        match events.recv() {
            Ok(SessionEvent::FrameReady) => {
                while let Ok(event) = events.try_recv() {
                    if matches!(event, SessionEvent::Stopped) {
                        return false;
                    }
                }
                return true;
            }
            Ok(SessionEvent::Stopped) | Err(_) => return false,
            Ok(_) => continue,
        }
    }
}

/// Shared control layout: 0 Start/Reset, 1 Select, 2 A/Fire, 3 B/Fire, 4-7 dpad.
fn button_control(button: gilrs::Button) -> Option<u8> {
    Some(match button {
        gilrs::Button::Start => 0,
        gilrs::Button::Select => 1,
        gilrs::Button::South => 2,
        gilrs::Button::East => 3,
        gilrs::Button::DPadUp => 4,
        gilrs::Button::DPadDown => 5,
        gilrs::Button::DPadLeft => 6,
        gilrs::Button::DPadRight => 7,
        _ => return None,
    })
}

/// Gamepad events → (control id, pressed), same stick handling as the emulator.
pub fn gamepad_worker() -> impl iced::futures::Stream<Item = (u8, bool)> {
    iced::stream::channel(64, async move |mut output| {
        let Ok(mut gilrs) = gilrs::Gilrs::new() else {
            return;
        };
        let mut stick = [false; 4]; // up, down, left, right
        const DEADZONE: f32 = 0.5;
        loop {
            while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                match event {
                    gilrs::EventType::ButtonPressed(button, ..) => {
                        if let Some(id) = button_control(button) {
                            let _ = output.send((id, true)).await;
                        }
                    }
                    gilrs::EventType::ButtonReleased(button, ..) => {
                        if let Some(id) = button_control(button) {
                            let _ = output.send((id, false)).await;
                        }
                    }
                    gilrs::EventType::AxisChanged(axis, value, ..) => {
                        let changes: [(usize, u8, bool); 2] = match axis {
                            gilrs::Axis::LeftStickX => {
                                [(3, 7, value > DEADZONE), (2, 6, value < -DEADZONE)]
                            }
                            gilrs::Axis::LeftStickY => {
                                [(0, 4, value > DEADZONE), (1, 5, value < -DEADZONE)]
                            }
                            _ => continue,
                        };
                        for (slot, id, now) in changes {
                            if stick[slot] != now {
                                stick[slot] = now;
                                let _ = output.send((id, now)).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
            smol::Timer::after(Duration::from_millis(4)).await;
        }
    })
}
