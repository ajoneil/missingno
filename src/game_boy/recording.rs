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

    pub fn load(path: &Path) -> Result<Self, String> {
        let data = fs::read_to_string(path).map_err(|e| e.to_string())?;
        ron::from_str(&data).map_err(|e| e.to_string())
    }

    pub fn rom_title(&self) -> &str {
        &self.rom.title
    }

    pub fn rom_checksum(&self) -> u16 {
        self.rom.checksum
    }

    pub fn events(&self) -> &[InputEvent] {
        &self.input
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

impl InputEvent {
    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn input(&self) -> &Input {
        &self.input
    }
}
