//! The Game Boy family's decode of VRAM/OAM into the system-agnostic graphics
//! vocabulary ([`missingno_core::graphics`]). The DMG builder lives here; the
//! CGB builder in `missingno-gbc` composes its own two-bank, CRAM-palette,
//! per-cell-attribute view over the shared decode helpers below.
//!
//! One builder serves both the live console (paused) and the per-vblank
//! snapshot (running) so the two agree by construction.

use missingno_core::graphics::{
    GraphicsView, MapEntry, Object, ObjectTable, PaletteSet, Tile, TileAtlas, TileMap, Viewport,
};
use missingno_core::inspect::Tone;

use crate::ppu::memory::{VramBank, VramView};
use crate::ppu::types::sprites::SpriteId;
use crate::ppu::types::tiles::{TileAddressMode, TileBlockId, TileIndex, TileMapId};

use super::inspection::PpuSource;

/// Addressable tiles per VRAM bank: three 128-tile blocks in address order.
pub const TILES_PER_BANK: usize = 384;

/// Decode one VRAM bank's 384 tiles (blocks 0, 1, 2 in address order) into an
/// atlas of 8×8, 2bpp palette indices. `palettes` says how a consumer colours
/// them — frontend shades on DMG, core-owned CRAM on CGB.
pub fn decode_bank_atlas(bank: &VramBank, label: String, palettes: PaletteSet) -> TileAtlas {
    let mut tiles = Vec::with_capacity(TILES_PER_BANK);
    for block in 0..3u8 {
        let tile_block = bank.tile_block(TileBlockId(block));
        for index in 0..128u8 {
            let tile = tile_block.tile(TileIndex(index));
            let mut indices = Vec::with_capacity(64);
            for y in 0..8 {
                for x in 0..8 {
                    indices.push(tile.pixel(x, y).0);
                }
            }
            tiles.push(Tile { indices });
        }
    }
    TileAtlas {
        label,
        tile_width: 8,
        tile_height: 8,
        depth_bits: 2,
        tiles,
        palettes,
    }
}

/// The atlas index (0..384) a raw tile-map index resolves to under `mode` —
/// applying the LCDC.4 addressing split, then flattening (block, index) into the
/// bank's address-order atlas.
pub fn resolved_atlas_index(mode: TileAddressMode, raw: TileIndex) -> u16 {
    let (block, index) = mode.tile(raw);
    block.0 as u16 * 128 + index.0 as u16
}

/// The BG/window viewports drawn over a map: the background rectangle at
/// (SCX,SCY) when this is the BG map, the window rectangle at (WX,WY) when it is
/// the window map. Shared by both models. Registers sampled as-of the instant.
pub fn map_viewports(ppu: &dyn PpuSource, map: TileMapId) -> Vec<Viewport> {
    let control = ppu.control();
    let mut viewports = Vec::new();
    if map == control.background_tile_map() {
        viewports.push(Viewport {
            label: "background (SCX,SCY)".into(),
            x: ppu.scx() as u16,
            y: ppu.scy() as u16,
            width: 160,
            height: 144,
            wraps: true,
            tone: Tone::Rendering,
        });
    }
    if map == control.window_tile_map() {
        viewports.push(Viewport {
            label: "window (WX,WY)".into(),
            x: ppu.wx() as u16,
            y: ppu.wy() as u16,
            width: 160,
            height: 144,
            wraps: false,
            tone: Tone::Scanning,
        });
    }
    viewports
}

/// The 40-entry OAM object table in screen space (the −8/−16 hardware offset
/// applied). `cgb` selects the per-entry CGB attributes (palette 0-7, VRAM
/// bank) versus DMG (frontend palette, no bank). Sprites always address the
/// 0x8000 (`Block0Block1`) pattern space, so `Object.tile` is a direct bank-0
/// atlas index; 8×16 composition (top `tile&FE`, bottom `tile|01`) is a
/// generic pane concern keyed off `object_height`.
pub fn object_table(ppu: &dyn PpuSource, cgb: bool) -> ObjectTable {
    let size = ppu.control().sprite_size();
    let objects = (0..40u8)
        .map(|index| {
            let sprite = ppu.sprite(SpriteId(index));
            let attributes = sprite.attributes;
            Object {
                index,
                x: sprite.position.x as i16 - 8,
                y: sprite.position.y as i16 - 16,
                tile: sprite.tile.0 as u16,
                on_screen: sprite.position.on_screen_x() && sprite.position.on_screen_y(size),
                palette: cgb.then(|| attributes.color_palette()),
                bank: cgb.then(|| attributes.vram_bank()),
                flip_x: attributes.flip_x(),
                flip_y: attributes.flip_y(),
                priority: attributes.behind_background(),
            }
        })
        .collect();
    ObjectTable {
        label: "OAM".into(),
        atlas: 0,
        object_height: size.height(),
        objects,
    }
}

/// The DMG graphics view: one VRAM bank as a frontend-shaded atlas, the two
/// tile maps (each cell's index resolved to an atlas index), and the OAM object
/// table.
pub fn dmg_graphics_view(ppu: &dyn PpuSource, vram: &dyn VramView) -> GraphicsView {
    let bank = vram.bank(0);
    let atlas = decode_bank_atlas(bank, "VRAM".into(), PaletteSet::FrontendShades);
    let mode = ppu.control().tile_address_mode();

    let maps = [TileMapId(0), TileMapId(1)]
        .into_iter()
        .map(|id| {
            let tile_map = bank.tile_map(id);
            let entries = (0..32u16)
                .flat_map(|row| (0..32u16).map(move |column| (column, row)))
                .map(|(column, row)| {
                    let raw = tile_map.get_tile(column as u8, row as u8);
                    MapEntry {
                        tile: resolved_atlas_index(mode, raw),
                        palette: None,
                        atlas: None,
                        flip_x: false,
                        flip_y: false,
                        priority: false,
                    }
                })
                .collect();
            TileMap {
                label: format!("Tile Map {}", id.0),
                columns: 32,
                rows: 32,
                atlas: 0,
                entries,
                viewports: map_viewports(ppu, id),
            }
        })
        .collect();

    GraphicsView {
        atlases: vec![atlas],
        maps,
        objects: Some(object_table(ppu, false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;
    use crate::{Console, Dmg};

    #[test]
    fn atlas_decodes_planar_bytes_to_indices() {
        // A recognisable tile at block 0 index 1 (flat VRAM 0x10): row 0 plane
        // low 0b1010_0000, high 0b0110_0000 → indices 1,2,3,0,... (high<<1|low
        // per pixel, MSB first).
        let mut data = vec![0u8; 0x2000];
        data[0x10] = 0b1010_0000;
        data[0x11] = 0b0110_0000;
        let bank = VramBank::from_bytes(&data);
        let atlas = decode_bank_atlas(&bank, "VRAM".into(), PaletteSet::FrontendShades);
        assert_eq!(atlas.tiles.len(), TILES_PER_BANK);
        assert!(matches!(atlas.palettes, PaletteSet::FrontendShades));
        assert_eq!(atlas.pixel(1, 0, 0), Some(1));
        assert_eq!(atlas.pixel(1, 1, 0), Some(2));
        assert_eq!(atlas.pixel(1, 2, 0), Some(3));
        assert_eq!(atlas.pixel(1, 3, 0), Some(0));
        // Every index stays within the 2bpp range.
        assert!(atlas.tiles.iter().all(|t| t.indices.iter().all(|&i| i < 4)));
    }

    #[test]
    fn resolved_index_applies_addressing_mode() {
        // Block0Block1: raw 0-255 maps to the identity atlas index.
        assert_eq!(
            resolved_atlas_index(TileAddressMode::Block0Block1, TileIndex(5)),
            5
        );
        assert_eq!(
            resolved_atlas_index(TileAddressMode::Block0Block1, TileIndex(200)),
            200
        );
        // Block2Block1: raw < 128 comes from block 2 (atlas 256..384).
        assert_eq!(
            resolved_atlas_index(TileAddressMode::Block2Block1, TileIndex(5)),
            261
        );
        assert_eq!(
            resolved_atlas_index(TileAddressMode::Block2Block1, TileIndex(200)),
            200
        );
    }

    #[test]
    fn view_shape_and_object_screen_offset() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let console = Console::<Dmg>::new(Cartridge::new(rom, None), None);
        let view = dmg_graphics_view(console.ppu(), console.vram());

        assert_eq!(view.atlases.len(), 1);
        assert_eq!(view.maps.len(), 2);
        for map in &view.maps {
            assert_eq!((map.columns, map.rows), (32, 32));
            assert_eq!(map.entries.len(), 32 * 32);
            assert!(
                map.entries
                    .iter()
                    .all(|e| e.palette.is_none() && e.atlas.is_none())
            );
        }
        let objects = view.objects.expect("OAM present");
        assert_eq!(objects.objects.len(), 40);
        // A default sprite (Y=0, X=0) lands at screen (−8, −16); DMG carries no
        // per-object palette or bank.
        assert_eq!((objects.objects[0].x, objects.objects[0].y), (-8, -16));
        assert!(objects.objects[0].palette.is_none());
        assert!(objects.objects[0].bank.is_none());
    }
}
