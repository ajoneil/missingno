use core::fmt;

#[derive(Clone, Copy)]
pub struct Sprite(pub [u8; 4]);

impl Sprite {
    pub fn new() -> Self {
        Self([0; 4])
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct SpriteId(pub u8);

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum SpriteSize {
    Single,
    Double,
}

impl fmt::Display for SpriteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpriteSize::Single => write!(f, "Single (8 x 8)"),
            SpriteSize::Double => write!(f, "Double (8 x 16)"),
        }
    }
}
