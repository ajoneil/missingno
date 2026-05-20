//! CGB color LCD framebuffer.
//!
//! Stores RGB pixels (15-bit hardware output is currently widened to
//! 8-bit per channel for the fixed greyscale palette). Until proper
//! CGB palette memory lands, the PPU emits a 2-bit shade index per
//! pixel and `GameBoyColor::apply_ppu_result` maps it through
//! [`GREYSCALE_PALETTE`] before drawing.

use rgb::RGB8;

pub const NUM_SCANLINES: u8 = 144;
pub const PIXELS_PER_LINE: u8 = 160;

/// Default shade-to-RGB mapping used while CGB palette memory and the
/// CGB-PPU palette path don't exist yet. Matches the DMG greyscale
/// reference (0xFF / 0xAA / 0x55 / 0x00) so the existing screenshot
/// test references work without re-rendering.
pub const GREYSCALE_PALETTE: [RGB8; 4] = [
    RGB8 { r: 0xFF, g: 0xFF, b: 0xFF },
    RGB8 { r: 0xAA, g: 0xAA, b: 0xAA },
    RGB8 { r: 0x55, g: 0x55, b: 0x55 },
    RGB8 { r: 0x00, g: 0x00, b: 0x00 },
];

#[derive(Clone, Debug)]
pub struct Screen {
    front: Box<Framebuffer>,
    back: Box<Framebuffer>,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            front: Box::new(Framebuffer::default()),
            back: Box::new(Framebuffer::default()),
        }
    }
}

impl Screen {
    pub fn pixel(&self, x: u8, y: u8) -> RGB8 {
        self.front.pixels[y as usize][x as usize]
    }

    pub fn draw_pixel(&mut self, x: u8, y: u8, pixel: RGB8) {
        self.back.pixels[y as usize][x as usize] = pixel;
    }

    /// Swap back→front and clear back. Returns true for callers tracking `new_screen`.
    pub fn present(&mut self) -> bool {
        std::mem::swap(&mut self.front, &mut self.back);
        *self.back = Framebuffer::default();
        true
    }

    pub fn blank(&mut self) {
        *self.front = Framebuffer::default();
        *self.back = Framebuffer::default();
    }

    pub fn front(&self) -> &Framebuffer {
        &self.front
    }

    /// Read the current front buffer as a flat greyscale byte buffer
    /// (160 × 144 = 23040 bytes, values 0x00-0xFF). Pixels are assumed
    /// to be R==G==B (true under the fixed greyscale palette); samples
    /// the red channel.
    pub fn to_greyscale_bytes(&self) -> Vec<u8> {
        (0..NUM_SCANLINES)
            .flat_map(|y| (0..PIXELS_PER_LINE).map(move |x| self.pixel(x, y).r))
            .collect()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Framebuffer {
    pub pixels: [[RGB8; PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self {
            pixels: [[RGB8 { r: 0xFF, g: 0xFF, b: 0xFF };
                PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
        }
    }
}
