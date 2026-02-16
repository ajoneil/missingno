use super::palette::PaletteIndex;

pub const NUM_SCANLINES: u8 = 144;
pub const PIXELS_PER_LINE: u8 = 160;

#[derive(Copy, Clone, Debug)]
pub struct Screen {
    lines: [Line; NUM_SCANLINES as usize],
}

impl Screen {
    pub fn new() -> Self {
        Self {
            lines: [Line::new(); NUM_SCANLINES as usize],
        }
    }

    pub fn pixel(&self, x: u8, y: u8) -> PaletteIndex {
        self.lines[y as usize].pixels[x as usize]
    }

    pub fn set_pixel(&mut self, x: u8, y: u8, pixel: PaletteIndex) {
        self.lines[y as usize].pixels[x as usize] = pixel;
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Line {
    pixels: [PaletteIndex; PIXELS_PER_LINE as usize],
}

impl Line {
    pub fn new() -> Self {
        Self {
            pixels: [PaletteIndex(0); PIXELS_PER_LINE as usize],
        }
    }
}
