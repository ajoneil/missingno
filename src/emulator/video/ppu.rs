use core::fmt;
use std::cmp::min;

use crate::emulator::{
    cpu::cycles::Cycles,
    video::{
        PpuAccessible,
        screen::{self, Screen},
    },
};

use super::palette::PaletteIndex;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    BetweenFrames = 1,
    PreparingScanline = 2,
    DrawingPixels = 3,
    BetweenLines = 0,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::BetweenFrames => write!(f, "Between Frames"),
            Mode::PreparingScanline => write!(f, "Preparing Scanline"),
            Mode::DrawingPixels => write!(f, "Drawing Pixels"),
            Mode::BetweenLines => write!(f, "Between Scanlines"),
        }
    }
}

const SCANLINE_TOTAL_DOTS: u32 = 456;
const SCANLINE_PREPARING_DOTS: u32 = 80;
const BETWEEN_FRAMES_DOTS: u32 = SCANLINE_TOTAL_DOTS * 10;

pub struct Rendering {
    screen: Screen,
    line_number: u8,
    line_pixels_drawn: u8,
    line_dots: u32,
    line_penalty: u8,
}

impl Rendering {
    fn new() -> Self {
        Rendering {
            screen: Screen::new(),
            line_number: 0,
            line_pixels_drawn: 0,
            line_dots: 0,
            line_penalty: 12,
        }
    }

    fn mode(&self) -> Mode {
        if self.line_dots < SCANLINE_PREPARING_DOTS {
            Mode::PreparingScanline
        } else if self.line_pixels_drawn < screen::PIXELS_PER_LINE {
            Mode::DrawingPixels
        } else {
            Mode::BetweenLines
        }
    }

    fn render(&mut self, dots: u32, data: &PpuAccessible) -> Option<u32> {
        let mut remaining_dots = dots;

        while remaining_dots > 0 {
            if self.line_dots < SCANLINE_PREPARING_DOTS {
                let time_preparing = min(remaining_dots, SCANLINE_PREPARING_DOTS - self.line_dots);
                self.line_dots += time_preparing;
                remaining_dots -= time_preparing;
            } else {
                while self.line_pixels_drawn < screen::PIXELS_PER_LINE && remaining_dots > 0 {
                    if self.line_penalty > 0 {
                        self.line_penalty -= 1;
                    } else {
                        self.draw_pixel(data);
                    }

                    remaining_dots -= 1;
                }

                let time_waiting = min(remaining_dots, SCANLINE_TOTAL_DOTS - self.line_dots);
                if time_waiting > 0 {
                    self.line_dots += time_waiting;
                    remaining_dots -= time_waiting;
                }

                if self.line_dots == SCANLINE_TOTAL_DOTS {
                    self.line_number += 1;
                    self.line_dots = 0;
                    self.line_pixels_drawn = 0;

                    if self.line_number == screen::NUM_SCANLINES {
                        return Some(remaining_dots);
                    }
                }
            }
        }

        None
    }

    fn draw_pixel(&mut self, data: &PpuAccessible) {
        let x = self.line_pixels_drawn;
        let y = self.line_number;

        let mut pixel = PaletteIndex(0);

        if data.control.background_and_window_enabled() {
            let map = data.memory.tile_map(data.control.background_tile_map());
            let map_x = x + data.background_viewport.x % 0xff;
            let map_y = y + data.background_viewport.y % 0xff;
            let tile_index = map.get_tile(map_x / 8, map_y / 8);

            let (tile_block, mapped_index) = data.control.tile_address_mode().tile(tile_index);

            let tile = data.memory.tile_block(tile_block).tile(mapped_index);
            pixel = tile.pixel(map_x % 8, map_y % 8);
        };

        self.screen.set_pixel(x, y, pixel);

        self.line_pixels_drawn += 1;
    }
}

pub enum PixelProcessingUnit {
    Rendering(Rendering),
    BetweenFrames(u32),
}

impl PixelProcessingUnit {
    pub fn new() -> Self {
        Self::Rendering(Rendering::new())
    }

    pub fn current_line(&self) -> u8 {
        match self {
            PixelProcessingUnit::Rendering(Rendering { line_number, .. }) => *line_number,
            PixelProcessingUnit::BetweenFrames(dots) => {
                screen::NUM_SCANLINES + (dots / SCANLINE_TOTAL_DOTS) as u8
            }
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            PixelProcessingUnit::Rendering(rendering) => rendering.mode(),
            PixelProcessingUnit::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn step(&mut self, cycles: Cycles, data: &PpuAccessible) -> Option<Screen> {
        let mut remaining_dots: u32 = cycles.0 * 4;
        let mut screen = None;

        while remaining_dots > 0 {
            match self {
                PixelProcessingUnit::Rendering(rendering) => {
                    if let Some(remainder) = rendering.render(remaining_dots, data) {
                        screen = Some(rendering.screen.clone());
                        remaining_dots = remainder;
                        *self = PixelProcessingUnit::BetweenFrames(0);
                    } else {
                        remaining_dots = 0;
                    }
                }
                PixelProcessingUnit::BetweenFrames(dots) => {
                    let total = *dots + remaining_dots;
                    if total < BETWEEN_FRAMES_DOTS {
                        *dots = total;
                        remaining_dots = 0;
                    } else {
                        remaining_dots = total - BETWEEN_FRAMES_DOTS;
                        *self = PixelProcessingUnit::Rendering(Rendering::new());
                    }
                }
            };
        }

        screen
    }
}
