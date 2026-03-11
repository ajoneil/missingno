// --- Background tile fetcher ---

use crate::game_boy::ppu::{PipelineRegisters, VideoControl, memory::Vram};

use super::super::tiles::{TileBlockId, TileIndex};
use super::shifters::BgShifter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetcherStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
    /// The fetcher has completed all three VRAM reads and is frozen,
    /// waiting for the SEKO-triggered reload (fine_count == 7).
    Idle,
}

/// Which half of LEBO's 2-dot clock cycle the fetcher is in.
/// The fetcher (and OAM scanner) are clocked at half the dot rate.
/// T1 is the first dot — address computation from live register values.
/// T2 is the second dot — VRAM read at the pre-computed address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetcherTick {
    T1,
    T2,
}

pub(super) struct TileFetcher {
    pub(super) step: FetcherStep,
    /// Which half of the 2-dot fetcher clock cycle we're in.
    pub(super) tick: FetcherTick,
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
    /// VRAM address computed at T1, to be read at T2.
    /// On hardware this is the combinational address on the VRAM bus
    /// during phase_tfetch 0-1.
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
    pub(super) fn new() -> Self {
        Self {
            step: FetcherStep::GetTile,
            tick: FetcherTick::T2,
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

    /// Compute the VRAM offset for tile data (GetTileDataLow/High T1).
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

    /// Advance the background tile fetcher by one dot.
    ///
    /// Each fetcher step spans 2 dots (T1 + T2):
    /// - T1: compute VRAM address from live register values (SCX, SCY, LCDC).
    /// - T2: read VRAM at the pre-computed address and latch the result.
    ///
    /// GetTile always enters at T2 (after load_into/reset). Its address
    /// bus is combinational, so T2 computes the address from current
    /// register values and reads VRAM atomically. For GetTileDataLow and
    /// GetTileDataHigh, T1 always precedes T2, so registers (LCDC tile
    /// address mode, SCY) are sampled 1 dot before the VRAM read — matching
    /// hardware's phase_tfetch 0-1 address computation.
    pub(super) fn advance(
        &mut self,
        pixel_counter: u8,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
    ) {
        match self.step {
            FetcherStep::GetTile => match self.tick {
                FetcherTick::T1 => {
                    // GetTile always enters at T2 (after load_into/reset),
                    // so T1 is only reachable if the entry point changes.
                    // Compute the address here for completeness.
                    self.vram_address =
                        self.tile_index_address(pixel_counter, window_line_counter, regs, video);
                    self.tick = FetcherTick::T2;
                }
                FetcherTick::T2 => {
                    // GetTile T2 computes the address atomically: the
                    // hardware address bus is combinational from SCX/SCY/LCDC,
                    // always reflecting current register values. Since no T1
                    // precedes this (GetTile always enters at T2 after reload),
                    // we compute the address here from live registers.
                    self.vram_address =
                        self.tile_index_address(pixel_counter, window_line_counter, regs, video);
                    self.tile_index = vram.read_byte(self.vram_address);
                    self.tick = FetcherTick::T1;
                    self.step = FetcherStep::GetTileDataLow;
                }
            },
            FetcherStep::GetTileDataLow => match self.tick {
                FetcherTick::T1 => {
                    self.vram_address =
                        self.tile_data_address(window_line_counter, regs, video, false);
                    self.tick = FetcherTick::T2;
                }
                FetcherTick::T2 => {
                    self.tile_data_low = vram.read_byte(self.vram_address);
                    self.tick = FetcherTick::T1;
                    self.step = FetcherStep::GetTileDataHigh;
                }
            },
            FetcherStep::GetTileDataHigh => match self.tick {
                FetcherTick::T1 => {
                    self.vram_address =
                        self.tile_data_address(window_line_counter, regs, video, true);
                    self.tick = FetcherTick::T2;
                }
                FetcherTick::T2 => {
                    self.tile_data_high = vram.read_byte(self.vram_address);
                    self.tick = FetcherTick::T1;
                    self.step = FetcherStep::Idle;
                }
            },
            FetcherStep::Idle => {
                // The fetcher is frozen — it waits here until the
                // SEKO-triggered reload (fine_count == 7) fires from
                // mode3_rising, which calls load_into() and resets
                // the fetcher to GetTile.
            }
        }
    }

    /// Load fetched tile data into the BG shifter and reset the fetcher to
    /// GetTile for the next tile.
    pub(super) fn load_into(&mut self, bg_shifter: &mut BgShifter) {
        bg_shifter.load(self.tile_data_low, self.tile_data_high);
        if self.fetching_window {
            self.window_tile_x = self.window_tile_x.wrapping_add(1);
        }
        self.step = FetcherStep::GetTile;
        self.tick = FetcherTick::T2;
    }

    /// Reset the fetcher for a window trigger.
    pub(super) fn reset_for_window(&mut self) {
        self.step = FetcherStep::GetTile;
        self.tick = FetcherTick::T2;
        self.window_tile_x = 0;
        self.fetching_window = true;
    }
}
