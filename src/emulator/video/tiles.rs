use core::fmt;

use super::palette::PaletteIndex;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct TileBlockId(pub u8);

impl fmt::Display for TileBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tile Block #{}", self.0)
    }
}

#[derive(Copy, Clone)]
pub struct TileBlock {
    pub data: [u8; 0x800],
}

impl TileBlock {
    pub fn new() -> Self {
        Self { data: [0; 0x800] }
    }

    pub fn tile(&self, index: TileIndex) -> Tile {
        let offset = index.0 as usize * 16;
        Tile {
            data: self.data[offset..offset + 16].try_into().unwrap(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct TileIndex(pub u8);

#[derive(Debug, Clone, Copy)]
pub struct Tile {
    data: [u8; 16],
}

impl Tile {
    pub fn pixel(&self, x: u8, y: u8) -> PaletteIndex {
        PaletteIndex(self.data[(y as usize * 2 + (x >> 3) as usize) & 0xF])
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TileAddressMode {
    Block2Block1,
    Block0Block1,
}

impl fmt::Display for TileAddressMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TileAddressMode::Block2Block1 => write!(f, "Blocks 2 & 1"),
            TileAddressMode::Block0Block1 => write!(f, "Blocks 0 & 1"),
        }
    }
}

// use rgb::RGBA8;

// use super::palette::Palette;

// #[derive(Debug)]
// pub struct Tile {
//     data: [u8; 16],
// }

// impl Tile {
//     pub fn new(data: [u8; 16]) -> Self {
//         Self { data }
//     }

//     pub fn line(&self, line_num: u8, palette: &Palette) -> [RGBA8; 8] {
//         [
//             self.pixel_color(0, line_num, palette),
//             self.pixel_color(1, line_num, palette),
//             self.pixel_color(2, line_num, palette),
//             self.pixel_color(3, line_num, palette),
//             self.pixel_color(4, line_num, palette),
//             self.pixel_color(5, line_num, palette),
//             self.pixel_color(6, line_num, palette),
//             self.pixel_color(7, line_num, palette),
//         ]
//     }

//     pub fn pixel_color(&self, x: u8, y: u8, palette: &Palette) -> RGBA8 {
//         palette.color(self.pixel_bits(x, y))
//     }

//     pub fn pixel_bits(&self, x: u8, y: u8) -> u8 {
//         let line_start = y * 2;

//         let low_byte = self.data[line_start as usize];
//         let high_byte = self.data[(line_start + 1) as usize];

//         let shift = 7 - x;

//         let low_bit = (low_byte >> shift) & 0b1;
//         let high_bit = (high_byte >> shift) & 0b1;

//         let out = (high_bit << 1) + low_bit;

//         out
//     }
// }
