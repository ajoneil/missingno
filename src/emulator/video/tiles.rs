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
        let low_byte = self.data[y as usize * 2];
        let high_byte = self.data[(y as usize * 2) + 1];
        let low_bit = (low_byte >> (7 - x)) & 0b1;
        let high_bit = (high_byte >> (7 - x)) & 0b1;
        PaletteIndex(high_bit << 1 | low_bit)
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
