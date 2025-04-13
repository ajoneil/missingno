use bitflags::bitflags;

bitflags! {
    pub struct Control: u8 {
        const VIDEO_ENABLE                 = 0b10000000;
        const WINDOW_TILE_MAP              = 0b01000000;
        const WINDOW_ENABLE                = 0b00100000;
        const TILE_DATA                    = 0b00010000;
        const BACKGROUND_TILE_MAP          = 0b00001000;
        const SPRITE_SIZE                  = 0b00000100;
        const SPRITE_ENABLE                = 0b00000010;
        const BACKGROUND_AND_WINDOW_ENABLE = 0b00000001;
    }
}

impl Control {
    pub fn new() -> Self {
        Control::VIDEO_ENABLE & Control::TILE_DATA & Control::BACKGROUND_AND_WINDOW_ENABLE
    }
}
