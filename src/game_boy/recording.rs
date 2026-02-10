use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use super::joypad::Button;

#[derive(Serialize, Deserialize)]
pub struct Recording {
    rom: Rom,
    initial_state: InitialState,
    input: Vec<InputEvent>,
}

#[derive(Serialize, Deserialize)]
struct Rom {
    title: String,
    checksum: u16,
}

#[derive(Serialize, Deserialize)]
pub enum InitialState {
    FreshBoot,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InputEvent {
    frame: u64,
    input: Input,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum Input {
    Press(Button),
    Release(Button),
}

impl Recording {
    pub fn new(title: String, checksum: u16) -> Self {
        Self {
            rom: Rom { title, checksum },
            initial_state: InitialState::FreshBoot,
            input: Vec::new(),
        }
    }

    pub fn record(&mut self, frame: u64, input: Input) {
        self.input.push(InputEvent { frame, input });
    }

    pub fn save(&self, path: &Path) {
        let config = ron::ser::PrettyConfig::default();
        if let Ok(data) = ron::ser::to_string_pretty(self, config) {
            let _ = fs::write(path, data);
        }
    }
}
