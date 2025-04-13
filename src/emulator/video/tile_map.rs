use core::{fmt, ops::RangeInclusive};

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TileMapRegion {
    Map9800,
    Map9c00,
}

impl TileMapRegion {
    pub fn range(self) -> RangeInclusive<u16> {
        match self {
            Self::Map9800 => 0x9800..=0x9bff,
            Self::Map9c00 => 0x9c00..=0x9fff,
        }
    }
}

impl fmt::Display for TileMapRegion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Map9800 => write!(f, "9800-9bff"),
            Self::Map9c00 => write!(f, "9c00-9fff"),
        }
    }
}
