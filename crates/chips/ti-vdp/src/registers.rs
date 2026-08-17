//! What the eight write-only registers mean: the mode bits, the table
//! bases R2-R6 select, the sprite geometry R1 carries, and R7's colours.

use crate::Vdp;

pub(crate) mod r0 {
    pub const M3: u8 = 0x02;
}

pub(crate) mod r1 {
    pub const RAM_16K: u8 = 0x80;
    pub const DISPLAY_ENABLE: u8 = 0x40;
    pub const INTERRUPT_ENABLE: u8 = 0x20;
    pub const M1: u8 = 0x10;
    pub const M2: u8 = 0x08;
    pub const SIZE_16: u8 = 0x02;
    pub const MAG: u8 = 0x01;
}

/// The M1/M2/M3 selection, the community-documented combinations included.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    GraphicsI,
    GraphicsII,
    Multicolor,
    Text,
    BitmapText,
    BitmapMulticolor,
    TextMulticolor,
}

impl Mode {
    /// Whether the mode fetches on the text grid — 40 six-pixel cells —
    /// rather than the 32-cell grid.
    pub(crate) fn text_grid(self) -> bool {
        matches!(self, Mode::Text | Mode::BitmapText | Mode::TextMulticolor)
    }
}

/// A pattern's row within its generator table: eight bytes per name.
pub(crate) fn pattern_row(name: u16, row: u16) -> u16 {
    name * 8 + row
}

impl Vdp {
    pub fn registers(&self) -> &[u8; 8] {
        &self.registers
    }

    pub fn mode(&self) -> Mode {
        let m1 = self.registers[1] & r1::M1 != 0;
        let m2 = self.registers[1] & r1::M2 != 0;
        let m3 = self.registers[0] & r0::M3 != 0;
        match (m1, m2, m3) {
            (false, false, false) => Mode::GraphicsI,
            (false, false, true) => Mode::GraphicsII,
            (false, true, false) => Mode::Multicolor,
            (true, false, false) => Mode::Text,
            (true, false, true) => Mode::BitmapText,
            (false, true, true) => Mode::BitmapMulticolor,
            (true, true, _) => Mode::TextMulticolor,
        }
    }

    pub(crate) fn display_enabled(&self) -> bool {
        self.registers[1] & r1::DISPLAY_ENABLE != 0
    }

    /// Whether R1 addresses the full 16K; 4K mode permutes the address pins.
    pub(crate) fn ram_16k(&self) -> bool {
        self.registers[1] & r1::RAM_16K != 0
    }

    /// R7's low nibble: the backdrop every transparent pixel falls through
    /// to, and the only plane that reaches the border.
    pub fn backdrop(&self) -> u8 {
        self.registers[7] & 0x0F
    }

    /// R7's high nibble: the text-family foreground.
    pub(crate) fn text_colour(&self) -> u8 {
        self.registers[7] >> 4
    }

    pub fn name_table_base(&self) -> u16 {
        (self.registers[2] as u16 & 0x0F) * 0x400
    }

    /// The pattern generator base R4 selects, outside the bitmap family.
    pub fn pattern_table_base(&self) -> u16 {
        (self.registers[4] as u16 & 0x07) * 0x800
    }

    /// The colour table base R3 selects in Graphics I, where one byte colours
    /// a group of eight patterns.
    pub fn colour_table_base(&self) -> u16 {
        self.registers[3] as u16 * 0x40
    }

    /// R3's AND mask over a bitmap-family fetch offset (silicon:
    /// gii-mask-pattern, gii-mask-colour).
    pub(crate) fn bitmap_mask(&self) -> u16 {
        ((self.registers[3] as u16 & 0x7F) << 6) | 0x3F
    }

    /// R4's high base bit: the half of DRAM a bitmap-family pattern fetch
    /// lands in.
    fn bitmap_half(&self) -> u16 {
        ((self.registers[4] as u16) & 0x04) << 11
    }

    /// Graphics II's two fetches for a tile row `offset` — pattern byte then
    /// colour byte. R3's mask governs both; R4 contributes only the pattern
    /// half select.
    pub fn graphics_ii_cells(&self, offset: u16) -> (u8, u8) {
        let mask = self.bitmap_mask();
        let colour_half = ((self.registers[3] as u16) & 0x80) << 6;
        (
            self.vram_cell(self.bitmap_half() | (offset & mask)),
            self.vram_cell(colour_half | (offset & mask)),
        )
    }

    /// The pattern-table base a bitmap-family third selects: the half from
    /// R4's high base bit, the second and third tables gated by its low bits.
    pub(crate) fn bitmap_third_table(&self, third: u16) -> u16 {
        let table = match third {
            1 if self.registers[4] & 0x01 != 0 => 1,
            2 if self.registers[4] & 0x02 != 0 => 2,
            _ => 0,
        };
        self.bitmap_half() + table * 0x800
    }

    pub fn sprite_attribute_base(&self) -> u16 {
        (self.registers[5] as u16 & 0x7F) * 0x80
    }

    pub fn sprite_pattern_base(&self) -> u16 {
        (self.registers[6] as u16 & 0x07) * 0x800
    }

    /// Whether R1's SIZE bit selects 16×16 sprites — four consecutive
    /// generators — rather than 8×8.
    pub fn sprites_16x16(&self) -> bool {
        self.registers[1] & r1::SIZE_16 != 0
    }

    /// Whether R1's MAG bit doubles every sprite pixel.
    pub fn magnified(&self) -> bool {
        self.registers[1] & r1::MAG != 0
    }
}
