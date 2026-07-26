//! The playtest pane: host a real free-running session (with sound) for the
//! selected entry and feed it gamepad input.

use std::sync::{Arc, Mutex, mpsc::Receiver};
use std::time::Duration;

use iced::futures::SinkExt;
use missingno_core::ports::{PanelControl, PortId};
use missingno_core::system::{ControlId, ControlInput, ControlRole};
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
    /// The console's latching panel switches, captured before the console
    /// moves into the session, with the level the UI last set for each.
    pub switches: Vec<PanelControl>,
    pub switch_levels: Vec<bool>,
    /// A paddle pair is in the play jack, so the pane aims it with the pointer
    /// and fires it with a click.
    pub paddles: bool,
    /// The `!Send` cpal stream stays on the UI thread, as in the emulator.
    _audio: Option<AudioOutput>,
    pub events: Arc<Mutex<Receiver<SessionEvent>>>,
}

pub fn start(
    filename_hint: &str,
    rom: &[u8],
    tv_standard: Option<String>,
    cart_type: Option<String>,
    paddles: bool,
) -> Result<PlaySession, String> {
    let options = factory::LoadOptions {
        tv_standard,
        boot_rom: None,
        cart_type,
    };
    let mut console =
        factory::create_console_with(std::path::Path::new(filename_hint), rom, &options)
            .map_err(|e| format!("core rejected ROM: {e}"))?
            .ok_or("no core recognizes this ROM")?;
    // Knob input reaches nothing until the pair is in the jack, and the paddle
    // trigger lands on the direction line it shares on real hardware.
    let paddles = paddles
        && console
            .plug(PLAY_PORT, missingno_vcs::debug::PADDLES)
            .is_ok();
    let technology = console.video_out();
    let switches: Vec<PanelControl> = console
        .panel_controls()
        .iter()
        .filter(|control| control.toggle().is_some())
        .copied()
        .collect();
    let switch_levels = switches
        .iter()
        .map(|switch| {
            switch
                .toggle()
                .is_some_and(|(_, default_high)| default_high)
        })
        .collect();
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
        paddles,
        _audio: audio,
        events: Arc::new(Mutex::new(events)),
    })
}

/// The jack the playtest drives — the curator plays VCS media, whose left
/// controller is the one games read.
pub const PLAY_PORT: PortId = PortId(0);

impl PlaySession {
    pub fn set_control(&self, control: ControlId, pressed: bool) {
        self.handle
            .set_control(control, ControlInput::Digital(pressed));
    }

    /// Screen-right maps to the fast-charging end of the pot, which paddle
    /// games read as right.
    pub fn set_paddle(&self, position: f32) {
        self.handle.set_control(
            ControlId::port(PLAY_PORT, ControlRole::Knob(0)),
            ControlInput::Axis(1.0 - position),
        );
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

/// The playtest pad's reading of a gamepad: the console panel's buttons, and
/// the joystick in the left jack.
fn button_control(button: gilrs::Button) -> Option<ControlId> {
    Some(match button {
        gilrs::Button::Start => ControlId::panel(ControlRole::Reset),
        gilrs::Button::Select => ControlId::panel(ControlRole::Select),
        gilrs::Button::South | gilrs::Button::East => {
            ControlId::port(PLAY_PORT, ControlRole::Action(0))
        }
        gilrs::Button::DPadUp => ControlId::port(PLAY_PORT, ControlRole::Up),
        gilrs::Button::DPadDown => ControlId::port(PLAY_PORT, ControlRole::Down),
        gilrs::Button::DPadLeft => ControlId::port(PLAY_PORT, ControlRole::Left),
        gilrs::Button::DPadRight => ControlId::port(PLAY_PORT, ControlRole::Right),
        _ => return None,
    })
}

/// A gamepad's contribution to the playtest: a shared-layout button edge, or
/// the trigger-wound paddle arriving at a new position.
#[derive(Clone, Copy, Debug)]
pub enum PadEvent {
    Button(ControlId, bool),
    Paddle(f32),
}

/// Gamepad events → [`PadEvent`], same stick handling as the emulator; the
/// analog triggers wind the paddle at squeeze-scaled speed.
pub fn gamepad_worker() -> impl iced::futures::Stream<Item = PadEvent> {
    iced::stream::channel(64, async move |mut output| {
        let Ok(mut gilrs) = gilrs::Gilrs::new() else {
            return;
        };
        let mut stick = [false; 4]; // up, down, left, right
        let mut paddle = missingno_iced::PaddleWind::new();
        let mut trigger_floor = [0.0f32; 2];
        let mut last_tick = std::time::Instant::now();
        const DEADZONE: f32 = 0.5;
        loop {
            while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                match event {
                    gilrs::EventType::ButtonPressed(button, ..) => {
                        if let Some(id) = button_control(button) {
                            let _ = output.send(PadEvent::Button(id, true)).await;
                        }
                    }
                    gilrs::EventType::ButtonReleased(button, ..) => {
                        if let Some(id) = button_control(button) {
                            let _ = output.send(PadEvent::Button(id, false)).await;
                        }
                    }
                    gilrs::EventType::ButtonChanged(gilrs::Button::LeftTrigger2, value, ..) => {
                        paddle.set_left(value);
                    }
                    gilrs::EventType::ButtonChanged(gilrs::Button::RightTrigger2, value, ..) => {
                        paddle.set_right(value);
                    }
                    // Some pads report the analog triggers as axes instead of
                    // button values; ranges differ per driver (0..1 or -1..1),
                    // so normalise against the lowest level seen.
                    gilrs::EventType::AxisChanged(
                        axis @ (gilrs::Axis::LeftZ | gilrs::Axis::RightZ),
                        value,
                        ..,
                    ) => {
                        let slot = usize::from(axis == gilrs::Axis::RightZ);
                        trigger_floor[slot] = trigger_floor[slot].min(value);
                        let depression = if trigger_floor[slot] < 0.0 {
                            (value + 1.0) / 2.0
                        } else {
                            value
                        };
                        match slot {
                            0 => paddle.set_left(depression),
                            _ => paddle.set_right(depression),
                        }
                    }
                    gilrs::EventType::AxisChanged(axis, value, ..) => {
                        let stick_control = |role| ControlId::port(PLAY_PORT, role);
                        let changes: [(usize, ControlId, bool); 2] = match axis {
                            gilrs::Axis::LeftStickX => [
                                (3, stick_control(ControlRole::Right), value > DEADZONE),
                                (2, stick_control(ControlRole::Left), value < -DEADZONE),
                            ],
                            gilrs::Axis::LeftStickY => [
                                (0, stick_control(ControlRole::Up), value > DEADZONE),
                                (1, stick_control(ControlRole::Down), value < -DEADZONE),
                            ],
                            _ => continue,
                        };
                        for (slot, control, now) in changes {
                            if stick[slot] != now {
                                stick[slot] = now;
                                let _ = output.send(PadEvent::Button(control, now)).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
            let dt = last_tick.elapsed().as_secs_f32();
            last_tick = std::time::Instant::now();
            if let Some(position) = paddle.tick(dt) {
                let _ = output.send(PadEvent::Paddle(position)).await;
            }
            smol::Timer::after(Duration::from_millis(4)).await;
        }
    })
}
