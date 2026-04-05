use std::sync::Mutex;
use std::time::Duration;

use iced::{
    Event, Subscription, event,
    keyboard::{self, Key, key},
    stream,
};

use crate::app;
use crate::app::settings::{Action, Bindings};
use missingno_gb::joypad;

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
            let action = bindings.find_action(&key_str)?;
            Some(action_to_press_message(action))
        }
        Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
            let key_str = key_to_string(&key)?;
            let action = bindings.find_action(&key_str)?;
            // Only game buttons produce release messages
            if action.is_game_button() {
                Some(app::Message::ReleaseButton(action_to_joypad(action)?))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Convert an action press into the appropriate app message.
fn action_to_press_message(action: Action) -> app::Message {
    match action {
        // Game buttons → PressButton
        action if action.is_game_button() => {
            // unwrap is safe: we just checked is_game_button
            app::Message::PressButton(action_to_joypad(action).unwrap())
        }
        // Emulator actions → dedicated messages
        Action::Screenshot => app::Message::TakeScreenshot,
        Action::ToggleFullscreen => app::Message::ToggleFullscreen,
        Action::Pause => app::Message::TogglePause,
        _ => unreachable!(),
    }
}

/// Map a game button action to a joypad button. Returns None for non-game actions.
fn action_to_joypad(action: Action) -> Option<joypad::Button> {
    match action {
        Action::GbA => Some(joypad::Button::A),
        Action::GbB => Some(joypad::Button::B),
        Action::GbStart => Some(joypad::Button::Start),
        Action::GbSelect => Some(joypad::Button::Select),
        Action::GbUp => Some(joypad::Button::DirectionalPad(joypad::DirectionalPad::Up)),
        Action::GbDown => Some(joypad::Button::DirectionalPad(joypad::DirectionalPad::Down)),
        Action::GbLeft => Some(joypad::Button::DirectionalPad(joypad::DirectionalPad::Left)),
        Action::GbRight => Some(joypad::Button::DirectionalPad(
            joypad::DirectionalPad::Right,
        )),
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
                    super::settings_view::Message::CancelCapture,
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
                    super::settings_view::Message::CancelCapture,
                ))
            } else if key == Key::Named(key::Named::Backspace)
                || key == Key::Named(key::Named::Delete)
            {
                Some(app::Message::Settings(
                    super::settings_view::Message::ClearBinding,
                ))
            } else if let Some(key_str) = key_to_string(&key) {
                Some(app::Message::Settings(
                    super::settings_view::Message::CaptureBinding(key_str),
                ))
            } else {
                None
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

            const DEADZONE: f32 = 0.5;

            loop {
                while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
                    let guard = GAMEPAD_BINDINGS.lock().unwrap();
                    let Some(bindings) = guard.as_ref() else {
                        continue;
                    };
                    match event {
                        gilrs::EventType::ButtonPressed(button, ..) => {
                            if let Some(button_str) = gamepad_button_to_string(button) {
                                if let Some(action) = bindings.find_action(&button_str) {
                                    let _ = sender.try_send(action_to_press_message(action));
                                }
                            }
                        }
                        gilrs::EventType::ButtonReleased(button, ..) => {
                            if let Some(button_str) = gamepad_button_to_string(button) {
                                if let Some(action) = bindings.find_action(&button_str) {
                                    if action.is_game_button() {
                                        if let Some(btn) = action_to_joypad(action) {
                                            let _ =
                                                sender.try_send(app::Message::ReleaseButton(btn));
                                        }
                                    }
                                }
                            }
                        }
                        gilrs::EventType::AxisChanged(axis, value, ..) => match axis {
                            gilrs::Axis::LeftStickX => {
                                let now_right = value > DEADZONE;
                                let now_left = value < -DEADZONE;

                                if now_right != stick_right {
                                    stick_right = now_right;
                                    let btn = joypad::Button::DirectionalPad(
                                        joypad::DirectionalPad::Right,
                                    );
                                    let msg = if stick_right {
                                        app::Message::PressButton(btn)
                                    } else {
                                        app::Message::ReleaseButton(btn)
                                    };
                                    let _ = sender.try_send(msg);
                                }
                                if now_left != stick_left {
                                    stick_left = now_left;
                                    let btn = joypad::Button::DirectionalPad(
                                        joypad::DirectionalPad::Left,
                                    );
                                    let msg = if stick_left {
                                        app::Message::PressButton(btn)
                                    } else {
                                        app::Message::ReleaseButton(btn)
                                    };
                                    let _ = sender.try_send(msg);
                                }
                            }
                            gilrs::Axis::LeftStickY => {
                                let now_up = value > DEADZONE;
                                let now_down = value < -DEADZONE;

                                if now_up != stick_up {
                                    stick_up = now_up;
                                    let btn =
                                        joypad::Button::DirectionalPad(joypad::DirectionalPad::Up);
                                    let msg = if stick_up {
                                        app::Message::PressButton(btn)
                                    } else {
                                        app::Message::ReleaseButton(btn)
                                    };
                                    let _ = sender.try_send(msg);
                                }
                                if now_down != stick_down {
                                    stick_down = now_down;
                                    let btn = joypad::Button::DirectionalPad(
                                        joypad::DirectionalPad::Down,
                                    );
                                    let msg = if stick_down {
                                        app::Message::PressButton(btn)
                                    } else {
                                        app::Message::ReleaseButton(btn)
                                    };
                                    let _ = sender.try_send(msg);
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }
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
                if let gilrs::EventType::ButtonPressed(button, ..) = event {
                    if let Some(s) = gamepad_button_to_string(button) {
                        let _ = sender.try_send(app::Message::Settings(
                            super::settings_view::Message::CaptureBinding(s),
                        ));
                    }
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
