//! CGB color LCD framebuffer.
//!
//! Stores 15-bit RGB555 pixels packed as `0b_bbbbb_ggggg_rrrrr` — the format
//! the CGB PPU emits (`CgbPpu::resolve` → `Color555`) and CRAM (BCPD/OCPD) holds.
//! [`GREYSCALE`] remains the DMG-reference grey ramp that `to_greyscale_bytes`
//! reverse-maps for the shade-pattern screenshot tests.
//!
//! `missingno_gb::sgb::Rgb555` is the same 15-bit packing for the SGB; the two
//! stay separate because CGB and SGB apply different display gamma.

use missingno_gb::ScreenBuffer;

pub const NUM_SCANLINES: u8 = 144;
pub const PIXELS_PER_LINE: u8 = 160;

/// A 15-bit RGB555 color, packed `0b_bbbbb_ggggg_rrrrr` (5 bits per
/// channel), as stored in CGB palette RAM.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Color555(pub u16);

impl Color555 {
    /// A neutral grey at the given 5-bit intensity (`0..=31`).
    pub const fn grey(level: u8) -> Self {
        let l = level as u16;
        Self(l | (l << 5) | (l << 10))
    }

    pub const fn red(self) -> u8 {
        (self.0 & 0x1F) as u8
    }
}

/// Placeholder shade→RGB555 mapping used while the CGB color pipeline
/// doesn't exist yet. The four greys are the canonical GB grey levels
/// (0x7FFF/0x56B5/0x294A/0x0000) and reverse-map exactly to the DMG
/// reference bytes in [`Screen::to_greyscale_bytes`], so the existing
/// greyscale screenshot references still match.
pub const GREYSCALE: [Color555; 4] = [
    Color555::grey(31),
    Color555::grey(21),
    Color555::grey(10),
    Color555::grey(0),
];

/// The DMG-reference greyscale byte for each [`GREYSCALE`] level, indexed
/// by shade — the single source of truth shared with `to_greyscale_bytes`.
const GREYSCALE_BYTE: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

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
    pub fn pixel(&self, x: u8, y: u8) -> Color555 {
        self.front.pixels[y as usize][x as usize]
    }

    pub fn front(&self) -> &Framebuffer {
        &self.front
    }

    /// Read the current front buffer as a flat greyscale byte buffer
    /// (160 × 144 = 23040 bytes), reverse-mapping each pixel to its DMG shade so
    /// shade-pattern references match independent of the palette tint: the
    /// placeholder/full-CGB greys, then the DMG-compat boot palette, else the
    /// 5→8-bit expansion of the red channel.
    pub fn to_greyscale_bytes(&self) -> Vec<u8> {
        (0..NUM_SCANLINES)
            .flat_map(|y| {
                (0..PIXELS_PER_LINE).map(move |x| {
                    let c = self.pixel(x, y);
                    match GREYSCALE.iter().position(|&grey| grey == c) {
                        Some(shade) => GREYSCALE_BYTE[shade],
                        None => match crate::dmg_compat_shade(c) {
                            Some(shade) => GREYSCALE_BYTE[shade as usize],
                            None => (c.red() << 3) | (c.red() >> 2),
                        },
                    }
                })
            })
            .collect()
    }
}

impl ScreenBuffer for Screen {
    type Pixel = Color555;

    fn draw_pixel(&mut self, x: u8, y: u8, pixel: Color555) {
        self.back.pixels[y as usize][x as usize] = pixel;
    }

    fn present(&mut self) -> bool {
        std::mem::swap(&mut self.front, &mut self.back);
        *self.back = Framebuffer::default();
        true
    }

    fn blank(&mut self) {
        *self.front = Framebuffer::default();
        *self.back = Framebuffer::default();
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Framebuffer {
    pub pixels: [[Color555; PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
}

impl Default for Framebuffer {
    fn default() -> Self {
        // A powered LCD with nothing drawn reads white, matching the DMG
        // screen's PaletteIndex(0). GREYSCALE[0] is the white grey.
        Self {
            pixels: [[GREYSCALE[0]; PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
        }
    }
}
