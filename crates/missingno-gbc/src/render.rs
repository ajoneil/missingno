//! The CGB attribute-aware tile-map render, shared by the debugger panes and
//! the headless HTTP endpoints. The DMG path lives in `missingno-gb`.

use missingno_gb::ppu::{
    memory::VramView, types::control::Control, types::palette::Palette, types::tiles::TileMapId,
};

use crate::BgAttribute;

/// CGB pre-render: each cell's attribute byte in bank 1 selects its palette,
/// tile bank, and flips.
pub fn tile_map_rgba_cgb(
    vram: &dyn VramView,
    tile_map_id: TileMapId,
    control: Control,
    bg_palettes: &[Palette; 8],
) -> Vec<u8> {
    let tile_map = vram.bank(0).tile_map(tile_map_id);
    let attributes = vram.bank(1).tile_map(tile_map_id);
    let mut pixels = Vec::with_capacity(256 * 256 * 4);

    for tile_row in 0..32 {
        for pixel_y in 0..8 {
            for tile_col in 0..32 {
                let map_tile_index = tile_map.get_tile(tile_col, tile_row);
                let attribute = BgAttribute(attributes.get_tile(tile_col, tile_row).0);
                let (block, mapped_index) = control.tile_address_mode().tile(map_tile_index);
                let tile = vram
                    .bank(attribute.tile_bank())
                    .tile_block(block)
                    .tile(mapped_index);
                let palette = &bg_palettes[attribute.palette() as usize];
                let y = if attribute.flip_y() {
                    7 - pixel_y
                } else {
                    pixel_y
                };

                for pixel_x in 0..8 {
                    let x = if attribute.flip_x() {
                        7 - pixel_x
                    } else {
                        pixel_x
                    };
                    let color = palette.color(tile.pixel(x, y));
                    pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        }
    }

    pixels
}
