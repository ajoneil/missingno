//! Tile-map rendering shared by the debugger panes and the headless HTTP
//! endpoints. The DMG path lives here; the CGB attribute-aware path lives
//! beside the colour model in `missingno-gbc`.

use crate::ppu::{
    memory::VramBank, types::control::Control, types::palette::Palette, types::tiles::TileMapId,
};

/// Pre-render a 32×32 tile map as 256×256 RGBA bytes.
pub fn tile_map_rgba(
    vram: &VramBank,
    tile_map_id: TileMapId,
    control: Control,
    palette: &Palette,
) -> Vec<u8> {
    let tile_map = vram.tile_map(tile_map_id);
    let mut pixels = Vec::with_capacity(256 * 256 * 4);

    for tile_row in 0..32 {
        for pixel_y in 0..8 {
            for tile_col in 0..32 {
                let map_tile_index = tile_map.get_tile(tile_col, tile_row);
                let (block, mapped_index) = control.tile_address_mode().tile(map_tile_index);
                let tile = vram.tile_block(block).tile(mapped_index);

                for pixel_x in 0..8 {
                    let color = palette.color(tile.pixel(pixel_x, pixel_y));
                    pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        }
    }

    pixels
}
