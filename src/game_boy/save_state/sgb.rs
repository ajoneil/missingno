use nanoserde::{DeRon, SerRon};

#[derive(SerRon, DeRon)]
pub struct SgbState {
    pub palettes: Vec<SgbPaletteState>,
    pub attribute_map: Vec<Vec<u8>>,
    pub system_palettes: Vec<SgbPaletteState>,
    pub attribute_files: Vec<Vec<Vec<u8>>>,
    pub mask_mode: u8,
    pub player_count: u8,
    pub current_player: u8,
    pub prev_p14_p15_both_low: bool,
}

#[derive(SerRon, DeRon)]
pub struct SgbPaletteState {
    pub colors: [u16; 4],
}
