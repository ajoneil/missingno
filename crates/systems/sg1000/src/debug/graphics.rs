//! The SG-1000's decode of the VDP's memory into the system-agnostic graphics
//! vocabulary ([`missingno_core::graphics`]).
//!
//! Colour is resolved into the pattern atlas: a pattern's colours are fixed by
//! the colour table at the pattern's own index, so the atlas ships TI colour
//! indices under the datasheet palette the console already presents through.
//! Sprites are the other way round — the attribute owns the colour, not the
//! pattern — so their patterns stay one bit deep and each object carries its
//! colour nibble as its palette selector.
//!
//! The decode reads the live registers and VRAM at the instant it runs, and
//! fetches through the chip's own side-effect-free cell read, so it sees what
//! the raster sees.

use std::sync::Arc;

use missingno_core::graphics::{
    AtlasRegion, Coverage, GraphicsView, MapEntry, NamedPalette, Object, ObjectTable, PaletteSet,
    TileAtlas, TileMap,
};
use missingno_ti_vdp::{ACTIVE_LINES, ACTIVE_WIDTH, Mode, SPRITE_TERMINATOR, Vdp};

use crate::console::Sg1000;

/// Where each surface's patterns live in [`GraphicsView::atlases`].
const PATTERN_ATLAS: u8 = 0;
const SPRITE_ATLAS: u8 = 1;

/// Patterns a generator table holds, and the cell grid the name table lays
/// them out in — 40 narrower cells across the same display area in the text
/// family.
const PATTERNS: usize = 256;
const MAP_COLUMNS: u16 = 32;
const TEXT_COLUMNS: u16 = 40;
const MAP_ROWS: u16 = 24;
const CELL_WIDTH: u8 = 8;
const TEXT_CELL_WIDTH: u8 = 6;
const CELL_HEIGHT: u8 = 8;
/// Graphics II gives each third of the display its own 256 patterns.
const THIRD_ROWS: u16 = MAP_ROWS / 3;

/// A pattern index is a TI colour index; a sprite pattern is one bit, coloured
/// by its attribute.
const COLOUR_BITS: u8 = 4;
const SPRITE_BITS: u8 = 1;

const SPRITE_ENTRIES: u8 = 32;
const SPRITE_ATTRIBUTE_BYTES: u16 = 4;
/// A 16×16 sprite assembles four consecutive generators.
const LARGE_SPRITE_PATTERNS: u16 = 4;
const SMALL_SPRITE_SIZE: u8 = 8;
const LARGE_SPRITE_SIZE: u8 = 16;
/// The early-clock bit in a sprite's tag byte, and the shift it applies.
const EARLY_CLOCK: u8 = 0x80;
const EARLY_CLOCK_DOTS: i16 = 32;
const SPRITE_COLOUR: u8 = 0x0F;

/// The chip's Y is one line above the sprite's first displayed line; values
/// past the terminator place a sprite above the display area instead.
const SPRITE_Y_OFFSET: i16 = 1;
const SPRITE_Y_WRAP: i16 = 256;

/// The three thirds of a Graphics II atlas, in display order.
const GRAPHICS_II_THIRDS: [AtlasRegion; 3] = [
    AtlasRegion {
        label: "Top third",
        start: 0,
        len: PATTERNS,
        help: Some("cell rows 0-7"),
    },
    AtlasRegion {
        label: "Middle third",
        start: PATTERNS,
        len: PATTERNS,
        help: Some("cell rows 8-15"),
    },
    AtlasRegion {
        label: "Bottom third",
        start: 2 * PATTERNS,
        len: PATTERNS,
        help: Some("cell rows 16-23"),
    },
];

/// The table layout a mode selects. The undocumented M1/M2/M3 combinations
/// have no stated layout of their own, so they read as Graphics I.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Layout {
    GraphicsI,
    GraphicsII,
    Multicolor,
    Text,
}

impl Layout {
    fn of(mode: Mode) -> Layout {
        match mode {
            Mode::GraphicsII => Layout::GraphicsII,
            Mode::Multicolor => Layout::Multicolor,
            Mode::Text => Layout::Text,
            Mode::GraphicsI | Mode::BitmapText | Mode::BitmapMulticolor | Mode::TextMulticolor => {
                Layout::GraphicsI
            }
        }
    }

    fn columns(self) -> u16 {
        match self {
            Layout::Text => TEXT_COLUMNS,
            _ => MAP_COLUMNS,
        }
    }

    fn cell_width(self) -> u8 {
        match self {
            Layout::Text => TEXT_CELL_WIDTH,
            _ => CELL_WIDTH,
        }
    }

    /// The atlas offset a cell row's patterns start at — Graphics II's thirds
    /// folded into the tile index so one atlas serves the whole map.
    fn third_offset(self, row: u16) -> u16 {
        match self {
            Layout::GraphicsII => (row / THIRD_ROWS) * PATTERNS as u16,
            _ => 0,
        }
    }
}

/// The graphics surfaces the VDP's memory decodes to, or `None` when no
/// consumer asked for them.
pub fn graphics_view(sg: &Sg1000) -> Option<GraphicsView> {
    sg.graphics_capture().then(|| view(sg.vdp()))
}

fn view(vdp: &Vdp) -> GraphicsView {
    let layout = Layout::of(vdp.mode());
    GraphicsView {
        atlases: vec![pattern_atlas(vdp, layout), sprite_atlas(vdp)],
        maps: vec![name_table(vdp, layout)],
        objects: Some(object_table(vdp)),
    }
}

/// The datasheet palette as the one named palette a pattern index resolves
/// through; index 0 is the all-planes-transparent pass-through and presents
/// black, as the console states.
fn ti_colours() -> PaletteSet {
    PaletteSet::Owned(Arc::from([NamedPalette {
        label: "TI colours".into(),
        colors: super::palette::ti_palette().to_vec(),
    }]))
}

/// One two-entry palette per TI colour: a sprite pattern bit is transparent or
/// the colour its attribute names.
fn sprite_colours() -> PaletteSet {
    let palette = super::palette::ti_palette();
    let named = (0..palette.len())
        .map(|colour| NamedPalette {
            label: format!("Colour {colour:X}"),
            colors: vec![palette[0], palette[colour]],
        })
        .collect::<Vec<_>>();
    PaletteSet::Owned(named.into())
}

/// The pattern generator as the mode reads it, colours already resolved.
fn pattern_atlas(vdp: &Vdp, layout: Layout) -> TileAtlas {
    let (indices, regions) = match layout {
        Layout::GraphicsI => (graphics_i_indices(vdp), vec![]),
        Layout::GraphicsII => (graphics_ii_indices(vdp), GRAPHICS_II_THIRDS.to_vec()),
        Layout::Multicolor => (multicolor_indices(vdp), vec![]),
        Layout::Text => (text_indices(vdp), vec![]),
    };
    let atlas = TileAtlas {
        label: "Patterns".into(),
        tile_width: layout.cell_width(),
        tile_height: CELL_HEIGHT,
        depth_bits: COLOUR_BITS,
        indices,
        palettes: ti_colours(),
        regions,
    };
    debug_assert!(atlas.regions_valid());
    atlas
}

/// One pattern row's eight pixels, MSB first.
fn row_pixels(bits: u8, foreground: u8, background: u8, indices: &mut Vec<u8>) {
    for bit in 0..CELL_WIDTH {
        let lit = bits & (0x80 >> bit) != 0;
        indices.push(if lit { foreground } else { background });
    }
}

/// Graphics I: 256 patterns, each coloured by the colour table's entry for its
/// group of eight.
fn graphics_i_indices(vdp: &Vdp) -> Vec<u8> {
    let patterns = vdp.pattern_table_base();
    let colours = vdp.colour_table_base();
    let mut indices = Vec::with_capacity(PATTERNS * CELL_WIDTH as usize * CELL_HEIGHT as usize);
    for name in 0..PATTERNS as u16 {
        let colour = vdp.vram_cell(colours + name / 8);
        for row in 0..CELL_HEIGHT as u16 {
            let bits = vdp.vram_cell(patterns + name * 8 + row);
            row_pixels(bits, colour >> 4, colour & 0x0F, &mut indices);
        }
    }
    indices
}

/// Graphics II: three thirds of 256 patterns, each row of a pattern carrying
/// its own two colours.
fn graphics_ii_indices(vdp: &Vdp) -> Vec<u8> {
    let tiles = PATTERNS * GRAPHICS_II_THIRDS.len();
    let mut indices = Vec::with_capacity(tiles * CELL_WIDTH as usize * CELL_HEIGHT as usize);
    for tile in 0..tiles as u16 {
        for row in 0..CELL_HEIGHT as u16 {
            let (bits, colour) = vdp.graphics_ii_cells(tile * 8 + row);
            row_pixels(bits, colour >> 4, colour & 0x0F, &mut indices);
        }
    }
    indices
}

/// Multicolor: a pattern's eight bytes each hold two colours. On the display a
/// byte paints a 4×4 block pair and a cell takes only the byte pair its map row
/// selects; the atlas shows the whole pattern instead, one byte per row.
fn multicolor_indices(vdp: &Vdp) -> Vec<u8> {
    let patterns = vdp.pattern_table_base();
    let mut indices = Vec::with_capacity(PATTERNS * CELL_WIDTH as usize * CELL_HEIGHT as usize);
    for name in 0..PATTERNS as u16 {
        for row in 0..CELL_HEIGHT as u16 {
            let byte = vdp.vram_cell(patterns + name * 8 + row);
            for x in 0..CELL_WIDTH {
                indices.push(if x < CELL_WIDTH / 2 {
                    byte >> 4
                } else {
                    byte & 0x0F
                });
            }
        }
    }
    indices
}

/// Text: 256 six-pixel patterns in R7's two colours.
fn text_indices(vdp: &Vdp) -> Vec<u8> {
    let patterns = vdp.pattern_table_base();
    let text_colours = vdp.registers()[7];
    let (foreground, background) = (text_colours >> 4, text_colours & 0x0F);
    let mut indices =
        Vec::with_capacity(PATTERNS * TEXT_CELL_WIDTH as usize * CELL_HEIGHT as usize);
    for name in 0..PATTERNS as u16 {
        for row in 0..CELL_HEIGHT as u16 {
            let bits = vdp.vram_cell(patterns + name * 8 + row);
            for bit in 0..TEXT_CELL_WIDTH {
                let lit = bits & (0x80 >> bit) != 0;
                indices.push(if lit { foreground } else { background });
            }
        }
    }
    indices
}

/// The name table over the pattern atlas. The chip does not scroll, so the map
/// is the screen and there is no viewport to draw over it.
fn name_table(vdp: &Vdp, layout: Layout) -> TileMap {
    let base = vdp.name_table_base();
    let columns = layout.columns();
    let entries = (0..MAP_ROWS)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| MapEntry {
            tile: vdp.vram_cell(base + row * columns + column) as u16 + layout.third_offset(row),
            palette: None,
            atlas: None,
            flip_x: false,
            flip_y: false,
            priority: false,
        })
        .collect();
    TileMap {
        label: "Name table".into(),
        columns,
        rows: MAP_ROWS,
        atlas: PATTERN_ATLAS,
        entries,
        viewports: vec![],
    }
}

/// The sprite generator: 8×8 patterns, or the 16×16 assemblies R1's SIZE bit
/// selects — four consecutive generators in the chip's quadrant order
/// (top-left, bottom-left, top-right, bottom-right).
fn sprite_atlas(vdp: &Vdp) -> TileAtlas {
    let base = vdp.sprite_pattern_base();
    let large = vdp.sprites_16x16();
    let size = sprite_size(large);
    let tiles = PATTERNS as u16 / if large { LARGE_SPRITE_PATTERNS } else { 1 };

    let mut indices = Vec::with_capacity(tiles as usize * size as usize * size as usize);
    for tile in 0..tiles {
        let pattern = base + tile * if large { LARGE_SPRITE_PATTERNS } else { 1 } * 8;
        for row in 0..size as u16 {
            let bits: u16 = if large {
                u16::from_be_bytes([
                    vdp.vram_cell(pattern + row),
                    vdp.vram_cell(pattern + row + LARGE_SPRITE_SIZE as u16),
                ])
            } else {
                (vdp.vram_cell(pattern + row) as u16) << 8
            };
            for bit in 0..size {
                indices.push(u8::from(bits & (0x8000 >> bit) != 0));
            }
        }
    }

    TileAtlas {
        label: "Sprite patterns".into(),
        tile_width: size,
        tile_height: size,
        depth_bits: SPRITE_BITS,
        indices,
        palettes: sprite_colours(),
        regions: vec![],
    }
}

/// The 32-entry sprite attribute table, coordinates raw as the table holds
/// them, with the screen relation in the coverages. The scan stops at the first
/// terminator, so entries from there on list without displaying. Magnification
/// doubles the displayed size and is not modelled in the coverages.
fn object_table(vdp: &Vdp) -> ObjectTable {
    let base = vdp.sprite_attribute_base();
    let large = vdp.sprites_16x16();
    let size = sprite_size(large);
    let entry_y = |index: u8| vdp.vram_cell(base + index as u16 * SPRITE_ATTRIBUTE_BYTES);
    let scanned = (0..SPRITE_ENTRIES)
        .find(|&index| entry_y(index) == SPRITE_TERMINATOR)
        .unwrap_or(SPRITE_ENTRIES);

    let objects = (0..SPRITE_ENTRIES)
        .map(|index| {
            let entry = base + index as u16 * SPRITE_ATTRIBUTE_BYTES;
            let y = entry_y(index);
            let x = vdp.vram_cell(entry + 1);
            let name = vdp.vram_cell(entry + 2) as u16;
            let tag = vdp.vram_cell(entry + 3);
            let shifted_x = x as i16
                - if tag & EARLY_CLOCK != 0 {
                    EARLY_CLOCK_DOTS
                } else {
                    0
                };
            let line = sprite_y(y);
            let coverage_x = Coverage::of_span(shifted_x as i32, size as u32, ACTIVE_WIDTH as u32);
            let coverage_y = Coverage::of_span(line as i32, size as u32, ACTIVE_LINES as u32);
            Object {
                index,
                x: x as u16,
                y: y as u16,
                tile: if large {
                    name / LARGE_SPRITE_PATTERNS
                } else {
                    name
                },
                coverage_x,
                coverage_y,
                on_screen: index < scanned
                    && coverage_x != Coverage::Off
                    && coverage_y != Coverage::Off,
                palette: Some(tag & SPRITE_COLOUR),
                bank: None,
                flip_x: false,
                flip_y: false,
                priority: false,
            }
        })
        .collect();

    ObjectTable {
        label: "SAT".into(),
        atlas: SPRITE_ATLAS,
        object_height: size,
        objects,
    }
}

/// The first display line a sprite's Y reaches. Values above the terminator
/// wrap to the lines above the display area, which is how a sprite enters from
/// the top edge.
fn sprite_y(y: u8) -> i16 {
    let line = y as i16 + SPRITE_Y_OFFSET;
    if y > SPRITE_TERMINATOR {
        line - SPRITE_Y_WRAP
    } else {
        line
    }
}

/// A sprite pattern is square: 8×8, or the 16×16 R1's SIZE bit selects.
fn sprite_size(large: bool) -> u8 {
    if large {
        LARGE_SPRITE_SIZE
    } else {
        SMALL_SPRITE_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_ti_vdp::Standard;

    /// A port write is claimed at the next memory cycle and locked 17 crystal
    /// periods later; with the display off every cycle is claimable.
    const SERVICE_XTALS: u32 = 24;

    fn write_register(vdp: &mut Vdp, index: u8, value: u8) {
        vdp.write_control(value);
        vdp.write_control(0x80 | index);
    }

    fn write_vram(vdp: &mut Vdp, address: u16, value: u8) {
        vdp.write_control((address & 0xFF) as u8);
        vdp.write_control(0x40 | (address >> 8) as u8);
        vdp.write_data(value);
        vdp.tick(SERVICE_XTALS);
    }

    fn vdp_with(registers: &[(u8, u8)], memory: &[(u16, u8)]) -> Vdp {
        let mut vdp = Vdp::new(Standard::Ntsc);
        for &(index, value) in registers {
            write_register(&mut vdp, index, value);
        }
        for &(address, value) in memory {
            write_vram(&mut vdp, address, value);
        }
        vdp
    }

    /// R0's M3 selects Graphics II; R3 then carries the colour table's base and
    /// its AND mask, so $FF leaves every offset through.
    const GRAPHICS_II: &[(u8, u8)] = &[(0, 0x02), (1, 0x00), (3, 0xFF), (4, 0x03)];

    #[test]
    fn graphics_i_resolves_the_group_colour_into_the_pattern() {
        // Colour table at $0800: group 0 (patterns 0-7) is colour 1 on 14.
        let vdp = vdp_with(
            &[(3, 0x20), (4, 0x00)],
            &[(0x0008, 0b1010_0000), (0x0800, 0x1E)],
        );
        let atlas = pattern_atlas(&vdp, Layout::GraphicsI);
        assert_eq!(atlas.tile_count(), PATTERNS);
        assert_eq!((atlas.tile_width, atlas.tile_height), (8, 8));
        assert_eq!(atlas.depth_bits, COLOUR_BITS);
        let row: Vec<u8> = (0..8).map(|x| atlas.pixel(1, x, 0).unwrap()).collect();
        assert_eq!(row, [1, 14, 1, 14, 14, 14, 14, 14]);
        // A pattern outside the written group keeps the colour table's zero.
        assert_eq!(atlas.pixel(9, 0, 0), Some(0));
    }

    #[test]
    fn graphics_ii_atlas_covers_three_thirds() {
        let vdp = vdp_with(GRAPHICS_II, &[]);
        let atlas = pattern_atlas(&vdp, Layout::GraphicsII);
        assert_eq!(atlas.tile_count(), 3 * PATTERNS);
        assert!(atlas.regions_valid());
        let labels: Vec<&str> = atlas.regions.iter().map(|region| region.label).collect();
        assert_eq!(labels, ["Top third", "Middle third", "Bottom third"]);
        assert_eq!(atlas.region_of(300).map(|r| r.label), Some("Middle third"));
    }

    #[test]
    fn graphics_ii_map_folds_the_third_into_the_tile() {
        // Name table at $3800: the same name byte in each third.
        let name_base = 0x3800;
        let cells = [
            (name_base, 5u8),
            (name_base + 8 * MAP_COLUMNS, 5),
            (name_base + 16 * MAP_COLUMNS, 5),
        ];
        let mut registers = GRAPHICS_II.to_vec();
        registers.push((2, 0x0E));
        let vdp = vdp_with(&registers, &cells);
        let map = name_table(&vdp, Layout::GraphicsII);
        assert_eq!((map.columns, map.rows), (MAP_COLUMNS, MAP_ROWS));
        assert!(map.viewports.is_empty());
        assert_eq!(map.entry(0, 0).map(|e| e.tile), Some(5));
        assert_eq!(map.entry(0, 8).map(|e| e.tile), Some(256 + 5));
        assert_eq!(map.entry(0, 16).map(|e| e.tile), Some(512 + 5));
    }

    #[test]
    fn text_map_is_forty_columns_of_six_pixel_cells() {
        let vdp = vdp_with(&[(1, 0x10), (7, 0xF1)], &[(0x0008, 0b1010_0000)]);
        let atlas = pattern_atlas(&vdp, Layout::Text);
        assert_eq!(atlas.tile_width, TEXT_CELL_WIDTH);
        assert_eq!(atlas.tile_count(), PATTERNS);
        // R7's nibbles colour every pattern: foreground $F on backdrop $1.
        let row: Vec<u8> = (0..6).map(|x| atlas.pixel(1, x, 0).unwrap()).collect();
        assert_eq!(row, [15, 1, 15, 1, 1, 1]);
        let map = name_table(&vdp, Layout::Text);
        assert_eq!((map.columns, map.rows), (TEXT_COLUMNS, MAP_ROWS));
    }

    #[test]
    fn sprites_past_the_terminator_stay_off_screen() {
        // Sprite attribute table at $0000: two on-screen entries, then the
        // terminator, then an entry that would otherwise display.
        let vdp = vdp_with(
            &[(5, 0x00)],
            &[
                (0x0000, 16),
                (0x0004, 32),
                (0x0008, SPRITE_TERMINATOR),
                (0x000C, 48),
            ],
        );
        let table = object_table(&vdp);
        assert_eq!(table.objects.len(), SPRITE_ENTRIES as usize);
        assert_eq!(table.object_height, SMALL_SPRITE_SIZE);
        assert!(table.objects[0].on_screen);
        assert!(table.objects[1].on_screen);
        assert!(!table.objects[2].on_screen);
        assert!(!table.objects[3].on_screen);
        // The table's Y byte reaches the object raw; the display relation is
        // the coverage's job.
        assert_eq!(table.objects[0].y, 16);
        assert_eq!(table.objects[0].coverage_y, Coverage::Full);
    }

    #[test]
    fn early_clock_shifts_a_sprite_left() {
        // Two entries at the same X near the left edge; only the first is
        // early-clocked, so only its span leaves the display area.
        let vdp = vdp_with(
            &[(5, 0x00)],
            &[
                (0x0000, 16),
                (0x0001, 28),
                (0x0002, 0),
                (0x0003, EARLY_CLOCK | 0x0A),
                (0x0004, 16),
                (0x0005, 28),
                (0x0006, 0),
                (0x0007, 0x0A),
                (0x0008, SPRITE_TERMINATOR),
            ],
        );
        let table = object_table(&vdp);
        assert_eq!(table.objects[0].x, 28);
        assert_eq!(table.objects[0].coverage_x, Coverage::Partial);
        assert_eq!(table.objects[1].x, 28);
        assert_eq!(table.objects[1].coverage_x, Coverage::Full);
        // The attribute's colour nibble rides the object's palette selector.
        assert_eq!(table.objects[0].palette, Some(0x0A));
        assert!(table.objects[0].on_screen);
    }

    #[test]
    fn large_sprites_assemble_four_generators() {
        let vdp = vdp_with(&[(1, 0x02), (6, 0x00)], &[(0x0000, 0x80), (0x0010, 0x01)]);
        let atlas = sprite_atlas(&vdp);
        assert_eq!((atlas.tile_width, atlas.tile_height), (16, 16));
        assert_eq!(atlas.tile_count(), PATTERNS / 4);
        assert_eq!(atlas.depth_bits, SPRITE_BITS);
        // The first generator is the left half, the third the right.
        assert_eq!(atlas.pixel(0, 0, 0), Some(1));
        assert_eq!(atlas.pixel(0, 15, 0), Some(1));
        assert_eq!(atlas.pixel(0, 1, 0), Some(0));
        let table = object_table(&vdp);
        assert_eq!(table.object_height, LARGE_SPRITE_SIZE);
    }

    #[test]
    fn undocumented_modes_read_as_graphics_i() {
        assert_eq!(Layout::of(Mode::BitmapText), Layout::GraphicsI);
        assert_eq!(Layout::of(Mode::BitmapMulticolor), Layout::GraphicsI);
        assert_eq!(Layout::of(Mode::TextMulticolor), Layout::GraphicsI);
    }

    #[test]
    fn capture_off_decodes_nothing() {
        let mut console = Sg1000::new(&[0u8; 0x2000], None).expect("flat cartridge image");
        assert!(graphics_view(&console).is_none());
        console.set_graphics_capture(true);
        let view = graphics_view(&console).expect("surfaces decoded");
        assert_eq!(view.atlases.len(), 2);
        assert_eq!(view.maps.len(), 1);
        assert!(view.objects.is_some());
    }
}
