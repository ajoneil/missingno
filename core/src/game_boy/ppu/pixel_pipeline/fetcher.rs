// --- Background tile fetcher ---

use crate::game_boy::ppu::{PipelineRegisters, VideoControl, memory::Vram};

use super::super::tiles::{TileBlockId, TileIndex};
use super::shifters::BgShifter;

pub(super) struct TileFetcher {
    /// Hardware's phase_tfetch counter: 0-11 (6 dots x 2 half-phases).
    /// Incremented by 2 each dot (since advance() is called once per dot,
    /// but the hardware counter ticks twice per dot on both half-phases).
    /// Reset to 0 on TAVE (pipe load) or window trigger.
    pub(super) phase_tfetch: u8,
    /// Window tile X counter (hardware's win_x.map). Increments per
    /// window tile fetched. Reset to 0 on window trigger.
    pub(super) window_tile_x: u8,
    /// Cached tile index from GetTile step.
    tile_index: u8,
    /// Cached low byte of tile row from GetTileDataLow step.
    tile_data_low: u8,
    /// Cached high byte of tile row from GetTileDataHigh step.
    tile_data_high: u8,
    /// Whether we're fetching from the window tilemap.
    pub(super) fetching_window: bool,
    /// Last VRAM address used by the fetcher. Set at T2 when the address
    /// is computed and VRAM is read. Retained for debugger visibility.
    vram_address: u16,
}

/// Compute the flat VRAM offset for a tilemap entry at (map_x, map_y).
/// Tilemaps are at 0x1800 (map 0) and 0x1C00 (map 1) within VRAM.
fn tile_map_offset(map_id_index: u8, map_x: u8, map_y: u8) -> u16 {
    let base: u16 = if map_id_index == 0 { 0x1800 } else { 0x1C00 };
    base + map_y as u16 * 32 + map_x as u16
}

/// Compute the flat VRAM offset for a tile data byte.
/// Each tile block is 0x800 bytes. Each tile is 16 bytes (2 bytes per row).
fn tile_data_offset(block_id: TileBlockId, mapped_idx: TileIndex, fine_y: u8, high: bool) -> u16 {
    let base: u16 = block_id.0 as u16 * 0x800;
    base + mapped_idx.0 as u16 * 16 + fine_y as u16 * 2 + high as u16
}

impl TileFetcher {
    /// LYRY: combinational decode. Fetch done when counter >= 10.
    pub(super) fn lyry(&self) -> bool {
        self.phase_tfetch >= 10
    }

    pub(super) fn new() -> Self {
        Self {
            phase_tfetch: 0,
            window_tile_x: 0,
            tile_index: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
            vram_address: 0,
        }
    }

    // --- Address generation (pages 26-27) ---
    //
    // On the die, BG and window have separate address generators:
    //   Page 26 (BACKGROUND): tilemap coords from pixel_counter, SCX, SCY, LY
    //   Page 27 (WINDOW MAP LOOKUP): tilemap coords from window_tile_x, window_line_counter
    // Both feed into the shared VRAM interface (page 25).

    /// BG tilemap coordinate computation (page 26).
    /// Applies SCX/SCY scroll offsets and wraps at 32-tile boundaries.
    fn bg_tilemap_coords(
        &self,
        pixel_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> (u8, u8) {
        let scx = regs.background_viewport.x.output();
        let scy = regs.background_viewport.y.output();
        (
            ((pixel_counter.wrapping_add(scx)) >> 3) & 31,
            (video.ly().wrapping_add(scy) / 8) & 31,
        )
    }

    /// Window tilemap coordinate computation (page 27).
    /// Uses the window's internal line counter, no scroll offset.
    fn window_tilemap_coords(&self, window_line_counter: u8) -> (u8, u8) {
        (self.window_tile_x, window_line_counter / 8)
    }

    /// Compute the VRAM offset for the current tilemap lookup.
    /// Reads SCX, SCY, LCDC (tilemap select) from live registers.
    fn tile_index_address(
        &self,
        pixel_counter: u8,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> u16 {
        let (map_x, map_y) = if self.fetching_window {
            self.window_tilemap_coords(window_line_counter)
        } else {
            self.bg_tilemap_coords(pixel_counter, regs, video)
        };
        let map_id = if self.fetching_window {
            regs.control.window_tile_map()
        } else {
            regs.control.background_tile_map()
        };
        tile_map_offset(map_id.0, map_x, map_y)
    }

    /// BG fine Y offset (page 26): which row within the tile, from SCY + LY.
    fn bg_fine_y(regs: &PipelineRegisters, video: &VideoControl) -> u8 {
        video.ly().wrapping_add(regs.background_viewport.y.output()) % 8
    }

    /// Window fine Y offset (page 27): which row within the tile, from
    /// the window's internal line counter.
    fn window_fine_y(window_line_counter: u8) -> u8 {
        window_line_counter % 8
    }

    /// Compute the VRAM offset for tile data (GetTileDataLow/High T2).
    /// Reads LCDC (tile address mode), SCY from live registers.
    fn tile_data_address(
        &self,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        high: bool,
    ) -> u16 {
        let tile_index = TileIndex(self.tile_index);
        let (block_id, mapped_idx) = regs.control.tile_address_mode().tile(tile_index);
        let fine_y = if self.fetching_window {
            Self::window_fine_y(window_line_counter)
        } else {
            Self::bg_fine_y(regs, video)
        };
        tile_data_offset(block_id, mapped_idx, fine_y, high)
    }

    /// Falling-edge advance: VRAM reads and counter increment.
    ///
    /// The hardware fetcher counter (LAXU/MESU/NYVA) increments on both
    /// half-phases. VRAM reads happen at counter values 0, 4, 8 (falling
    /// edge, driven by LEBO = NAND(ALET, MOCE)). The counter saturates
    /// at 10 (Idle) until reset.
    pub(super) fn advance_falling(
        &mut self,
        pixel_counter: u8,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
    ) {
        match self.phase_tfetch {
            0 => {
                // Tilemap VRAM read (dot 0, falling).
                self.vram_address =
                    self.tile_index_address(pixel_counter, window_line_counter, regs, video);
                self.tile_index = vram.read_byte(self.vram_address);
            }
            4 => {
                // Tile data low VRAM read (dot 2, falling).
                self.vram_address = self.tile_data_address(window_line_counter, regs, video, false);
                self.tile_data_low = vram.read_byte(self.vram_address);
            }
            8 => {
                // Tile data high VRAM read (dot 4, falling).
                self.vram_address = self.tile_data_address(window_line_counter, regs, video, true);
                self.tile_data_high = vram.read_byte(self.vram_address);
            }
            _ => {}
        }
        if self.phase_tfetch < 10 {
            self.phase_tfetch += 1;
        }
    }

    /// Rising-edge advance: counter increment only.
    ///
    /// No VRAM reads on rising. When phase_tfetch reaches 10, LYRY
    /// fires combinationally on the rising edge — the hardware-correct
    /// phase for the fetch-done signal.
    pub(super) fn advance_rising(&mut self) {
        if self.phase_tfetch < 10 {
            self.phase_tfetch += 1;
        }
    }

    /// Load fetched tile data into the BG shifter and reset the fetcher to
    /// GetTile for the next tile.
    pub(super) fn load_into(&mut self, bg_shifter: &mut BgShifter) {
        bg_shifter.load(self.tile_data_low, self.tile_data_high);
        if self.fetching_window {
            self.window_tile_x = self.window_tile_x.wrapping_add(1);
        }
        self.phase_tfetch = 0;
    }

    /// Reset the fetcher for a window trigger.
    pub(super) fn reset_for_window(&mut self) {
        self.phase_tfetch = 0;
        self.window_tile_x = 0;
        self.fetching_window = true;
    }
}
