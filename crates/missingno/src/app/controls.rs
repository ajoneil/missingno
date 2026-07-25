use std::sync::Mutex;
use std::time::Duration;

use iced::{
    Event, Subscription, event,
    keyboard::{self, Key, key},
    stream,
};

use missingno_core::system::ControlRole;

use crate::app;
use crate::app::settings::{Action, Bindings};

/// Current keyboard bindings, updated from settings.
static KEYBOARD_BINDINGS: Mutex<Option<Bindings>> = Mutex::new(None);
/// Current gamepad bindings, updated from settings.
static GAMEPAD_BINDINGS: Mutex<Option<Bindings>> = Mutex::new(None);

/// Call when settings change to update the bindings used by event handlers.
pub fn update_bindings(keyboard: &Bindings, gamepad: &Bindings) {
    *KEYBOARD_BINDINGS.lock().unwrap() = Some(keyboard.clone());
    *GAMEPAD_BINDINGS.lock().unwrap() = Some(gamepad.clone());
}

pub fn event_handler(
    event: Event,
    _status: event::Status,
    _window: iced::window::Id,
) -> Option<app::Message> {
    let guard = KEYBOARD_BINDINGS.lock().unwrap();
    let bindings = guard.as_ref()?;
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
            let key_str = key_to_string(&key)?;
            press_message(&bindings.find_actions(&key_str))
        }
        Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
            let key_str = key_to_string(&key)?;
            let roles = control_roles(&bindings.find_actions(&key_str));
            (!roles.is_empty()).then(|| app::Message::SetControl(roles, false))
        }
        _ => None,
    }
}

/// The control roles among a key's bound actions.
fn control_roles(actions: &[Action]) -> Vec<ControlRole> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Control(role) => Some(*role),
            _ => None,
        })
        .collect()
}

/// What a key's bound actions do on press: its control roles in one message,
/// or the single emulator action it works.
fn press_message(actions: &[Action]) -> Option<app::Message> {
    let roles = control_roles(actions);
    if !roles.is_empty() {
        return Some(app::Message::SetControl(roles, true));
    }
    actions.first().copied().map(action_to_press_message)
}

/// Convert an emulator action press into the appropriate app message.
fn action_to_press_message(action: Action) -> app::Message {
    match action {
        Action::Control(role) => app::Message::SetControl(vec![role], true),
        // Emulator actions → dedicated messages
        Action::Screenshot => app::Message::TakeScreenshot,
        Action::ToggleFullscreen => app::Message::ToggleFullscreen,
        Action::Pause => app::Message::TogglePause,
        Action::SaveState => app::Message::SaveState,
        Action::LoadState => app::Message::LoadState,
        Action::ToggleRecording => app::Message::ToggleRecording,
        Action::Replay => app::Message::Replay,
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

pub fn gamepad_subscription() -> Subscription<app::Message> {
    Subscription::run(|| {
        stream::channel(64, async |mut sender| {
            let mut gilrs = gilrs::Gilrs::new().unwrap();

            let mut stick_left = false;
            let mut stick_right = false;
            let mut stick_up = false;
            let mut stick_down = false;
            let mut paddle = missingno_iced::PaddleWind::new();
            let mut trigger_floor = [0.0f32; 2];
            let mut last_tick = std::time::Instant::now();

            const DEADZONE: f32 = 0.5;

            loop {
                while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                    let guard = GAMEPAD_BINDINGS.lock().unwrap();
                    let Some(bindings) = guard.as_ref() else {
                        continue;
                    };
                    match event {
                        gilrs::EventType::ButtonPressed(button, ..) => {
                            if let Some(button_str) = gamepad_button_to_string(button)
                                && let Some(message) =
                                    press_message(&bindings.find_actions(&button_str))
                            {
                                let _ = sender.try_send(message);
                            }
                        }
                        gilrs::EventType::ButtonReleased(button, ..) => {
                            if let Some(button_str) = gamepad_button_to_string(button) {
                                let roles = control_roles(&bindings.find_actions(&button_str));
                                if !roles.is_empty() {
                                    let _ = sender.try_send(app::Message::SetControl(roles, false));
                                }
                            }
                        }
                        // An unbound analog trigger winds the paddle
                        // (differential rate: right up, left down); a trigger
                        // the user bound to an action keeps that binding.
                        gilrs::EventType::ButtonChanged(
                            button @ (gilrs::Button::LeftTrigger2 | gilrs::Button::RightTrigger2),
                            value,
                            ..,
                        ) => {
                            let bound = gamepad_button_to_string(button)
                                .is_some_and(|name| bindings.find_action(&name).is_some());
                            if !bound {
                                match button {
                                    gilrs::Button::LeftTrigger2 => paddle.set_left(value),
                                    _ => paddle.set_right(value),
                                }
                            }
                        }
                        // Trigger-as-axis pads: normalise against the lowest
                        // level seen (0..1 and -1..1 ranges both occur).
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
                        gilrs::EventType::AxisChanged(axis, value, ..) => match axis {
                            gilrs::Axis::LeftStickX => {
                                let now_right = value > DEADZONE;
                                let now_left = value < -DEADZONE;

                                if now_right != stick_right {
                                    stick_right = now_right;
                                    let _ = sender.try_send(app::Message::SetControl(
                                        vec![ControlRole::Right],
                                        stick_right,
                                    ));
                                }
                                if now_left != stick_left {
                                    stick_left = now_left;
                                    let _ = sender.try_send(app::Message::SetControl(
                                        vec![ControlRole::Left],
                                        stick_left,
                                    ));
                                }
                            }
                            gilrs::Axis::LeftStickY => {
                                let now_up = value > DEADZONE;
                                let now_down = value < -DEADZONE;

                                if now_up != stick_up {
                                    stick_up = now_up;
                                    let _ = sender.try_send(app::Message::SetControl(
                                        vec![ControlRole::Up],
                                        stick_up,
                                    ));
                                }
                                if now_down != stick_down {
                                    stick_down = now_down;
                                    let _ = sender.try_send(app::Message::SetControl(
                                        vec![ControlRole::Down],
                                        stick_down,
                                    ));
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }

                let dt = last_tick.elapsed().as_secs_f32();
                last_tick = std::time::Instant::now();
                if let Some(position) = paddle.tick(dt) {
                    let _ = sender.try_send(app::Message::SetAxis(ControlRole::Knob(0), position));
                }

                smol::Timer::after(Duration::from_millis(4)).await;
            }
        })
    })
}

pub fn gamepad_capture_subscription() -> Subscription<app::Message> {
    Subscription::run(gamepad_capture_stream)
}

fn gamepad_capture_stream() -> impl iced::futures::Stream<Item = app::Message> {
    stream::channel(64, async |mut sender| {
        let mut gilrs = gilrs::Gilrs::new().unwrap();
        loop {
            while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                if let gilrs::EventType::ButtonPressed(button, ..) = event
                    && let Some(s) = gamepad_button_to_string(button)
                {
                    let _ = sender.try_send(app::Message::Settings(
                        super::settings::view::Message::CaptureBinding(s),
                    ));
                }
            }
            smol::Timer::after(Duration::from_millis(4)).await;
        }
    })
}

/// Convert an iced keyboard Key to a stable string for storage/comparison.
pub fn key_to_string(key: &Key) -> Option<String> {
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
        "Enter" => "Enter",
        "Shift" => "Shift",
        "Space" => "Space",
        "Tab" => "Tab",
        "Backspace" => "Backspace",
        "Control" => "Ctrl",
        "Alt" => "Alt",
        "F11" => "F11",
        "F12" => "F12",
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
