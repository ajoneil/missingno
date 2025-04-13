use crate::emulator::cpu::cycles::Cycles;

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
    current_line: u32,
    current_line_dots: u32,
}

impl PixelProcessingUnit {
    const NUM_SCANLINES: u32 = 144;
    const NUM_SCANLINES_BETWEEN: u32 = 10;
    const SCANLINE_TOTAL_DOTS: u32 = 456;
    const PREPARING_SCANLINE_DOTS: u32 = 80;
    const DRAWING_PIXELS_MIN_DOTS: u32 = 172;

    pub fn new() -> Self {
        Self {
            current_line: 0,
            current_line_dots: 0,
        }
    }

    pub fn current_line(&self) -> u8 {
        self.current_line as u8
    }

    pub fn mode(&self) -> Mode {
        if self.current_line > Self::NUM_SCANLINES {
            Mode::BetweenFrames
        } else {
            if self.current_line_dots < Self::PREPARING_SCANLINE_DOTS {
                Mode::PreparingScanline
            } else if self.current_line_dots
                < (Self::PREPARING_SCANLINE_DOTS + Self::DRAWING_PIXELS_MIN_DOTS)
            {
                Mode::DrawingPixels
            } else {
                Mode::FinishingScanline
            }
        }
    }

    pub fn step(&mut self, cycles: Cycles) {
        let mut remaining_dots = cycles.0 * 4;
        while remaining_dots > 0 {
            self.current_line_dots += remaining_dots;
            if self.current_line_dots > Self::SCANLINE_TOTAL_DOTS {
                remaining_dots = self.current_line_dots - Self::SCANLINE_TOTAL_DOTS;
                self.current_line_dots = 0;
                self.current_line += 1;
                if self.current_line > Self::NUM_SCANLINES + Self::NUM_SCANLINES_BETWEEN {
                    self.current_line = 0;
                }
            } else {
                remaining_dots = 0;
            }
        }
    }
}
