use super::joypad::Button;

pub struct Recording {
    input: Vec<InputEvent>,
}

#[derive(Clone)]
pub struct InputEvent {
    frame: u64,
    input: Input,
}

#[derive(Clone)]
pub enum Input {
    Press(Button),
    Release(Button),
}

impl Recording {
    pub fn new() -> Self {
        Self { input: Vec::new() }
    }

    pub fn events(&self) -> &[InputEvent] {
        &self.input
    }

    pub fn record(&mut self, frame: u64, input: Input) {
        self.input.push(InputEvent { frame, input });
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
