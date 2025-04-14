use core::fmt;

#[derive(Copy, Clone)]
pub struct TileMap {
    pub data: [u8; 0x400],
}

impl TileMap {
    pub fn new() -> Self {
        Self { data: [0; 0x400] }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct TileMapId(pub u8);

impl fmt::Display for TileMapId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tile Map #{}", self.0)
    }
}
