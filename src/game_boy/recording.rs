use std::{fs, path::Path};

use nanoserde::{DeRon, SerRon};

use super::joypad::Button;

#[derive(SerRon, DeRon)]
pub struct Recording {
    rom: Rom,
    initial_state: InitialState,
    input: Vec<InputEvent>,
}

#[derive(SerRon, DeRon)]
struct Rom {
    title: String,
    checksum: u16,
}

#[derive(SerRon, DeRon)]
pub enum InitialState {
    FreshBoot,
}

#[derive(SerRon, DeRon, Clone)]
pub struct InputEvent {
    frame: u64,
    input: Input,
}

#[derive(SerRon, DeRon, Clone)]
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
        Self::deserialize_ron(&data).map_err(|e| e.to_string())
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
        let data = self.serialize_ron();
        let _ = fs::write(path, data);
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
