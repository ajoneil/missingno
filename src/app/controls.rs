use iced::{
    Event, event,
    keyboard::{self, Key, key},
};

use crate::{app, emulator::joypad};

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
