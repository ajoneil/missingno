use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use iced::{
    Event, Subscription, event,
    keyboard::{self, Key, key},
    stream,
};

use missingno_core::ports::{PeripheralId, PortId};
use missingno_core::system::{ControlId, ControlRole};
use missingno_session::ControlSurfaces;

use crate::app;
use crate::app::settings::{ControlSlot, EmulatorAction, GamepadIdentity, Surface, WindDirection};

/// One thing an input works on the running machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actuation {
    /// A control held down for as long as the input is.
    Hold(ControlId),
    /// A latching panel switch: a press moves it to its other position. The
    /// emulation layer holds the level, so it decides which that is.
    Flip(ControlRole),
}

/// A host input device that can drive a console port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputSource {
    Keyboard,
    Gamepad(gilrs::GamepadId),
}

/// Which console port each host device drives — one port each. Devices are
/// peers: the keyboard and a pad both start on the first port, so a phantom HID
/// enumerating as a gamepad never takes the player's port away.
#[derive(Debug, Clone)]
pub struct PortAssignments {
    pub keyboard: PortId,
    /// The port each pad drives; a pad with no entry of its own drives the
    /// machine's lowest port.
    pub gamepads: HashMap<gilrs::GamepadId, PortId>,
}

impl Default for PortAssignments {
    fn default() -> Self {
        Self {
            keyboard: PortId(0),
            gamepads: HashMap::new(),
        }
    }
}

/// The machine's first port, the one a device with nothing recorded plays.
pub(super) fn first_port(surfaces: &ControlSurfaces) -> Option<PortId> {
    surfaces
        .ports
        .iter()
        .map(|plugged| plugged.descriptor.port)
        .min_by_key(|port| port.0)
}

/// A gamepad the host has connected: the id this session's events carry, and
/// the identity that recognises it in a later one.
#[derive(Debug, Clone)]
pub struct ConnectedPad {
    pub id: gilrs::GamepadId,
    pub identity: GamepadIdentity,
}

/// One input on one device, as a press remembers itself.
type HeldKey = (InputSource, Surface, String);

/// What a held input is turning: the knobs it winds and which way, and how far
/// it is pushed — a key is all the way down, a trigger as far as it is squeezed.
pub struct Wind {
    knobs: Vec<(ControlId, WindDirection)>,
    magnitude: f32,
}

/// What an input event needs to reach a control: the loaded system's effective
/// bindings, what is plugged where, and which device drives which port.
pub struct Routing {
    pub emulator_keyboard: HashMap<EmulatorAction, String>,
    pub emulator_gamepad: HashMap<EmulatorAction, String>,
    pub system_keyboard: HashMap<ControlSlot, String>,
    pub system_gamepad: HashMap<ControlSlot, String>,
    /// The running machine's ports and built-in controls; `None` with no game
    /// loaded, when no port control can fire.
    pub surfaces: Option<ControlSurfaces>,
    pub assignments: PortAssignments,
    /// Whether the pointer over the screen turns the loaded system's knobs.
    pub pointer_drives_knob: bool,
    /// What each held input actually put down. A release lifts what its own
    /// press held, so a device reassigned (or a controller swapped) while a
    /// key is down does not strand the control it was holding.
    pub(super) held: HashMap<HeldKey, Vec<ControlId>>,
    /// The winding each held input is contributing, lifted by its own release
    /// the way `held` is.
    pub(super) winding: HashMap<HeldKey, Wind>,
    /// Where each wound knob stands. Positions outlive the presses that turn
    /// them — a released knob stays where it was left.
    pub(super) knobs: HashMap<ControlId, missingno_iced::PaddleWind>,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            emulator_keyboard: HashMap::new(),
            emulator_gamepad: HashMap::new(),
            system_keyboard: HashMap::new(),
            system_gamepad: HashMap::new(),
            surfaces: None,
            assignments: PortAssignments::default(),
            pointer_drives_knob: true,
            held: HashMap::new(),
            winding: HashMap::new(),
            knobs: HashMap::new(),
        }
    }
}

static ROUTING: LazyLock<Mutex<Routing>> = LazyLock::new(Mutex::default);

/// Hand the event handlers a fresh routing state. Called from `update()` on
/// every change that moves it: a settings edit, a load, a plug, a device
/// assignment, a pad connecting or going away. What is currently held survives
/// — it belongs to the presses in flight, not to the bindings.
pub fn publish(mut routing: Routing) {
    let mut current = ROUTING.lock().unwrap();
    routing.held = std::mem::take(&mut current.held);
    routing.winding = std::mem::take(&mut current.winding);
    routing.knobs = std::mem::take(&mut current.knobs);
    *current = routing;
}

impl Routing {
    fn emulator(&self, surface: Surface) -> &HashMap<EmulatorAction, String> {
        match surface {
            Surface::Keyboard => &self.emulator_keyboard,
            Surface::Gamepad => &self.emulator_gamepad,
        }
    }

    fn system(&self, surface: Surface) -> &HashMap<ControlSlot, String> {
        match surface {
            Surface::Keyboard => &self.system_keyboard,
            Surface::Gamepad => &self.system_gamepad,
        }
    }

    fn emulator_action(&self, surface: Surface, input: &str) -> Option<EmulatorAction> {
        self.emulator(surface)
            .iter()
            .find(|(_, bound)| bound.as_str() == input)
            .map(|(action, _)| *action)
    }

    /// Whether anything on this surface answers to `input`.
    fn bound(&self, surface: Surface, input: &str) -> bool {
        self.emulator_action(surface, input).is_some()
            || self
                .system(surface)
                .values()
                .any(|bound| bound.as_str() == input)
    }

    /// What this device works with `input` on the loaded machine: the console's
    /// own controls answer to every device, a controller's only while that
    /// controller is plugged into the port this device drives.
    fn controls(&self, source: InputSource, surface: Surface, input: &str) -> Vec<Actuation> {
        self.system(surface)
            .iter()
            .filter(|(_, bound)| bound.as_str() == input)
            .filter_map(|(slot, _)| match *slot {
                ControlSlot::Integrated(role) => Some(Actuation::Hold(ControlId::integrated(role))),
                ControlSlot::Panel(role) if self.latching(role) => Some(Actuation::Flip(role)),
                ControlSlot::Panel(role) => Some(Actuation::Hold(ControlId::panel(role))),
                ControlSlot::Peripheral { peripheral, role } => self
                    .port_control(source, peripheral, role)
                    .map(Actuation::Hold),
                // Winding is integrated here rather than worked as a press.
                ControlSlot::Wind { .. } => None,
            })
            .collect()
    }

    /// The knobs this input winds, and which way. A wind slot passes the same
    /// gate a controller's buttons do: the controller must be in the port this
    /// device drives.
    fn winds(
        &self,
        source: InputSource,
        surface: Surface,
        input: &str,
    ) -> Vec<(ControlId, WindDirection)> {
        self.system(surface)
            .iter()
            .filter(|(_, bound)| bound.as_str() == input)
            .filter_map(|(slot, _)| match *slot {
                ControlSlot::Wind {
                    peripheral,
                    role,
                    direction,
                } => self
                    .port_control(source, peripheral, role)
                    .map(|control| (control, direction)),
                _ => None,
            })
            .collect()
    }

    /// A controller's control on the port this device drives, if that is the
    /// controller plugged there.
    fn port_control(
        &self,
        source: InputSource,
        peripheral: PeripheralId,
        role: ControlRole,
    ) -> Option<ControlId> {
        let port = self.source_port(source)?;
        (self.plugged(port) == Some(peripheral)).then(|| ControlId::port(port, role))
    }

    /// Whether this panel control is a switch left in a position rather than a
    /// button held down.
    fn latching(&self, role: ControlRole) -> bool {
        self.surfaces.as_ref().is_some_and(|surfaces| {
            surfaces
                .panel
                .iter()
                .any(|control| control.role == role && control.toggle().is_some())
        })
    }

    fn plugged(&self, port: PortId) -> Option<PeripheralId> {
        self.surfaces
            .as_ref()?
            .ports
            .iter()
            .find(|plugged| plugged.descriptor.port == port)?
            .plugged
    }

    /// What a press works, remembering the controls it holds down. A key or
    /// button winds at full rate; an analog input's own level says how hard.
    fn press(
        &mut self,
        source: InputSource,
        surface: Surface,
        input: &str,
    ) -> Option<app::Message> {
        if !analog_input(input) {
            self.set_wind(source, surface, input, 1.0);
        }
        let actuations = self.controls(source, surface, input);
        if actuations.is_empty() {
            return None;
        }
        let held: Vec<ControlId> = actuations
            .iter()
            .filter_map(|actuation| match actuation {
                Actuation::Hold(control) => Some(*control),
                Actuation::Flip(_) => None,
            })
            .collect();
        if !held.is_empty() {
            self.held.insert((source, surface, input.to_string()), held);
        }
        Some(app::Message::SetControl(actuations, true))
    }

    /// Lift whatever the matching press put down. An input that worked an
    /// emulator action, or only flipped a switch, held nothing and so releases
    /// nothing.
    fn release(
        &mut self,
        source: InputSource,
        surface: Surface,
        input: &str,
    ) -> Option<app::Message> {
        if !analog_input(input) {
            self.set_wind(source, surface, input, 0.0);
        }
        let held = self.held.remove(&(source, surface, input.to_string()))?;
        Some(app::Message::SetControl(
            held.into_iter().map(Actuation::Hold).collect(),
            false,
        ))
    }

    /// Lift everything the devices `keep` rejects are holding, and forget them.
    fn release_sources(&mut self, keep: impl Fn(InputSource) -> bool) -> Option<app::Message> {
        let mut lifted = Vec::new();
        self.held.retain(|(source, ..), held| {
            if keep(*source) {
                return true;
            }
            lifted.extend(held.iter().copied().map(Actuation::Hold));
            false
        });
        self.winding.retain(|(source, ..), _| keep(*source));
        (!lifted.is_empty()).then_some(app::Message::SetControl(lifted, false))
    }

    /// How hard this input is now winding whatever it is bound to; `0.0` stops
    /// it. The knobs are resolved at this moment, so a wind survives a
    /// reassignment the way a held control does.
    fn set_wind(&mut self, source: InputSource, surface: Surface, input: &str, magnitude: f32) {
        let key = (source, surface, input.to_string());
        let knobs = match magnitude > 0.0 {
            true => self.winds(source, surface, input),
            false => Vec::new(),
        };
        match knobs.is_empty() {
            true => {
                self.winding.remove(&key);
            }
            false => {
                self.winding.insert(key, Wind { knobs, magnitude });
            }
        }
    }

    /// Turn every knob its held inputs ask for by the elapsed time. Inputs
    /// winding the same knob the same way don't stack — the hardest push wins —
    /// and opposing winds cancel, as two squeezed triggers always have.
    fn wind(&mut self, dt: f32) -> Vec<app::Message> {
        let mut rates: HashMap<ControlId, [f32; 2]> = self
            .knobs
            .keys()
            .map(|&control| (control, [0.0; 2]))
            .collect();
        for wind in self.winding.values() {
            for &(control, direction) in &wind.knobs {
                let rate = rates.entry(control).or_default();
                let way = usize::from(direction == WindDirection::CounterClockwise);
                rate[way] = rate[way].max(wind.magnitude);
            }
        }

        rates
            .into_iter()
            .filter_map(|(control, [clockwise, counter])| {
                let knob = self.knobs.entry(control).or_default();
                knob.set_right(clockwise);
                knob.set_left(counter);
                knob.tick(dt)
                    .map(|position| app::Message::SetAxis(control, position))
            })
            .collect()
    }

    /// The one port this device drives: what it is assigned, or the machine's
    /// first port for a pad that has never been assigned one.
    fn source_port(&self, source: InputSource) -> Option<PortId> {
        match source {
            InputSource::Keyboard => Some(self.assignments.keyboard),
            InputSource::Gamepad(id) => self
                .assignments
                .gamepads
                .get(&id)
                .copied()
                .or_else(|| self.surfaces.as_ref().and_then(first_port)),
        }
    }
}

/// What a device works by pressing `input`: the controls it reaches, or the
/// emulator action bound to it.
fn press_message(source: InputSource, surface: Surface, input: &str) -> Option<app::Message> {
    let mut routing = ROUTING.lock().unwrap();
    if let Some(action) = routing.emulator_action(surface, input) {
        return Some(emulator_message(action));
    }
    routing.press(source, surface, input)
}

fn release_message(source: InputSource, surface: Surface, input: &str) -> Option<app::Message> {
    ROUTING.lock().unwrap().release(source, surface, input)
}

/// A press or release of the controls alone, for inputs that never work an
/// emulator action (an analog stick standing in for the d-pad).
fn control_message(
    source: InputSource,
    surface: Surface,
    input: &str,
    pressed: bool,
) -> Option<app::Message> {
    let mut routing = ROUTING.lock().unwrap();
    match pressed {
        true => routing.press(source, surface, input),
        false => routing.release(source, surface, input),
    }
}

/// Lift everything a device that has gone away was holding.
pub fn release_source(source: InputSource) -> Option<app::Message> {
    ROUTING
        .lock()
        .unwrap()
        .release_sources(|held| held != source)
}

/// Lift everything held by pads the host no longer reports — a rebuilt
/// subscription hands out fresh ids, and the old ones never see a release.
pub fn release_missing_pads(present: &[gilrs::GamepadId]) -> Option<app::Message> {
    ROUTING.lock().unwrap().release_sources(|held| match held {
        InputSource::Keyboard => true,
        InputSource::Gamepad(id) => present.contains(&id),
    })
}

/// The knob the pointer over the screen points: knob `index` of the port the
/// keyboard plays, while this system lets the pointer turn it.
pub fn pointer_knob(index: u8) -> Option<ControlId> {
    let routing = ROUTING.lock().unwrap();
    if !routing.pointer_drives_knob {
        return None;
    }
    let port = routing.source_port(InputSource::Keyboard)?;
    Some(ControlId::port(port, ControlRole::Knob(index)))
}

/// How far an analog input is pushed, which is how fast it winds.
fn set_analog_level(source: InputSource, input: &str, level: f32) {
    ROUTING
        .lock()
        .unwrap()
        .set_wind(source, Surface::Gamepad, input, level);
}

/// Advance every knob a held input is winding; each one that moved reports its
/// new position.
fn wind_tick(dt: f32) -> Vec<app::Message> {
    ROUTING.lock().unwrap().wind(dt)
}

/// Whether this input says how far it is pushed rather than only that it is
/// down: its winding follows that level, not its press.
fn analog_input(input: &str) -> bool {
    matches!(input, "LeftTrigger2" | "RightTrigger2") || stick_direction(input)
}

/// The stick direction names, and nothing else on the pad, start this way.
fn stick_direction(input: &str) -> bool {
    input.starts_with("LeftStick") || input.starts_with("RightStick")
}

fn emulator_message(action: EmulatorAction) -> app::Message {
    match action {
        EmulatorAction::Screenshot => app::Message::TakeScreenshot,
        EmulatorAction::ToggleFullscreen => app::Message::ToggleFullscreen,
        EmulatorAction::Pause => app::Message::TogglePause,
        EmulatorAction::SaveState => app::Message::SaveState,
        EmulatorAction::LoadState => app::Message::LoadState,
        EmulatorAction::ToggleRecording => app::Message::ToggleRecording,
        EmulatorAction::Replay => app::Message::Replay,
    }
}

pub fn event_handler(
    event: Event,
    _status: event::Status,
    _window: iced::window::Id,
) -> Option<app::Message> {
    match event {
        // Auto-repeat re-presses a key that never came up: a held control is
        // already down, and an emulator action would flap.
        Event::Keyboard(keyboard::Event::KeyPressed { key, repeat, .. }) if !repeat => {
            let key_str = key_to_string(&key)?;
            press_message(InputSource::Keyboard, Surface::Keyboard, &key_str)
        }
        Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
            let key_str = key_to_string(&key)?;
            release_message(InputSource::Keyboard, Surface::Keyboard, &key_str)
        }
        _ => None,
    }
}

/// During gamepad capture, only listen for Escape on the keyboard to cancel.
pub fn escape_cancel_handler(
    event: Event,
    _status: event::Status,
    _window: iced::window::Id,
) -> Option<app::Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
            if key == Key::Named(key::Named::Escape) {
                Some(app::Message::Settings(
                    super::settings::view::Message::CancelCapture,
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn capture_event_handler(
    event: Event,
    _status: event::Status,
    _window: iced::window::Id,
) -> Option<app::Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
            if key == Key::Named(key::Named::Escape) {
                Some(app::Message::Settings(
                    super::settings::view::Message::CancelCapture,
                ))
            } else if key == Key::Named(key::Named::Backspace)
                || key == Key::Named(key::Named::Delete)
            {
                Some(app::Message::Settings(
                    super::settings::view::Message::ClearBinding,
                ))
            } else {
                key_to_string(&key).map(|key_str| {
                    app::Message::Settings(super::settings::view::Message::CaptureBinding(key_str))
                })
            }
        }
        _ => None,
    }
}

/// How this pad is recognised in a later session.
fn identity_of(pad: &gilrs::Gamepad<'_>) -> GamepadIdentity {
    GamepadIdentity {
        uuid: pad.uuid(),
        name: pad.name().to_string(),
    }
}

/// Per-pad input state: which stick directions are latched down, and the lowest
/// trigger level seen (trigger-as-axis pads report either range).
#[derive(Default)]
struct PadState {
    stick: HashMap<&'static str, bool>,
    trigger_floor: [f32; 2],
}

/// An analog stick's axis: what each way past the deadzone is called, positive
/// value first, and the d-pad buttons it stands in for while nothing is bound
/// to its own names.
struct StickAxis {
    directions: [&'static str; 2],
    dpad: Option<[&'static str; 2]>,
}

fn stick_axis(axis: gilrs::Axis) -> Option<StickAxis> {
    let stick = match axis {
        gilrs::Axis::LeftStickX => StickAxis {
            directions: ["LeftStickRight", "LeftStickLeft"],
            dpad: Some(["DPadRight", "DPadLeft"]),
        },
        gilrs::Axis::LeftStickY => StickAxis {
            directions: ["LeftStickUp", "LeftStickDown"],
            dpad: Some(["DPadUp", "DPadDown"]),
        },
        gilrs::Axis::RightStickX => StickAxis {
            directions: ["RightStickRight", "RightStickLeft"],
            dpad: None,
        },
        gilrs::Axis::RightStickY => StickAxis {
            directions: ["RightStickUp", "RightStickDown"],
            dpad: None,
        },
        _ => return None,
    };
    Some(stick)
}

/// The trigger name a trigger-as-axis pad's axis reports.
fn trigger_axis(axis: gilrs::Axis) -> Option<&'static str> {
    match axis {
        gilrs::Axis::LeftZ => Some("LeftTrigger2"),
        gilrs::Axis::RightZ => Some("RightTrigger2"),
        _ => None,
    }
}

pub fn gamepad_subscription() -> Subscription<app::Message> {
    Subscription::run(|| {
        stream::channel(64, async |mut sender| {
            let mut gilrs = gilrs::Gilrs::new().unwrap();
            let mut pads: HashMap<gilrs::GamepadId, PadState> = HashMap::new();
            let mut last_tick = std::time::Instant::now();

            const DEADZONE: f32 = 0.5;

            // Pads already connected raise no event of their own. The ids
            // belong to this `Gilrs` alone, so a restarted subscription hands
            // over the whole roster rather than adding to a stale one.
            let roster: Vec<ConnectedPad> = gilrs
                .gamepads()
                .map(|(id, pad)| ConnectedPad {
                    id,
                    identity: identity_of(&pad),
                })
                .collect();
            let _ = sender.try_send(app::Message::GamepadRoster(roster));

            loop {
                while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
                    let source = InputSource::Gamepad(id);
                    match event {
                        gilrs::EventType::Connected => {
                            let identity = identity_of(&gilrs.gamepad(id));
                            let _ = sender.try_send(app::Message::GamepadConnected(id, identity));
                        }
                        gilrs::EventType::Disconnected => {
                            pads.remove(&id);
                            let _ = sender.try_send(app::Message::GamepadDisconnected(id));
                        }
                        gilrs::EventType::ButtonPressed(button, ..) => {
                            if let Some(name) = gamepad_button_to_string(button)
                                && let Some(message) =
                                    press_message(source, Surface::Gamepad, &name)
                            {
                                let _ = sender.try_send(message);
                            }
                        }
                        gilrs::EventType::ButtonReleased(button, ..) => {
                            if let Some(name) = gamepad_button_to_string(button)
                                && let Some(message) =
                                    release_message(source, Surface::Gamepad, &name)
                            {
                                let _ = sender.try_send(message);
                            }
                        }
                        // The trigger's squeeze depth: how fast it winds
                        // whatever knob it is bound to.
                        gilrs::EventType::ButtonChanged(
                            button @ (gilrs::Button::LeftTrigger2 | gilrs::Button::RightTrigger2),
                            value,
                            ..,
                        ) => {
                            if let Some(name) = gamepad_button_to_string(button) {
                                set_analog_level(source, &name, value);
                            }
                        }
                        gilrs::EventType::AxisChanged(axis, value, ..) => {
                            let pad = pads.entry(id).or_default();
                            // Trigger-as-axis pads: normalise against the
                            // lowest level seen (0..1 and -1..1 both occur).
                            if let Some(name) = trigger_axis(axis) {
                                let slot = usize::from(axis == gilrs::Axis::RightZ);
                                pad.trigger_floor[slot] = pad.trigger_floor[slot].min(value);
                                let depression = match pad.trigger_floor[slot] < 0.0 {
                                    true => (value + 1.0) / 2.0,
                                    false => value,
                                };
                                set_analog_level(source, name, depression);
                                continue;
                            }
                            // Each stick direction is an input of its own,
                            // winding by how far it is pushed; the left stick
                            // still stands in for the d-pad while nothing is
                            // bound to it.
                            let Some(stick) = stick_axis(axis) else {
                                continue;
                            };
                            let pushed = [value.max(0.0), (-value).max(0.0)];
                            for way in 0..2 {
                                let name = stick.directions[way];
                                set_analog_level(source, name, pushed[way]);

                                let now = pushed[way] > DEADZONE;
                                let held = pad.stick.entry(name).or_default();
                                if now == *held {
                                    continue;
                                }
                                *held = now;
                                let bound = ROUTING.lock().unwrap().bound(Surface::Gamepad, name);
                                let input = match bound {
                                    true => name,
                                    false => stick.dpad.map_or(name, |dpad| dpad[way]),
                                };
                                if let Some(message) =
                                    control_message(source, Surface::Gamepad, input, now)
                                {
                                    let _ = sender.try_send(message);
                                }
                            }
                        }
                        _ => {}
                    }
                }

                let dt = last_tick.elapsed().as_secs_f32();
                last_tick = std::time::Instant::now();
                for message in wind_tick(dt) {
                    let _ = sender.try_send(message);
                }

                smol::Timer::after(Duration::from_millis(4)).await;
            }
        })
    })
}

pub fn gamepad_capture_subscription() -> Subscription<app::Message> {
    Subscription::run(gamepad_capture_stream)
}

/// How far a trigger or stick must move to be captured as a binding, well past
/// the deadzone so a resting stick never claims a row.
const CAPTURE_THRESHOLD: f32 = 0.7;

fn gamepad_capture_stream() -> impl iced::futures::Stream<Item = app::Message> {
    stream::channel(64, async |mut sender| {
        let mut gilrs = gilrs::Gilrs::new().unwrap();
        loop {
            while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                // An analog input is captured on a decisive movement: the
                // triggers and each stick direction bind like any button.
                let captured = match event {
                    gilrs::EventType::ButtonPressed(button, ..) => gamepad_button_to_string(button),
                    gilrs::EventType::AxisChanged(axis, value, ..) => {
                        match (trigger_axis(axis), stick_axis(axis)) {
                            // A trigger reporting -1..1 rests at its low end,
                            // so only a squeeze towards the top captures.
                            (Some(trigger), _) => {
                                (value > CAPTURE_THRESHOLD).then(|| trigger.to_string())
                            }
                            (_, Some(stick)) => (value.abs() > CAPTURE_THRESHOLD)
                                .then(|| stick.directions[usize::from(value < 0.0)].to_string()),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(input) = captured {
                    let _ = sender.try_send(app::Message::Settings(
                        super::settings::view::Message::CaptureBinding(input),
                    ));
                }
            }
            smol::Timer::after(Duration::from_millis(4)).await;
        }
    })
}

/// Convert an iced keyboard Key to a stable string for storage/comparison.
fn key_to_string(key: &Key) -> Option<String> {
    match key.as_ref() {
        Key::Named(named) => Some(format!("{named:?}")),
        Key::Character(c) => Some(c.to_string()),
        Key::Unidentified => None,
    }
}

/// Human-readable display name for a key binding string.
pub fn display_key_name(s: &str) -> &str {
    match s {
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "Control" => "Ctrl",
        other => other,
    }
}

/// Human-readable display name for a gamepad binding string (Xbox/Steam Deck layout).
pub fn display_gamepad_name(s: &str) -> &str {
    match s {
        "South" => "A",
        "East" => "B",
        "West" => "X",
        "North" => "Y",
        "Start" => "Menu ≡",
        "Select" => "View ⧉",
        "DPadUp" => "D-Pad ↑",
        "DPadDown" => "D-Pad ↓",
        "DPadLeft" => "D-Pad ←",
        "DPadRight" => "D-Pad →",
        "LeftTrigger" => "LB",
        "RightTrigger" => "RB",
        "LeftTrigger2" => "LT",
        "RightTrigger2" => "RT",
        "LeftThumb" => "L3",
        "RightThumb" => "R3",
        "LeftStickUp" => "L-Stick ↑",
        "LeftStickDown" => "L-Stick ↓",
        "LeftStickLeft" => "L-Stick ←",
        "LeftStickRight" => "L-Stick →",
        "RightStickUp" => "R-Stick ↑",
        "RightStickDown" => "R-Stick ↓",
        "RightStickLeft" => "R-Stick ←",
        "RightStickRight" => "R-Stick →",
        other => other,
    }
}

fn gamepad_button_to_string(button: gilrs::Button) -> Option<String> {
    let s = match button {
        gilrs::Button::South => "South",
        gilrs::Button::East => "East",
        gilrs::Button::West => "West",
        gilrs::Button::North => "North",
        gilrs::Button::Start => "Start",
        gilrs::Button::Select => "Select",
        gilrs::Button::DPadUp => "DPadUp",
        gilrs::Button::DPadDown => "DPadDown",
        gilrs::Button::DPadLeft => "DPadLeft",
        gilrs::Button::DPadRight => "DPadRight",
        gilrs::Button::LeftTrigger => "LeftTrigger",
        gilrs::Button::RightTrigger => "RightTrigger",
        gilrs::Button::LeftTrigger2 => "LeftTrigger2",
        gilrs::Button::RightTrigger2 => "RightTrigger2",
        gilrs::Button::LeftThumb => "LeftThumb",
        gilrs::Button::RightThumb => "RightThumb",
        _ => return None,
    };
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::settings::{Surface, default_system};
    use crate::app::system::Platform;
    use missingno_session::PluggedPort;
    use missingno_vcs::debug::{JOYSTICK, KEYPAD, LEFT_PORT, PADDLES, PORTS, RIGHT_PORT};

    /// A VCS with these peripherals in its two jacks, its default keyboard
    /// bindings, and every device on the left jack.
    fn vcs(plugged: [PeripheralId; 2]) -> Routing {
        Routing {
            system_keyboard: default_system(Platform::AtariVcs, Surface::Keyboard),
            surfaces: Some(ControlSurfaces {
                ports: PORTS
                    .iter()
                    .zip(plugged)
                    .map(|(descriptor, peripheral)| PluggedPort {
                        descriptor: *descriptor,
                        plugged: Some(peripheral),
                    })
                    .collect(),
                integrated: &[],
                panel: missingno_vcs::debug::PANEL_CONTROLS,
            }),
            ..Routing::default()
        }
    }

    fn pressed(routing: &Routing, key: &str) -> Vec<Actuation> {
        let mut worked = routing.controls(InputSource::Keyboard, Surface::Keyboard, key);
        worked.sort_by_key(|actuation| format!("{actuation:?}"));
        worked
    }

    #[test]
    fn a_key_reaches_only_the_plugged_peripheral_of_the_assigned_port() {
        let routing = vcs([JOYSTICK; 2]);
        // "x" is the joystick's Fire and a paddle button, and the bindings name
        // neither jack: only the controller plugged into the keyboard's own
        // port answers.
        assert_eq!(
            pressed(&routing, "x"),
            vec![Actuation::Hold(ControlId::port(
                LEFT_PORT,
                ControlRole::Action(0)
            ))]
        );
        assert!(pressed(&routing, "1").is_empty());
    }

    #[test]
    fn reassigning_the_keyboard_moves_which_port_answers() {
        let mut routing = vcs([JOYSTICK; 2]);
        routing.assignments.keyboard = RIGHT_PORT;
        assert_eq!(
            pressed(&routing, "x"),
            vec![Actuation::Hold(ControlId::port(
                RIGHT_PORT,
                ControlRole::Action(0)
            ))]
        );
    }

    #[test]
    fn a_keypad_key_fires_once_the_keypad_is_in_that_jack() {
        let routing = vcs([KEYPAD, JOYSTICK]);
        assert_eq!(
            pressed(&routing, "1"),
            vec![Actuation::Hold(ControlId::port(
                LEFT_PORT,
                ControlRole::Key(0)
            ))]
        );
    }

    #[test]
    fn the_console_panel_answers_whatever_port_a_device_drives() {
        let mut routing = vcs([JOYSTICK; 2]);
        routing.assignments.keyboard = RIGHT_PORT;
        assert_eq!(
            pressed(&routing, "Enter"),
            vec![Actuation::Hold(ControlId::panel(ControlRole::Reset))]
        );
    }

    /// The controls a `SetControl` message works, and at what level.
    fn worked(message: Option<app::Message>) -> Option<(Vec<Actuation>, bool)> {
        match message? {
            app::Message::SetControl(actuations, pressed) => Some((actuations, pressed)),
            other => panic!("expected a control message, got {other:?}"),
        }
    }

    #[test]
    fn a_release_lifts_the_port_its_press_held_not_the_one_now_assigned() {
        let mut routing = vcs([JOYSTICK; 2]);
        let fire = |port| Actuation::Hold(ControlId::port(port, ControlRole::Action(0)));

        let press = routing.press(InputSource::Keyboard, Surface::Keyboard, "x");
        assert_eq!(worked(press), Some((vec![fire(LEFT_PORT)], true)));

        // The keyboard moves to the other jack with the key still down.
        routing.assignments.keyboard = RIGHT_PORT;
        let release = routing.release(InputSource::Keyboard, Surface::Keyboard, "x");
        assert_eq!(worked(release), Some((vec![fire(LEFT_PORT)], false)));

        // Nothing is left holding: a second release works nothing.
        assert!(
            routing
                .release(InputSource::Keyboard, Surface::Keyboard, "x")
                .is_none()
        );
    }

    #[test]
    fn a_press_that_only_flips_a_switch_releases_nothing() {
        let mut routing = vcs([JOYSTICK; 2]);
        routing
            .system_keyboard
            .insert(ControlSlot::Panel(ControlRole::Toggle(2)), "t".to_string());
        assert_eq!(
            worked(routing.press(InputSource::Keyboard, Surface::Keyboard, "t")),
            Some((vec![Actuation::Flip(ControlRole::Toggle(2))], true))
        );
        assert!(
            routing
                .release(InputSource::Keyboard, Surface::Keyboard, "t")
                .is_none()
        );
    }

    // The path a disconnected pad takes, worked here through the keyboard: no
    // `gilrs::GamepadId` can be built without a live gilrs context.
    #[test]
    fn a_device_going_away_lifts_what_it_was_holding() {
        let mut routing = vcs([JOYSTICK; 2]);
        let gone = InputSource::Keyboard;

        routing.press(gone, Surface::Keyboard, "x");
        assert_eq!(
            worked(routing.release_sources(|source| source != gone)),
            Some((
                vec![Actuation::Hold(ControlId::port(
                    LEFT_PORT,
                    ControlRole::Action(0)
                ))],
                false
            ))
        );
        assert!(routing.release_sources(|source| source != gone).is_none());
    }

    /// A VCS with `plugged` in its jacks and a key winding the paddle each way.
    fn winding_vcs(plugged: [PeripheralId; 2]) -> Routing {
        let mut routing = vcs(plugged);
        for (direction, key) in [
            (WindDirection::Clockwise, "e"),
            (WindDirection::CounterClockwise, "q"),
        ] {
            routing.system_keyboard.insert(
                ControlSlot::Wind {
                    peripheral: PADDLES,
                    role: ControlRole::Knob(0),
                    direction,
                },
                key.to_string(),
            );
        }
        routing
    }

    /// Where a tick left the knob, if it moved one.
    fn wound(messages: Vec<app::Message>, knob: ControlId) -> Option<f32> {
        match messages.as_slice() {
            [] => None,
            [app::Message::SetAxis(control, position)] if *control == knob => Some(*position),
            other => panic!("expected the knob to be wound, got {other:?}"),
        }
    }

    #[test]
    fn a_held_key_winds_the_knob_of_the_paddle_in_the_port_it_plays() {
        let knob = ControlId::port(LEFT_PORT, ControlRole::Knob(0));
        let mut routing = winding_vcs([PADDLES, JOYSTICK]);

        routing.press(InputSource::Keyboard, Surface::Keyboard, "e");
        let clockwise = wound(routing.wind(0.1), knob).expect("the knob turns");
        assert!(clockwise > 0.5, "clockwise winds up the range");

        // The key comes up and the knob stays where it was left.
        routing.release(InputSource::Keyboard, Surface::Keyboard, "e");
        assert_eq!(wound(routing.wind(0.1), knob), None);

        // The other way winds back down.
        routing.press(InputSource::Keyboard, Surface::Keyboard, "q");
        let counter = wound(routing.wind(0.1), knob).expect("the knob turns");
        assert!(counter < clockwise);
    }

    #[test]
    fn a_wind_slot_answers_only_where_the_paddle_is_plugged() {
        let mut routing = winding_vcs([JOYSTICK; 2]);
        routing.press(InputSource::Keyboard, Surface::Keyboard, "e");
        assert!(
            routing.wind(0.1).is_empty(),
            "nothing to wind with no paddle in the port"
        );

        // The paddle is in the other jack: the port this device plays decides,
        // the same way a controller's buttons do.
        let mut routing = winding_vcs([JOYSTICK, PADDLES]);
        routing.press(InputSource::Keyboard, Surface::Keyboard, "e");
        assert!(routing.wind(0.1).is_empty());

        routing.assignments.keyboard = RIGHT_PORT;
        routing.press(InputSource::Keyboard, Surface::Keyboard, "e");
        assert!(
            wound(
                routing.wind(0.1),
                ControlId::port(RIGHT_PORT, ControlRole::Knob(0))
            )
            .is_some()
        );
    }

    #[test]
    fn a_bound_latching_switch_flips_rather_than_being_held() {
        let mut routing = vcs([JOYSTICK; 2]);
        routing
            .system_keyboard
            .insert(ControlSlot::Panel(ControlRole::Toggle(2)), "t".to_string());
        assert_eq!(
            pressed(&routing, "t"),
            vec![Actuation::Flip(ControlRole::Toggle(2))]
        );
    }
}
