use super::types::palette::PaletteIndex;

pub const NUM_SCANLINES: u8 = 144;
pub const PIXELS_PER_LINE: u8 = 160;

/// Double-buffered LCD screen.
///
/// The **front buffer** holds the last completed frame (read by the
/// GUI, debugger, and screenshot comparisons). The **back buffer**
/// accumulates pixels from the PPU during the current frame.
///
/// Call [`present()`](Screen::present) at frame boundaries (VBlank or
/// LCD-off) to promote the back buffer to front and clear for the
/// next frame.
///
/// Framebuffers are heap-allocated to avoid stack overflow when
/// `Screen` is passed through deeply nested message enums.
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
    /// Read a pixel from the front buffer (the last completed frame).
    pub fn pixel(&self, x: u8, y: u8) -> PaletteIndex {
        self.front.pixels[y as usize][x as usize]
    }

    /// Write a pixel to the back buffer (the frame being drawn).
    pub fn draw_pixel(&mut self, x: u8, y: u8, pixel: PaletteIndex) {
        self.back.pixels[y as usize][x as usize] = pixel;
    }

    /// Promote the back buffer to front and clear the back buffer
    /// for the next frame. Returns true (convenience for callers
    /// tracking `new_screen`).
    pub fn present(&mut self) -> bool {
        std::mem::swap(&mut self.front, &mut self.back);
        *self.back = Framebuffer::default();
        true
    }

    /// Direct reference to the front buffer for bulk pixel reads
    /// (e.g. shader upload, screenshot comparison).
    pub fn front(&self) -> &Framebuffer {
        &self.front
    }
}

/// A single 160×144 framebuffer of palette indices.
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
