use core::fmt;

use crate::emulator::{cpu::cycles::Cycles, interrupts::Interrupt};

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
    current_line_cycles: Cycles,
}

impl PixelProcessingUnit {
    const NUM_SCANLINES: u32 = 144;
    const NUM_SCANLINES_BETWEEN: u32 = 10;
    const SCANLINE_TOTAL_CYCLES: Cycles = Cycles(456 / 4);
    const PREPARING_SCANLINE_CYCLES: Cycles = Cycles(80 / 4);
    const DRAWING_PIXELS_MIN_CYCLES: Cycles = Cycles(172 / 4);

    pub fn new() -> Self {
        Self {
            current_line: 0,
            current_line_cycles: Cycles(0),
        }
    }

    pub fn current_line(&self) -> u8 {
        self.current_line as u8
    }

    pub fn mode(&self) -> Mode {
        if self.current_line > Self::NUM_SCANLINES {
            Mode::BetweenFrames
        } else {
            if self.current_line_cycles < Self::PREPARING_SCANLINE_CYCLES {
                Mode::PreparingScanline
            } else if self.current_line_cycles
                < (Self::PREPARING_SCANLINE_CYCLES + Self::DRAWING_PIXELS_MIN_CYCLES)
            {
                Mode::DrawingPixels
            } else {
                Mode::FinishingScanline
            }
        }
    }

    pub fn step(&mut self, cycles: Cycles) -> Option<Interrupt> {
        let mut interrupt = None;

        let mut remaining = cycles;
        while remaining > Cycles(0) {
            self.current_line_cycles += remaining;
            if self.current_line_cycles > Self::SCANLINE_TOTAL_CYCLES {
                remaining = self.current_line_cycles - Self::SCANLINE_TOTAL_CYCLES;
                self.current_line_cycles = Cycles(0);
                self.current_line += 1;

                if self.current_line > Self::NUM_SCANLINES + Self::NUM_SCANLINES_BETWEEN {
                    self.current_line = 0;
                } else if self.current_line == Self::NUM_SCANLINES {
                    interrupt = Some(Interrupt::VideoBetweenFrames);
                }
            } else {
                remaining = Cycles(0);
            }
        }

        interrupt
    }
}
