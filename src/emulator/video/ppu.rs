use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    BetweenFrames = 1,
    PreparingScanline = 2,
    DrawingPixels = 3,
    FinishingScanline = 0,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::BetweenFrames => write!(f, "Between Frames"),
            Mode::PreparingScanline => write!(f, "Preparing Scanline"),
            Mode::DrawingPixels => write!(f, "Drawing Pixels"),
            Mode::FinishingScanline => write!(f, "Finishing Scanline"),
        }
    }
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
