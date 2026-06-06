//! CGB color LCD framebuffer.
//!
//! Stores 15-bit RGB555 pixels packed as `0b_bbbbb_ggggg_rrrrr` — the format
//! the CGB PPU emits (`CgbPpu::resolve` → `Color555`) and CRAM (BCPD/OCPD) holds.
//! [`GREYSCALE`] remains the DMG-reference grey ramp that `to_greyscale_bytes`
//! reverse-maps for the shade-pattern screenshot tests.
//!
//! `missingno_gb::sgb::Rgb555` is the same 15-bit packing for the SGB; the two
//! stay separate because CGB and SGB apply different display gamma.

use std::sync::OnceLock;

use missingno_gb::ScreenBuffer;
use rgb::RGB8;

pub const NUM_SCANLINES: u8 = 144;
pub const PIXELS_PER_LINE: u8 = 160;

/// 5-bit channel → 8-bit display response of the CGB LCD, as measured by
/// SameBoy (Core/display.c, the non-AGB curve).
const CGB_LCD_CURVE: [u8; 32] = [
    0, 6, 12, 20, 28, 36, 45, 56, 66, 76, 88, 100, 113, 125, 137, 149, 161, 172, 182, 192, 202,
    210, 218, 225, 232, 238, 243, 247, 250, 252, 254, 255,
];

/// Corrected green for each (green, blue) 5-bit pair: SameBoy's
/// modern-balanced green←blue bleed, `((g^1.6 * 3 + b^1.6) / 4)^(1/1.6)`
/// over the curved channels.
fn green_mix_table() -> &'static [[u8; 32]; 32] {
    static TABLE: OnceLock<[[u8; 32]; 32]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [[0; 32]; 32];
        for (green, row) in table.iter_mut().enumerate() {
            for (blue, mixed) in row.iter_mut().enumerate() {
                let g = (CGB_LCD_CURVE[green] as f64 / 255.0).powf(1.6);
                let b = (CGB_LCD_CURVE[blue] as f64 / 255.0).powf(1.6);
                *mixed = (((g * 3.0 + b) / 4.0).powf(1.0 / 1.6) * 255.0).round() as u8;
            }
        }
        table
    })
}

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

    pub const fn green(self) -> u8 {
        ((self.0 >> 5) & 0x1F) as u8
    }

    pub const fn blue(self) -> u8 {
        ((self.0 >> 10) & 0x1F) as u8
    }

    /// This colour as it appears on the CGB LCD, mapped to sRGB —
    /// SameBoy's modern-balanced correction.
    pub fn to_corrected_rgb8(self) -> RGB8 {
        RGB8::new(
            CGB_LCD_CURVE[self.red() as usize],
            green_mix_table()[self.green() as usize][self.blue() as usize],
            CGB_LCD_CURVE[self.blue() as usize],
        )
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

    /// Read the current front buffer as flat RGBA bytes (160 × 144 × 4, alpha
    /// 255), each pixel colour-corrected for display.
    pub fn to_corrected_rgba(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(NUM_SCANLINES as usize * PIXELS_PER_LINE as usize * 4);
        for y in 0..NUM_SCANLINES {
            for x in 0..PIXELS_PER_LINE {
                let c = self.pixel(x, y).to_corrected_rgb8();
                bytes.extend_from_slice(&[c.r, c.g, c.b, 255]);
            }
        }
        bytes
    }

    /// Read the current front buffer as flat RGB888 bytes (160 × 144 × 3),
    /// each 5-bit channel expanded to 8 bits — the colourised form compared
    /// against full-colour reference images.
    pub fn to_rgb_bytes(&self) -> Vec<u8> {
        let expand = |c: u8| (c << 3) | (c >> 2);
        (0..NUM_SCANLINES)
            .flat_map(|y| {
                (0..PIXELS_PER_LINE).flat_map(move |x| {
                    let c = self.pixel(x, y);
                    [expand(c.red()), expand(c.green()), expand(c.blue())]
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
        self.back.clear();
        true
    }

    fn blank(&mut self) {
        self.front.clear();
        self.back.clear();
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Framebuffer {
    pub pixels: [[Color555; PIXELS_PER_LINE as usize]; NUM_SCANLINES as usize],
}

impl Framebuffer {
    /// Clears in place — assigning `Framebuffer::default()` by value puts a
    /// screen-sized temporary on the stack of every caller it inlines into.
    fn clear(&mut self) {
        self.pixels.fill([GREYSCALE[0]; PIXELS_PER_LINE as usize]);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_endpoints() {
        assert_eq!(
            Color555(0x7FFF).to_corrected_rgb8(),
            RGB8::new(255, 255, 255)
        );
        assert_eq!(Color555(0).to_corrected_rgb8(), RGB8::new(0, 0, 0));
        // Equal green and blue skip the bleed: pure red stays pure.
        assert_eq!(Color555(0x001F).to_corrected_rgb8(), RGB8::new(255, 0, 0));
    }

    #[test]
    fn correction_bleeds_blue_into_green() {
        let c = Color555(20 << 10 | 10 << 5).to_corrected_rgb8();
        assert_eq!(c.b, CGB_LCD_CURVE[20]);
        assert!(c.g > CGB_LCD_CURVE[10] && c.g < CGB_LCD_CURVE[20]);
    }

    #[test]
    fn correction_is_monotonic_per_channel() {
        for level in 1..32u16 {
            let prev = Color555::grey((level - 1) as u8).to_corrected_rgb8();
            let next = Color555::grey(level as u8).to_corrected_rgb8();
            assert!(next.r > prev.r && next.g > prev.g && next.b > prev.b);
        }
    }
}
