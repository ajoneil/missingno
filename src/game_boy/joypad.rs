use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub struct Joypad {
    read_buttons: bool,
    read_dpad: bool,

    pressed_buttons: HashSet<Button>,
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Button {
    Start,
    Select,
    A,
    B,
    DirectionalPad(DirectionalPad),
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DirectionalPad {
    Up,
    Down,
    Left,
    Right,
}

impl Joypad {
    const UNUSED: u8 = 0b1100_0000;
    const READ_BUTTONS: u8 = 0b0010_0000;
    const READ_DPAD: u8 = 0b0001_0000;
    const START_DOWN: u8 = 0b0000_1000;
    const SELECT_UP: u8 = 0b0000_0100;
    const B_LEFT: u8 = 0b0000_0010;
    const A_RIGHT: u8 = 0b0000_0001;
    const NONE_PRESSED: u8 = 0xf;

    pub fn new() -> Self {
        Self {
            read_buttons: false,
            read_dpad: false,
            pressed_buttons: HashSet::new(),
        }
    }

    pub fn read_register(&self) -> u8 {
        // Bits are weirdly inverted for joypad
        let mut value = Self::UNUSED | Self::NONE_PRESSED;

        if self.read_buttons {
            if self.pressed_buttons.contains(&Button::Start) {
                value ^= Self::START_DOWN;
            }
            if self.pressed_buttons.contains(&Button::Select) {
                value ^= Self::SELECT_UP;
            }
            if self.pressed_buttons.contains(&Button::B) {
                value ^= Self::B_LEFT;
            }
            if self.pressed_buttons.contains(&Button::A) {
                value ^= Self::A_RIGHT;
            }
        } else {
            value |= Self::READ_BUTTONS;
        }

        if self.read_dpad {
            if self
                .pressed_buttons
                .contains(&Button::DirectionalPad(DirectionalPad::Down))
            {
                value ^= Self::START_DOWN;
            }
            if self
                .pressed_buttons
                .contains(&Button::DirectionalPad(DirectionalPad::Up))
            {
                value ^= Self::SELECT_UP;
            }
            if self
                .pressed_buttons
                .contains(&Button::DirectionalPad(DirectionalPad::Left))
            {
                value ^= Self::B_LEFT;
            }
            if self
                .pressed_buttons
                .contains(&Button::DirectionalPad(DirectionalPad::Right))
            {
                value ^= Self::A_RIGHT;
            }
        } else {
            value |= Self::READ_DPAD;
        }

        value
    }

    pub fn write_register(&mut self, value: u8) {
        self.read_buttons = value & Self::READ_BUTTONS == 0;
        self.read_dpad = value & Self::READ_DPAD == 0;
    }

    pub fn press_button(&mut self, button: Button) {
        self.pressed_buttons.insert(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.pressed_buttons.remove(&button);
    }
}
