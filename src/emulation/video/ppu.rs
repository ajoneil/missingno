#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Mode {
    BetweenFrames = 1,
    PreparingScanline = 2,
    DrawingPixels = 3,
    FinishingScanline = 0,
}

pub struct PixelProcessingUnit {
    mode: Mode,
    current_line: u8,
}

impl PixelProcessingUnit {
    pub fn new() -> Self {
        Self {
            mode: Mode::BetweenFrames,
            current_line: 0,
        }
    }

    pub fn current_line(&self) -> u8 {
        self.current_line
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
}
