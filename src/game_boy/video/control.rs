use super::{sprites::SpriteSize, tile_maps::TileMapId, tiles::TileAddressMode};

use bitflags::bitflags;

bitflags! {
    #[derive(Copy, Clone)]
    pub struct ControlFlags: u8 {
        const VIDEO_ENABLE                 = 0b10000000;
        const WINDOW_TILE_MAP              = 0b01000000;
        const WINDOW_ENABLE                = 0b00100000;
        const TILE_ADDRESS_MODE            = 0b00010000;
        const BACKGROUND_TILE_MAP          = 0b00001000;
        const SPRITE_SIZE                  = 0b00000100;
        const SPRITE_ENABLE                = 0b00000010;
        const BACKGROUND_AND_WINDOW_ENABLE = 0b00000001;
    }
}

#[derive(Copy, Clone)]
pub struct Control(ControlFlags);

impl Default for Control {
    fn default() -> Self {
        Self::new(
            ControlFlags::VIDEO_ENABLE
                & ControlFlags::TILE_ADDRESS_MODE
                & ControlFlags::BACKGROUND_AND_WINDOW_ENABLE,
        )
    }
}

impl Control {
    pub fn new(flags: ControlFlags) -> Self {
        Self(flags)
    }

    pub fn bits(&self) -> u8 {
        self.0.bits()
    }

    pub fn video_enabled(&self) -> bool {
        self.0.contains(ControlFlags::VIDEO_ENABLE)
    }

    pub fn tile_address_mode(&self) -> TileAddressMode {
        if self.0.contains(ControlFlags::TILE_ADDRESS_MODE) {
            TileAddressMode::Block0Block1
        } else {
            TileAddressMode::Block2Block1
        }
    }

    pub fn background_and_window_enabled(&self) -> bool {
        self.0.contains(ControlFlags::BACKGROUND_AND_WINDOW_ENABLE)
    }

    pub fn background_tile_map(&self) -> TileMapId {
        if self.0.contains(ControlFlags::BACKGROUND_TILE_MAP) {
            TileMapId(1)
        } else {
            TileMapId(0)
        }
    }

    pub fn window_enabled(&self) -> bool {
        self.0.contains(ControlFlags::WINDOW_ENABLE)
    }

    pub fn window_tile_map(&self) -> TileMapId {
        if self.0.contains(ControlFlags::WINDOW_TILE_MAP) {
            TileMapId(1)
        } else {
            TileMapId(0)
        }
    }

    pub fn sprites_enabled(&self) -> bool {
        self.0.contains(ControlFlags::SPRITE_ENABLE)
    }

    pub fn sprite_size(&self) -> SpriteSize {
        if self.0.contains(ControlFlags::SPRITE_SIZE) {
            SpriteSize::Double
        } else {
            SpriteSize::Single
        }
    }
}
