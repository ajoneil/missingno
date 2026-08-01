//! The debugger's graphics surfaces — the tile atlas, tile map, and object
//! table panes — rendered from the system-agnostic [`missingno_core::graphics`]
//! vocabulary a core decodes its video memory into. One pane body serves every
//! family that fills a [`GraphicsView`]; the Game Boy family is the only one
//! wired today (CGB included). Structure and palette indices come from the
//! view; the frontend's DMG palette / CGB CRAM colours come from the pane's
//! [`ConsoleColors`] context, exactly as the retired bespoke panes resolved them.
//!
//! [`GraphicsView`]: missingno_core::graphics::GraphicsView
//! [`ConsoleColors`]: crate::app::console::ConsoleColors

pub mod atlas;
pub mod map;
pub mod objects;

use missingno_core::graphics::TileAtlas;
use rgb::RGB8;

/// Tiles per row in an atlas texture — the retired Tiles pane's 16-wide block.
pub const ATLAS_COLUMNS: usize = 16;

/// RGBA bytes for a whole atlas laid out `columns` tiles wide (row-major over
/// pixels), each palette index coloured by `resolve`. Returns `(width, height,
/// pixels)`; the trailing partial tile-row is padded with blank cells.
pub fn atlas_texture(
    atlas: &TileAtlas,
    columns: usize,
    resolve: impl Fn(u8) -> RGB8,
) -> (u32, u32, Vec<u8>) {
    atlas_span_texture(atlas, 0, atlas.tile_count(), columns, resolve)
}

/// As [`atlas_texture`], but over the tile span `start..start + len` — the unit
/// a region draws. Tiles beyond the atlas pad with blank cells.
pub fn atlas_span_texture(
    atlas: &TileAtlas,
    start: usize,
    len: usize,
    columns: usize,
    resolve: impl Fn(u8) -> RGB8,
) -> (u32, u32, Vec<u8>) {
    let tile_w = atlas.tile_width as usize;
    let tile_h = atlas.tile_height as usize;
    let columns = columns.max(1);
    let rows = len.div_ceil(columns).max(1);
    let width = (columns * tile_w) as u32;
    let height = (rows * tile_h) as u32;

    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for tile_row in 0..rows {
        for pixel_y in 0..tile_h {
            for tile_col in 0..columns {
                let offset = tile_row * columns + tile_col;
                let tile = start + offset;
                for pixel_x in 0..tile_w {
                    let index = if offset < len {
                        atlas.pixel(tile, pixel_x as u8, pixel_y as u8).unwrap_or(0)
                    } else {
                        0
                    };
                    let color = resolve(index);
                    pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        }
    }
    (width, height, pixels)
}

/// The source pixel a flip maps a display `(x, y)` back to within a `w × h`
/// tile.
pub fn flipped(x: u8, y: u8, w: u8, h: u8, flip_x: bool, flip_y: bool) -> (u8, u8) {
    let sx = if flip_x { w.saturating_sub(1) - x } else { x };
    let sy = if flip_y { h.saturating_sub(1) - y } else { y };
    (sx, sy)
}

/// The two atlas tile indices composing an 8×16 object, top slot first for
/// display. The hardware forces the top tile to `tile & !1` and the bottom to
/// `tile | 1`; a vertical flip swaps which shows on top.
pub fn stacked_tiles(tile: u16, flip_y: bool) -> (u16, u16) {
    let top = tile & !1;
    let bottom = tile | 1;
    if flip_y { (bottom, top) } else { (top, bottom) }
}

/// Split a rectangle that runs past the map edges into its wrapped-around
/// pieces, all within `[0, map_size)`. A rectangle inside the map returns one
/// piece unchanged.
pub fn wrapping_parts(x: f32, y: f32, w: f32, h: f32, map_size: f32) -> Vec<(f32, f32, f32, f32)> {
    let mut parts = Vec::new();
    let wraps_x = x + w > map_size;
    let wraps_y = y + h > map_size;

    let w1 = if wraps_x { map_size - x } else { w };
    let h1 = if wraps_y { map_size - y } else { h };

    parts.push((x, y, w1, h1));

    if wraps_x {
        let w2 = w - w1;
        parts.push((0.0, y, w2, h1));
        if wraps_y {
            let h2 = h - h1;
            parts.push((0.0, 0.0, w2, h2));
        }
    }
    if wraps_y {
        let h2 = h - h1;
        parts.push((x, 0.0, w1, h2));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_maps_source_pixel() {
        // No flip is the identity.
        assert_eq!(flipped(0, 0, 8, 8, false, false), (0, 0));
        assert_eq!(flipped(2, 5, 8, 8, false, false), (2, 5));
        // Horizontal flip mirrors across the tile width, vertical across height.
        assert_eq!(flipped(0, 0, 8, 8, true, false), (7, 0));
        assert_eq!(flipped(0, 0, 8, 8, false, true), (0, 7));
        assert_eq!(flipped(1, 6, 8, 8, true, true), (6, 1));
    }

    #[test]
    fn stacking_selects_and_swaps() {
        // Bit 0 is ignored: 0x11 composes tiles 0x10 (top) and 0x11 (bottom).
        assert_eq!(stacked_tiles(0x10, false), (0x10, 0x11));
        assert_eq!(stacked_tiles(0x11, false), (0x10, 0x11));
        // A vertical flip swaps which tile occupies the top slot.
        assert_eq!(stacked_tiles(0x10, true), (0x11, 0x10));
    }

    #[test]
    fn viewport_inside_map_is_one_piece() {
        let parts = wrapping_parts(10.0, 20.0, 160.0, 144.0, 256.0);
        assert_eq!(parts, vec![(10.0, 20.0, 160.0, 144.0)]);
    }

    #[test]
    fn viewport_wraps_on_both_axes() {
        // A viewport straddling the right and bottom edges splits into four
        // pieces covering all four corners.
        let parts = wrapping_parts(200.0, 200.0, 160.0, 144.0, 256.0);
        assert_eq!(
            parts,
            vec![
                (200.0, 200.0, 56.0, 56.0), // bottom-right origin corner
                (0.0, 200.0, 104.0, 56.0),  // wrapped in x
                (0.0, 0.0, 104.0, 88.0),    // wrapped in both
                (200.0, 0.0, 56.0, 88.0),   // wrapped in y
            ]
        );
    }

    #[test]
    fn atlas_texture_dimensions_and_colour() {
        use missingno_core::graphics::PaletteSet;
        let atlas = TileAtlas {
            label: "t".into(),
            tile_width: 8,
            tile_height: 8,
            depth_bits: 2,
            // 20 tiles over 16 columns → 2 rows, the second row half blank.
            indices: vec![0u8; 20 * 64],
            palettes: PaletteSet::FrontendShades,
            regions: vec![],
        };
        let (w, h, pixels) = atlas_texture(&atlas, ATLAS_COLUMNS, |_| RGB8::new(1, 2, 3));
        assert_eq!((w, h), (128, 16)); // 16 cols × 8px wide, 2 rows × 8px tall
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        assert_eq!(&pixels[0..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn span_texture_covers_a_region_slice() {
        use missingno_core::graphics::PaletteSet;
        let atlas = TileAtlas {
            label: "t".into(),
            tile_width: 8,
            tile_height: 8,
            depth_bits: 2,
            indices: vec![0u8; 40 * 64],
            palettes: PaletteSet::FrontendShades,
            regions: vec![],
        };
        // A 16-tile span over 16 columns is one tile-row: 128×8.
        let (w, h, pixels) =
            atlas_span_texture(&atlas, 8, 16, ATLAS_COLUMNS, |_| RGB8::new(9, 9, 9));
        assert_eq!((w, h), (128, 8));
        assert_eq!(pixels.len(), (w * h * 4) as usize);
    }
}
