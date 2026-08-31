//! The playtest pane: host a real free-running session (with sound) for the
//! selected entry and feed it gamepad input.

use std::sync::{Arc, Mutex, mpsc::Receiver};
use std::time::Duration;

use iced::futures::SinkExt;
use missingno_core::cartridge::BoardValue;
use missingno_core::launch::LaunchValues;
use missingno_core::ports::{PanelControl, PeripheralId, PortId};
use missingno_core::system::{ControlId, ControlInput, ControlRole};
use missingno_core::video::DisplayTechnology;
use missingno_gamedb::platform::Peripheral;
use missingno_session::{
    SessionEvent, SessionHandle, SharedSession, audio_output::AudioOutput, factory,
};

pub struct PlaySession {
    /// Owns the session thread; dropping stops the machine.
    _shared: SharedSession,
    pub handle: SessionHandle,
    /// The display the console states, driving the screen renderer.
    pub technology: DisplayTechnology,
    /// The console's own panel controls, captured before the console moves
    /// into the session, with the level the UI last set for each toggle.
    pub switches: Vec<PanelControl>,
    pub switch_levels: Vec<bool>,
    /// A paddle pair is in the play jack, so the pane aims it with the pointer
    /// and fires it with a click.
    pub paddles: bool,
    /// The jacks holding a keypad, so host key presses know where to land.
    pub keypads: Vec<PortId>,
    /// The jack the host gamepad is patched into. A game that reads the right
    /// controller is only playable with the pad moved there.
    pub pad_jack: PortId,
    /// Roles on the console's own controller — the Game Boy's pad. Empty on a
    /// console whose controllers all arrive through jacks.
    integrated_roles: Vec<ControlRole>,
    /// Roles on the console shell, where the VCS keeps Reset and Select.
    panel_roles: Vec<ControlRole>,
    /// The `!Send` cpal stream stays on the UI thread, as in the emulator.
    _audio: Option<AudioOutput>,
    pub events: Arc<Mutex<Receiver<SessionEvent>>>,
}

/// What each jack gets from the peripherals the db states. A keypad game wants
/// one in each jack unless it also states the joystick, the arrangement
/// keypad-plus-joystick titles use: stick left, keypad right. A paddle game
/// takes the pair in the jack the pane drives.
fn jack_peripherals(stated: &[Peripheral]) -> [PeripheralId; 2] {
    let has = |peripheral| stated.contains(&peripheral);
    if has(Peripheral::Keypad) {
        if has(Peripheral::Joystick) {
            [missingno_vcs::debug::JOYSTICK, missingno_vcs::debug::KEYPAD]
        } else {
            [missingno_vcs::debug::KEYPAD; 2]
        }
    } else if has(Peripheral::Paddle) {
        [
            missingno_vcs::debug::PADDLES,
            missingno_vcs::debug::JOYSTICK,
        ]
    } else {
        [missingno_vcs::debug::JOYSTICK; 2]
    }
}

pub fn start(
    filename_hint: &str,
    rom: &[u8],
    tv_standard: Option<String>,
    cart_type: Option<BoardValue>,
    runner: Option<&str>,
    overdump: bool,
    peripherals: &[Peripheral],
) -> Result<PlaySession, String> {
    let mut launch = LaunchValues::default();
    if let Some(standard) = tv_standard {
        launch.set_choice(missingno_vcs::debug::TV_STANDARD, standard);
    }
    if let Some(board) = cart_type {
        // Every core with a board vocabulary publishes it under one id.
        launch.set_board(missingno_vcs::debug::BOARD, board);
    }
    if let Some(console) = runner {
        // Left to the header, a Color-enhanced flag boots the Color core.
        launch.set_choice(missingno_gbc::launch::RUNNER, console);
    }
    launch.set_toggle(missingno_vcs::debug::OVERDUMP, overdump);
    let mut console =
        factory::create_console_with(std::path::Path::new(filename_hint), rom, &launch)
            .map_err(|error| format!("core rejected ROM: {error}"))?;
    // Knob and key input reach nothing until the peripheral is in the jack, and
    // a paddle trigger lands on the direction line it shares on real hardware.
    let mut plugged = [missingno_vcs::debug::JOYSTICK; 2];
    for (jack, peripheral) in jack_peripherals(peripherals).into_iter().enumerate() {
        let port = PortId(jack as u8);
        if console.plug(port, peripheral).is_ok() {
            plugged[jack] = peripheral;
        }
    }
    let paddles = plugged[PLAY_PORT.0 as usize] == missingno_vcs::debug::PADDLES;
    let keypads = plugged
        .iter()
        .enumerate()
        .filter(|&(_, &peripheral)| peripheral == missingno_vcs::debug::KEYPAD)
        .map(|(jack, _)| PortId(jack as u8))
        .collect();
    let technology = console.video_out();
    let integrated_roles = console
        .integrated_controls()
        .iter()
        .map(|control| control.role)
        .collect();
    let switches: Vec<PanelControl> = console.panel_controls().to_vec();
    let panel_roles = switches.iter().map(|switch| switch.role).collect();
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
        keypads,
        pad_jack: PLAY_PORT,
        integrated_roles,
        panel_roles,
        _audio: audio,
        events: Arc::new(Mutex::new(events)),
    })
}

/// The jack the playtest starts in — the curator plays VCS media, whose left
/// controller is the one most games read.
pub const PLAY_PORT: PortId = PortId(0);

/// What the gamepad holds in a jack, released when it moves to the other one.
const PAD_ROLES: [ControlRole; 6] = [
    ControlRole::Up,
    ControlRole::Down,
    ControlRole::Left,
    ControlRole::Right,
    ControlRole::Action(0),
    ControlRole::Action(1),
];

impl PlaySession {
    pub fn set_control(&self, control: ControlId, pressed: bool) {
        self.handle
            .set_control(control, ControlInput::Digital(pressed));
    }

    /// Where a host pad button lands on this console: its own controller if it
    /// has one, else the shell, else the controller in the pad's jack. Asking
    /// the console rather than assuming jacks is what makes one pad play a
    /// Game Boy, whose buttons are all integrated, and a VCS, whose are not.
    fn resolve(&self, roles: &[ControlRole]) -> ControlId {
        resolve_role(
            roles,
            &self.integrated_roles,
            &self.panel_roles,
            self.pad_jack,
        )
    }

    /// A host gamepad button, landing wherever this console keeps that control.
    pub fn set_pad_control(&self, roles: &[ControlRole], pressed: bool) {
        self.set_control(self.resolve(roles), pressed);
    }

    /// Patch the gamepad into the other jack, releasing what it held so a
    /// direction pressed across the move doesn't stick in the jack it left.
    pub fn swap_pad_jack(&mut self) {
        for role in PAD_ROLES {
            if !self.integrated_roles.contains(&role) {
                self.set_control(ControlId::port(self.pad_jack, role), false);
            }
        }
        self.pad_jack = if self.pad_jack == missingno_vcs::debug::LEFT_PORT {
            missingno_vcs::debug::RIGHT_PORT
        } else {
            missingno_vcs::debug::LEFT_PORT
        };
    }

    /// A host key onto a plugged keypad: the left one unmodified and the right
    /// with Shift, except that a lone keypad answers either way.
    pub fn set_key(&self, key: u8, shift: bool, pressed: bool) {
        let port = match self.keypads.as_slice() {
            [only] => *only,
            _ if shift => missingno_vcs::debug::RIGHT_PORT,
            _ => missingno_vcs::debug::LEFT_PORT,
        };
        if self.keypads.contains(&port) {
            self.set_control(ControlId::port(port, ControlRole::Key(key)), pressed);
        }
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

/// Where a host pad button lands on a console that states these roles. Taking
/// the console's word is what lets one pad play a Game Boy, whose buttons are
/// all on its own controller, and a VCS, which has none of its own.
fn resolve_role(
    roles: &[ControlRole],
    integrated: &[ControlRole],
    panel: &[ControlRole],
    pad_jack: PortId,
) -> ControlId {
    for role in roles {
        if integrated.contains(role) {
            return ControlId::integrated(*role);
        }
        if panel.contains(role) {
            return ControlId::panel(*role);
        }
    }
    ControlId::port(pad_jack, roles[0])
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

/// What a host pad button asks for, most specific first: a console lacking the
/// first role answers with the next. Start is the pad's own on a Game Boy and
/// the console panel's Reset on a VCS, which is the same request either way.
fn button_roles(button: gilrs::Button) -> Option<&'static [ControlRole]> {
    Some(match button {
        gilrs::Button::Start => &[ControlRole::Start, ControlRole::Reset],
        gilrs::Button::Select => &[ControlRole::Select],
        // The emulator's default pad layout: South fires, East is the second
        // button on pads that have one.
        gilrs::Button::South => &[ControlRole::Action(0)],
        gilrs::Button::East => &[ControlRole::Action(1)],
        gilrs::Button::DPadUp => &[ControlRole::Up],
        gilrs::Button::DPadDown => &[ControlRole::Down],
        gilrs::Button::DPadLeft => &[ControlRole::Left],
        gilrs::Button::DPadRight => &[ControlRole::Right],
        _ => return None,
    })
}

/// A host key onto a keypad key, row-major from the pad's top left. Digits sit
/// where they read, on the top row or the numpad; the numpad's `*` and `/`
/// carry `*` and `#`, and where there is no numpad the two keys past the digit
/// row (`-` and `=`) stand in for them.
pub fn keypad_key(key: &iced::keyboard::Key) -> Option<u8> {
    let iced::keyboard::Key::Character(text) = key else {
        return None;
    };
    Some(match text.as_str() {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        "4" => 3,
        "5" => 4,
        "6" => 5,
        "7" => 6,
        "8" => 7,
        "9" => 8,
        "*" | "-" => 9,
        "0" => 10,
        "/" | "=" => 11,
        _ => return None,
    })
}

/// A gamepad's contribution to the playtest: a shared-layout button edge, or
/// the trigger-wound paddle arriving at a new position.
#[derive(Clone, Copy, Debug)]
pub enum PadEvent {
    Button(&'static [ControlRole], bool),
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
                        if let Some(roles) = button_roles(button) {
                            let _ = output.send(PadEvent::Button(roles, true)).await;
                        }
                    }
                    gilrs::EventType::ButtonReleased(button, ..) => {
                        if let Some(roles) = button_roles(button) {
                            let _ = output.send(PadEvent::Button(roles, false)).await;
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
                        let changes: [(usize, &'static [ControlRole], bool); 2] = match axis {
                            gilrs::Axis::LeftStickX => [
                                (3, &[ControlRole::Right], value > DEADZONE),
                                (2, &[ControlRole::Left], value < -DEADZONE),
                            ],
                            gilrs::Axis::LeftStickY => [
                                (0, &[ControlRole::Up], value > DEADZONE),
                                (1, &[ControlRole::Down], value < -DEADZONE),
                            ],
                            _ => continue,
                        };
                        for (slot, roles, now) in changes {
                            if stick[slot] != now {
                                stick[slot] = now;
                                let _ = output.send(PadEvent::Button(roles, now)).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::system::ControlSite;

    /// The Game Boy's buttons are all on its own pad, Start and Select
    /// included — the roles the VCS keeps on its shell and in a jack.
    const GAME_BOY: [ControlRole; 8] = [
        ControlRole::Up,
        ControlRole::Down,
        ControlRole::Left,
        ControlRole::Right,
        ControlRole::Action(0),
        ControlRole::Action(1),
        ControlRole::Start,
        ControlRole::Select,
    ];

    /// The VCS states no controller of its own; its shell carries these.
    const VCS_PANEL: [ControlRole; 2] = [ControlRole::Reset, ControlRole::Select];

    fn game_boy(button: gilrs::Button) -> ControlId {
        resolve_role(button_roles(button).unwrap(), &GAME_BOY, &[], PLAY_PORT)
    }

    fn vcs(button: gilrs::Button) -> ControlId {
        resolve_role(button_roles(button).unwrap(), &[], &VCS_PANEL, PLAY_PORT)
    }

    #[test]
    fn a_game_boy_takes_every_button_on_its_own_pad() {
        for button in [
            gilrs::Button::South,
            gilrs::Button::East,
            gilrs::Button::DPadUp,
            gilrs::Button::Start,
            gilrs::Button::Select,
        ] {
            assert_eq!(
                game_boy(button).site,
                ControlSite::Integrated,
                "{button:?} did not reach the console's own pad"
            );
        }
        assert_eq!(game_boy(gilrs::Button::Start).role, ControlRole::Start);
        assert_eq!(game_boy(gilrs::Button::South).role, ControlRole::Action(0));
    }

    /// The same pad on a console with no controller of its own: the shell
    /// answers what it has, and the rest is the joystick in the jack.
    #[test]
    fn a_vcs_takes_the_shells_buttons_and_the_jacks() {
        assert_eq!(
            vcs(gilrs::Button::Start),
            ControlId::panel(ControlRole::Reset),
            "the pad's Start is the console's Reset where there is no Start"
        );
        assert_eq!(
            vcs(gilrs::Button::Select),
            ControlId::panel(ControlRole::Select)
        );
        assert_eq!(
            vcs(gilrs::Button::South),
            ControlId::port(PLAY_PORT, ControlRole::Action(0))
        );
        assert_eq!(
            vcs(gilrs::Button::DPadUp),
            ControlId::port(PLAY_PORT, ControlRole::Up)
        );
    }
}
