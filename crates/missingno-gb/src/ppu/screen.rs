use super::types::palette::PaletteIndex;

pub const NUM_SCANLINES: u8 = 144;
pub const PIXELS_PER_LINE: u8 = 160;

/// Double-buffered LCD screen. Heap-allocated to keep `Screen` cheap to move through message enums.
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
    pub fn pixel(&self, x: u8, y: u8) -> PaletteIndex {
        self.front.pixels[y as usize][x as usize]
    }

    pub fn draw_pixel(&mut self, x: u8, y: u8, pixel: PaletteIndex) {
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
}

#[derive(Copy, Clone, Debug)]
pub struct Framebuffer {
    pub pixels: [[PaletteIndex; PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self {
            pixels: [[PaletteIndex(0); PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
        }
    }
}
