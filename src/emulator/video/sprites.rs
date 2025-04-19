use core::fmt;

use bitflags::bitflags;

use crate::emulator::video::tiles::TileIndex;

#[derive(Clone, Copy)]
pub struct Sprite {
    pub position: Position,
    pub tile: TileIndex,
    pub attributes: Attributes,
}

impl Sprite {
    pub fn new() -> Self {
        Self {
            position: Position {
                x_plus_8: 0,
                y_plus_16: 0,
            },
            tile: TileIndex(0),
            attributes: Attributes::empty(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Position {
    pub x_plus_8: u8,
    pub y_plus_16: u8,
}

impl Position {
    pub fn on_screen_x(&self) -> bool {
        (1..168).contains(&self.x_plus_8)
    }

    pub fn on_screen_y(&self, size: SpriteSize) -> bool {
        let min = match size {
            SpriteSize::Single => 9,
            SpriteSize::Double => 1,
        };

        (min..160).contains(&self.y_plus_16)
    }

    pub fn on_line(&self, line: u8, size: SpriteSize) -> bool {
        let first_line = self.y_plus_16 as i16 - 16;
        let line_after = first_line + size.height() as i16;

        (first_line..line_after).contains(&(line as i16))
    }
}

#[derive(Clone, Copy)]
pub struct Attributes(pub u8);

bitflags! {
    impl Attributes: u8 {
        const PRIORITY = 0b1000_0000;
        const FLIP_Y = 0b0100_0000;
        const FLIP_X = 0b0010_0000;
        const PALETTE = 0b0001_0000;
        const REST = 0b0000_1111;
    }
}

#[derive(PartialEq, Eq)]
pub enum Priority {
    Sprite,
    BackgroundAndWindow,
}

#[allow(dead_code)]
pub enum Palette {
    Palette0,
    Palette1,
}

#[allow(dead_code)]
impl Attributes {
    pub fn priority(&self) -> Priority {
        if self.contains(Attributes::PRIORITY) {
            Priority::BackgroundAndWindow
        } else {
            Priority::Sprite
        }
    }

    pub fn flip_y(&self) -> bool {
        self.contains(Attributes::FLIP_Y)
    }

    pub fn flip_x(&self) -> bool {
        self.contains(Attributes::FLIP_X)
    }

    pub fn palette(&self) -> Palette {
        if self.contains(Attributes::PALETTE) {
            Palette::Palette1
        } else {
            Palette::Palette0
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct SpriteId(pub u8);

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum SpriteSize {
    Single,
    Double,
}

impl SpriteSize {
    pub fn height(&self) -> u8 {
        match self {
            SpriteSize::Single => 8,
            SpriteSize::Double => 16,
        }
    }
}

impl fmt::Display for SpriteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpriteSize::Single => write!(f, "Single (8 x 8)"),
            SpriteSize::Double => write!(f, "Double (8 x 16)"),
        }
    }
}
