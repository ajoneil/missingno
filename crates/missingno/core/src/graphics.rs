//! System-agnostic vocabulary for the debugger's graphics surfaces: the tile
//! atlases, tile maps, and object table a family's PPU/VDP exposes. A core
//! decodes its own video memory into these plain shapes; the panes, CLI, and
//! MCP tools render them without knowing the hardware.
//!
//! The load-bearing honesty here mirrors the waveform surface's "codes, not
//! floats": a tile has **no intrinsic palette** on any of these systems — the
//! palette is chosen at the use-site (map entry, sprite attribute). So an atlas
//! ships the **palette indices as the silicon holds them**, plus the palette
//! set needed to preview them ([`PaletteSet`]): frontend-owned shades on DMG,
//! core-owned resolved colours on CGB. Maps and objects index into the atlases.

use crate::inspect::Tone;

/// Everything a family's PPU/VDP exposes for the graphics panes, bundled so one
/// interest gate and one seam accessor cover all three surfaces (they
/// cross-reference: maps and objects index into `atlases`).
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicsView {
    pub atlases: Vec<TileAtlas>,
    pub maps: Vec<TileMap>,
    pub objects: Option<ObjectTable>,
}

// --- Tile atlas --------------------------------------------------------------

/// A grid of tiles as the hardware holds them, in index order.
#[derive(Clone, Debug, PartialEq)]
pub struct TileAtlas {
    /// Display name — "VRAM bank 0", "Pattern table 0".
    pub label: String,
    pub tile_width: u8,
    pub tile_height: u8,
    /// Palette-index depth: 2 (Game Boy / NES) or 4 (SMS).
    pub depth_bits: u8,
    /// Every tile's decoded palette indices, row-major within a tile and one
    /// tile after another in hardware index order — a single flat buffer, each
    /// tile occupying [`tile_stride`](Self::tile_stride) entries. Index it
    /// through [`pixel`](Self::pixel) or [`tile_indices`](Self::tile_indices).
    pub indices: Vec<u8>,
    /// How to colour the indices.
    pub palettes: PaletteSet,
    /// The hardware's grouping of the index range, for the frontend to lay out
    /// and label — the Game Boy's three tile-data blocks. Empty where a family
    /// exposes no grouping. Presentation metadata only: maps and objects still
    /// index tiles by number, never a region.
    pub regions: Vec<AtlasRegion>,
}

/// A contiguous, named span of an atlas's tiles — the memory-map analog for
/// tile indices. The `label` is the hardware's name for the span (Game Boy
/// "Block 0"), the `help` an optional address-range hint ("$8000–$87FF").
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasRegion {
    pub label: &'static str,
    /// First tile index in the span.
    pub start: usize,
    /// Tile count in the span.
    pub len: usize,
    pub help: Option<&'static str>,
}

impl TileAtlas {
    /// Palette indices per tile: `tile_width * tile_height`.
    fn tile_stride(&self) -> usize {
        self.tile_width as usize * self.tile_height as usize
    }

    /// Number of tiles in the atlas.
    pub fn tile_count(&self) -> usize {
        match self.tile_stride() {
            0 => 0,
            stride => self.indices.len() / stride,
        }
    }

    /// One tile's row-major palette indices, or `None` when out of range.
    pub fn tile_indices(&self, tile: usize) -> Option<&[u8]> {
        let stride = self.tile_stride();
        let start = tile.checked_mul(stride)?;
        self.indices.get(start..start.checked_add(stride)?)
    }

    /// The palette index at pixel `(x, y)` of `tile`, or `None` when either
    /// index is out of range. Row-major within the tile.
    pub fn pixel(&self, tile: usize, x: u8, y: u8) -> Option<u8> {
        if x >= self.tile_width || y >= self.tile_height {
            return None;
        }
        let within = y as usize * self.tile_width as usize + x as usize;
        self.tile_indices(tile)
            .and_then(|indices| indices.get(within).copied())
    }

    /// The region containing `tile`, or `None` when unannotated or out of range.
    pub fn region_of(&self, tile: usize) -> Option<&AtlasRegion> {
        self.regions
            .iter()
            .find(|region| tile >= region.start && tile < region.start + region.len)
    }

    /// Whether `regions` is a valid grouping: ordered, contiguous, and covering
    /// exactly `0..tile_count()`. An empty grouping (unannotated) is valid.
    pub fn regions_valid(&self) -> bool {
        if self.regions.is_empty() {
            return true;
        }
        let mut next = 0;
        for region in &self.regions {
            if region.start != next {
                return false;
            }
            next += region.len;
        }
        next == self.tile_count()
    }
}

/// How an atlas's indices become colours.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteSet {
    /// Frontend owns the palette (DMG shade selection): a consumer resolves
    /// indices through the user palette, exactly like the BGP/OBP swatches.
    FrontendShades,
    /// Core owns the palettes (CGB CRAM, NES palette RAM, SMS CRAM): named
    /// resolved-colour palettes a consumer previews or picks between. Reference
    /// counted so the atlases sharing one palette set hold one allocation.
    Owned(std::sync::Arc<[NamedPalette]>),
}

/// A named resolved-colour palette: `2^depth_bits` entries.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedPalette {
    /// Display name — "BG0", "OBP3", "Palette 1".
    pub label: String,
    pub colors: Vec<rgb::RGB8>,
}

// --- Tile map ----------------------------------------------------------------

/// A background/name-table map: a grid of cells indexing into an atlas, with
/// the on-screen viewports drawn over it.
#[derive(Clone, Debug, PartialEq)]
pub struct TileMap {
    /// Display name — "Tile Map 0", "Nametable 0".
    pub label: String,
    pub columns: u16,
    pub rows: u16,
    /// The atlas its entries address by default (index into
    /// [`GraphicsView::atlases`]).
    pub atlas: u8,
    /// Cells, row-major.
    pub entries: Vec<MapEntry>,
    pub viewports: Vec<Viewport>,
}

impl TileMap {
    /// The entry at `(column, row)`, or `None` when either is out of range.
    pub fn entry(&self, column: u16, row: u16) -> Option<&MapEntry> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        self.entries
            .get(row as usize * self.columns as usize + column as usize)
    }
}

/// One tile-map cell.
#[derive(Clone, Debug, PartialEq)]
pub struct MapEntry {
    pub tile: u16,
    /// Core-owned palette selector for this cell (CGB 0-7, NES 0-3, SMS 0-1);
    /// `None` where the frontend owns it (DMG).
    pub palette: Option<u8>,
    /// Overrides [`TileMap::atlas`] for this cell (CGB tile VRAM bank); `None`
    /// where the map has a single atlas.
    pub atlas: Option<u8>,
    pub flip_x: bool,
    pub flip_y: bool,
    /// BG-over-OBJ priority (CGB) / SMS priority bit.
    pub priority: bool,
}

/// An on-screen region drawn over the map. Honest sampling: the scroll/window
/// registers **as of the sample instant** (per-vblank when running, live when
/// paused) — NOT what any individual scanline used, so raster-split scroll
/// shows the post-frame value only.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    /// Display name — "background (SCX,SCY)", "window (WX,WY)".
    pub label: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// The region wraps the map edges (GB background); a non-wrapping region
    /// clips (GB window).
    pub wraps: bool,
    /// Overlay accent; the frontend maps it to a colour.
    pub tone: Tone,
}

// --- Object table ------------------------------------------------------------

/// The sprite/object table (GB OAM, NES OAM, SMS SAT).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectTable {
    /// Display name — "OAM", "Sprites", "SAT".
    pub label: String,
    /// The sprite pattern source (index into [`GraphicsView::atlases`]).
    pub atlas: u8,
    pub object_height: u8,
    pub objects: Vec<Object>,
}

/// How much of an object's span lies inside the visible area on one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    Full,
    Partial,
    Off,
}

impl Coverage {
    /// Classify the span `[start, start + len)` against the visible `[0, extent)`.
    pub fn of_span(start: i32, len: u32, extent: u32) -> Self {
        let end = start.saturating_add_unsigned(len);
        let extent = extent as i32;
        if start >= 0 && end <= extent {
            Coverage::Full
        } else if end <= 0 || start >= extent {
            Coverage::Off
        } else {
            Coverage::Partial
        }
    }
}

/// One object/sprite.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub index: u8,
    /// The raw table values as the hardware stores them (GB OAM's +8/+16
    /// encoding, the TMS9918's one-line-above Y); the coverage fields carry the
    /// relation to the screen.
    pub x: u16,
    pub y: u16,
    pub tile: u16,
    pub coverage_x: Coverage,
    pub coverage_y: Coverage,
    /// Displayed by the raster — the core's verdict, which can be stricter than
    /// the coverages (the TMS9918 stops scanning at the sprite-attribute
    /// terminator, so later entries never display).
    pub on_screen: bool,
    /// Core-owned palette selector; `None` where frontend-owned (DMG) or absent.
    pub palette: Option<u8>,
    /// Tile VRAM bank; `None` where absent.
    pub bank: Option<u8>,
    pub flip_x: bool,
    pub flip_y: bool,
    pub priority: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas(indices: Vec<u8>) -> TileAtlas {
        TileAtlas {
            label: "test".into(),
            tile_width: 8,
            tile_height: 8,
            depth_bits: 2,
            indices,
            palettes: PaletteSet::FrontendShades,
            regions: vec![],
        }
    }

    fn region(label: &'static str, start: usize, len: usize) -> AtlasRegion {
        AtlasRegion {
            label,
            start,
            len,
            help: None,
        }
    }

    #[test]
    fn empty_regions_are_valid() {
        let a = atlas(vec![0; 64]);
        assert!(a.regions_valid());
        assert!(a.region_of(0).is_none());
    }

    #[test]
    fn contiguous_covering_regions_are_valid() {
        let mut a = atlas(vec![0; 6 * 64]);
        a.regions = vec![region("A", 0, 2), region("B", 2, 4)];
        assert!(a.regions_valid());
        assert_eq!(a.region_of(0).map(|r| r.label), Some("A"));
        assert_eq!(a.region_of(1).map(|r| r.label), Some("A"));
        assert_eq!(a.region_of(2).map(|r| r.label), Some("B"));
        assert_eq!(a.region_of(5).map(|r| r.label), Some("B"));
        assert!(a.region_of(6).is_none());
    }

    #[test]
    fn non_contiguous_or_short_regions_are_invalid() {
        let mut a = atlas(vec![0; 6 * 64]);
        // A gap between the two spans.
        a.regions = vec![region("A", 0, 2), region("B", 3, 3)];
        assert!(!a.regions_valid());
        // Fails to cover the whole atlas.
        a.regions = vec![region("A", 0, 2), region("B", 2, 2)];
        assert!(!a.regions_valid());
        // Overshoots the atlas.
        a.regions = vec![region("A", 0, 2), region("B", 2, 6)];
        assert!(!a.regions_valid());
    }

    #[test]
    fn atlas_pixel_indexes_row_major() {
        // A single 8×8 tile whose index equals x for every row.
        let indices: Vec<u8> = (0..64).map(|i| (i % 8) as u8).collect();
        let a = atlas(indices);
        assert_eq!(a.pixel(0, 0, 0), Some(0));
        assert_eq!(a.pixel(0, 5, 0), Some(5));
        assert_eq!(a.pixel(0, 5, 3), Some(5));
        assert_eq!(a.pixel(0, 7, 7), Some(7));
    }

    #[test]
    fn atlas_pixel_bounds_check() {
        let a = atlas(vec![0; 64]);
        assert_eq!(a.pixel(0, 8, 0), None); // x out of range
        assert_eq!(a.pixel(0, 0, 8), None); // y out of range
        assert_eq!(a.pixel(1, 0, 0), None); // no such tile
    }

    #[test]
    fn coverage_classifies_a_span_against_the_visible_extent() {
        assert_eq!(Coverage::of_span(0, 8, 160), Coverage::Full);
        assert_eq!(Coverage::of_span(152, 8, 160), Coverage::Full);
        // Hanging off the left edge, then the right.
        assert_eq!(Coverage::of_span(-1, 8, 160), Coverage::Partial);
        assert_eq!(Coverage::of_span(153, 8, 160), Coverage::Partial);
        // Entirely outside on either side.
        assert_eq!(Coverage::of_span(-8, 8, 160), Coverage::Off);
        assert_eq!(Coverage::of_span(160, 8, 160), Coverage::Off);
        // A span wider than the visible area covers it without being inside it.
        assert_eq!(Coverage::of_span(-4, 200, 160), Coverage::Partial);
    }

    #[test]
    fn map_entry_indexes_row_major() {
        let entry = |tile| MapEntry {
            tile,
            palette: None,
            atlas: None,
            flip_x: false,
            flip_y: false,
            priority: false,
        };
        // 3 columns × 2 rows: cell value = row*10 + column.
        let entries = vec![
            entry(0),
            entry(1),
            entry(2),
            entry(10),
            entry(11),
            entry(12),
        ];
        let map = TileMap {
            label: "m".into(),
            columns: 3,
            rows: 2,
            atlas: 0,
            entries,
            viewports: vec![],
        };
        assert_eq!(map.entry(0, 0).map(|e| e.tile), Some(0));
        assert_eq!(map.entry(2, 0).map(|e| e.tile), Some(2));
        assert_eq!(map.entry(1, 1).map(|e| e.tile), Some(11));
        assert_eq!(map.entry(2, 1).map(|e| e.tile), Some(12));
        // Out of range on either axis.
        assert_eq!(map.entry(3, 0), None);
        assert_eq!(map.entry(0, 2), None);
    }
}
