use std::sync::Mutex;
use std::time::Duration;

use iced::{
    Event, Subscription, event,
    keyboard::{self, Key, key},
    stream,
};

use crate::app;
use crate::app::settings::{GbButton, KeyBindings};
use missingno_gb::joypad;

/// Current keyboard bindings, updated from settings.
static KEYBOARD_BINDINGS: Mutex<KeyBindings> = Mutex::new(KeyBindings::DEFAULT_KEYBOARD);
/// Current gamepad bindings, updated from settings.
static GAMEPAD_BINDINGS: Mutex<KeyBindings> = Mutex::new(KeyBindings::DEFAULT_GAMEPAD);

/// Call when settings change to update the bindings used by event handlers.
pub fn update_bindings(keyboard: &KeyBindings, gamepad: &KeyBindings) {
    *KEYBOARD_BINDINGS.lock().unwrap() = keyboard.clone();
    *GAMEPAD_BINDINGS.lock().unwrap() = gamepad.clone();
}

pub fn event_handler(
    event: Event,
    _status: event::Status,
    _window: iced::window::Id,
) -> Option<app::Message> {
    let bindings = KEYBOARD_BINDINGS.lock().unwrap();
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
            map_key(&key, &bindings).map(app::Message::PressButton)
        }
        Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
            map_key(&key, &bindings).map(app::Message::ReleaseButton)
        }
        _ => None,
    }
}

fn map_key(key: &Key, bindings: &KeyBindings) -> Option<joypad::Button> {
    let key_str = key_to_string(key)?;
    for gb_button in GbButton::ALL {
        if bindings.get(gb_button) == key_str {
            return Some(gb_button_to_joypad(gb_button));
        }
    }
    None
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
                    let bindings = GAMEPAD_BINDINGS.lock().unwrap().clone();
                    match event {
                        gilrs::EventType::ButtonPressed(button, ..) => {
                            if let Some(btn) = map_gamepad_button(button, &bindings) {
                                let _ = sender.try_send(app::Message::PressButton(btn));
                            }
                        }
                        gilrs::EventType::ButtonReleased(button, ..) => {
                            if let Some(btn) = map_gamepad_button(button, &bindings) {
                                let _ = sender.try_send(app::Message::ReleaseButton(btn));
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

fn map_gamepad_button(button: gilrs::Button, bindings: &KeyBindings) -> Option<joypad::Button> {
    let button_str = gamepad_button_to_string(button)?;
    for gb_button in GbButton::ALL {
        if bindings.get(gb_button) == button_str {
            return Some(gb_button_to_joypad(gb_button));
        }
    }
    None
}

fn gb_button_to_joypad(gb: GbButton) -> joypad::Button {
    match gb {
        GbButton::A => joypad::Button::A,
        GbButton::B => joypad::Button::B,
        GbButton::Start => joypad::Button::Start,
        GbButton::Select => joypad::Button::Select,
        GbButton::Up => joypad::Button::DirectionalPad(joypad::DirectionalPad::Up),
        GbButton::Down => joypad::Button::DirectionalPad(joypad::DirectionalPad::Down),
        GbButton::Left => joypad::Button::DirectionalPad(joypad::DirectionalPad::Left),
        GbButton::Right => joypad::Button::DirectionalPad(joypad::DirectionalPad::Right),
    }
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
