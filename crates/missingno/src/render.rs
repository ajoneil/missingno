//! Tile-map and palette rendering shared by the debugger panes and the
//! headless HTTP endpoints.

use missingno_gb::ppu::{
    memory::{Vram, VramBank},
    types::control::Control,
    types::palette::Palette,
    types::tiles::TileMapId,
};
use missingno_gbc::BgAttribute;

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

/// CGB pre-render: each cell's attribute byte in bank 1 selects its palette,
/// tile bank, and flips.
pub fn tile_map_rgba_cgb(
    vram: &impl Vram,
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

/// The 8 corrected display palettes of one CGB palette RAM.
pub fn cram_palettes(color: impl Fn(u8, u8) -> missingno_gbc::screen::Color555) -> [Palette; 8] {
    std::array::from_fn(|palette| {
        Palette::new(std::array::from_fn(|index| {
            color(palette as u8, index as u8).to_corrected_rgb8()
        }))
    })
}
