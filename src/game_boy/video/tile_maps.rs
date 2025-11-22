use core::fmt;

use crate::game_boy::video::tiles::TileIndex;

#[derive(Copy, Clone)]
pub struct TileMap {
    pub data: [TileIndex; 0x400],
}

impl TileMap {
    pub fn new() -> Self {
        Self {
            data: [TileIndex(0); 0x400],
        }
    }

    pub fn get_tile(&self, x: u8, y: u8) -> TileIndex {
        self.data[y as usize * 32 + x as usize]
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct TileMapId(pub u8);

impl fmt::Display for TileMapId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tile Map #{}", self.0)
    }
}
