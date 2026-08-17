use super::types::palette::PaletteIndex;

pub const NUM_SCANLINES: u8 = 144;
pub const PIXELS_PER_LINE: u8 = 160;

/// The DMG-reference greyscale byte for each shade, lightest first.
const GREYSCALE_BYTE: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

/// What a framebuffer cell holds where nothing has been drawn: white on the
/// DMG's shade indices, the white grey on the CGB's colours.
pub trait ScreenPixel: Copy {
    const BLANK: Self;
}

impl ScreenPixel for PaletteIndex {
    const BLANK: Self = PaletteIndex(0);
}

/// Double-buffered LCD screen. Heap-allocated to keep `Screen` cheap to move through message enums.
#[derive(Clone, Debug)]
pub struct Screen<P = PaletteIndex> {
    front: Box<Framebuffer<P>>,
    back: Box<Framebuffer<P>>,
}

impl<P: ScreenPixel> Default for Screen<P> {
    fn default() -> Self {
        Self {
            front: Box::new(Framebuffer::default()),
            back: Box::new(Framebuffer::default()),
        }
    }
}

impl<P: ScreenPixel> Screen<P> {
    pub fn pixel(&self, x: u8, y: u8) -> P {
        self.front.pixels[y as usize][x as usize]
    }

    /// Every displayed pixel in row-major order.
    pub fn pixels(&self) -> impl Iterator<Item = P> + '_ {
        (0..NUM_SCANLINES).flat_map(move |y| (0..PIXELS_PER_LINE).map(move |x| self.pixel(x, y)))
    }

    pub fn draw_pixel(&mut self, x: u8, y: u8, pixel: P) {
        self.back.pixels[y as usize][x as usize] = pixel;
    }

    /// Swap back→front and clear back. Returns true for callers tracking `new_screen`.
    pub fn present(&mut self) -> bool {
        std::mem::swap(&mut self.front, &mut self.back);
        self.back.clear();
        true
    }

    pub fn blank(&mut self) {
        self.front.clear();
        self.back.clear();
    }

    pub fn front(&self) -> &Framebuffer<P> {
        &self.front
    }

    /// The displayed buffer, for a caller seeding it from a save state's
    /// screenshot in its own pixel format.
    pub fn front_mut(&mut self) -> &mut Framebuffer<P> {
        &mut self.front
    }
}

impl Screen {
    /// Seed the displayed (front) buffer from a row-major slice of shade indices
    /// — restoring a save state's screenshot so the first frame after a restore
    /// matches the saved display. The back buffer stays cleared for the next draw.
    pub fn restore_front(&mut self, shades: &[u8]) {
        for y in 0..NUM_SCANLINES as usize {
            for x in 0..PIXELS_PER_LINE as usize {
                if let Some(&shade) = shades.get(y * PIXELS_PER_LINE as usize + x) {
                    self.front.pixels[y][x] = PaletteIndex(shade & 3);
                }
            }
        }
    }

    /// The displayed buffer as flat greyscale bytes (160 × 144), each shade on
    /// the DMG reference ramp the shade-pattern screenshots compare against.
    pub fn to_greyscale_bytes(&self) -> Vec<u8> {
        self.pixels()
            .map(|pixel| GREYSCALE_BYTE[pixel.0 as usize])
            .collect()
    }
}

impl crate::ScreenBuffer for Screen {
    type Pixel = PaletteIndex;
    fn draw_pixel(&mut self, x: u8, y: u8, pixel: PaletteIndex) {
        Screen::draw_pixel(self, x, y, pixel);
    }
    fn present(&mut self) -> bool {
        Screen::present(self)
    }
    fn blank(&mut self) {
        Screen::blank(self);
    }
    fn restore(&mut self, bytes: &[u8]) {
        self.restore_front(bytes);
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Framebuffer<P = PaletteIndex> {
    pub pixels: [[P; PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
}

impl<P: ScreenPixel> Framebuffer<P> {
    /// Clears in place — assigning `Framebuffer::default()` by value puts a
    /// screen-sized temporary on the stack of every caller it inlines into.
    fn clear(&mut self) {
        self.pixels.fill([P::BLANK; PIXELS_PER_LINE as usize]);
    }
}

impl<P: ScreenPixel> Default for Framebuffer<P> {
    fn default() -> Self {
        Self {
            pixels: [[P::BLANK; PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
        }
    }
}
