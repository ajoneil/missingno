use std::time::Duration;

use iced::{
    Event, Subscription, event,
    keyboard::{self, Key, key},
    stream,
};

use crate::{app, game_boy::joypad};

pub fn event_handler(
    event: Event,
    _status: event::Status,
    _window: iced::window::Id,
) -> Option<app::Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
            if let Some(button) = map_key(key) {
                Some(app::Message::PressButton(button).into())
            } else {
                None
            }
        }
        Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
            if let Some(button) = map_key(key) {
                Some(app::Message::ReleaseButton(button).into())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn map_key(key: Key) -> Option<joypad::Button> {
    Some(match key.as_ref() {
        Key::Named(key::Named::ArrowUp) => {
            joypad::Button::DirectionalPad(joypad::DirectionalPad::Up)
        }
        Key::Named(key::Named::ArrowDown) => {
            joypad::Button::DirectionalPad(joypad::DirectionalPad::Down)
        }
        Key::Named(key::Named::ArrowLeft) => {
            joypad::Button::DirectionalPad(joypad::DirectionalPad::Left)
        }
        Key::Named(key::Named::ArrowRight) => {
            joypad::Button::DirectionalPad(joypad::DirectionalPad::Right)
        }
        Key::Named(key::Named::Enter) => joypad::Button::Start,
        Key::Named(key::Named::Shift) => joypad::Button::Select,
        Key::Character("x") => joypad::Button::A,
        Key::Character("z") => joypad::Button::B,

        _ => return None,
    })
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
                    match event {
                        gilrs::EventType::ButtonPressed(button, ..) => {
                            if let Some(btn) = map_gamepad_button(button) {
                                let _ = sender.try_send(app::Message::PressButton(btn));
                            }
                        }
                        gilrs::EventType::ButtonReleased(button, ..) => {
                            if let Some(btn) = map_gamepad_button(button) {
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

fn map_gamepad_button(button: gilrs::Button) -> Option<joypad::Button> {
    Some(match button {
        gilrs::Button::South => joypad::Button::A,
        gilrs::Button::East => joypad::Button::B,
        gilrs::Button::Start => joypad::Button::Start,
        gilrs::Button::Select => joypad::Button::Select,
        gilrs::Button::DPadUp => joypad::Button::DirectionalPad(joypad::DirectionalPad::Up),
        gilrs::Button::DPadDown => joypad::Button::DirectionalPad(joypad::DirectionalPad::Down),
        gilrs::Button::DPadLeft => joypad::Button::DirectionalPad(joypad::DirectionalPad::Left),
        gilrs::Button::DPadRight => joypad::Button::DirectionalPad(joypad::DirectionalPad::Right),
        _ => return None,
    })
}
