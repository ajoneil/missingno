// --- Background tile fetcher ---

use crate::game_boy::ppu::{PipelineRegisters, VideoControl, memory::Vram};

use super::super::tiles::TileIndex;
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
/// T1 is the first dot (LEBO low → high edge); T2 is the second
/// (LEBO high → low edge, when the actual work fires).
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

    /// Read the tile index from the tilemap for the current fetch position.
    fn read_tile_index(
        &self,
        pixel_counter: u8,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
    ) -> u8 {
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
        vram.tile_map(map_id).get_tile(map_x, map_y).0
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

    /// Read one byte of tile data (low or high bitplane) for the
    /// current BG/window fetch.
    ///
    /// The tile data address combines the tile index (cached from the
    /// tilemap read) with the fine Y offset from the appropriate
    /// address generator. The VRAM interface (page 25) performs the read.
    fn read_tile_data(
        &self,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
        high: bool,
    ) -> u8 {
        let tile_index = TileIndex(self.tile_index);
        let (block_id, mapped_idx) = regs.control.tile_address_mode().tile(tile_index);

        let fine_y = if self.fetching_window {
            Self::window_fine_y(window_line_counter)
        } else {
            Self::bg_fine_y(regs, video)
        };

        let block = vram.tile_block(block_id);
        block.data[mapped_idx.0 as usize * 16 + fine_y as usize * 2 + high as usize]
    }

    /// Advance the background tile fetcher by one dot.
    pub(super) fn advance(
        &mut self,
        pixel_counter: u8,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
    ) {
        match self.step {
            FetcherStep::GetTile => {
                if self.tick == FetcherTick::T1 {
                    self.tick = FetcherTick::T2;
                } else {
                    self.tile_index =
                        self.read_tile_index(pixel_counter, window_line_counter, regs, video, vram);
                    self.tick = FetcherTick::T1;
                    self.step = FetcherStep::GetTileDataLow;
                }
            }
            FetcherStep::GetTileDataLow => {
                if self.tick == FetcherTick::T1 {
                    self.tick = FetcherTick::T2;
                } else {
                    self.tile_data_low =
                        self.read_tile_data(window_line_counter, regs, video, vram, false);
                    self.tick = FetcherTick::T1;
                    self.step = FetcherStep::GetTileDataHigh;
                }
            }
            FetcherStep::GetTileDataHigh => {
                if self.tick == FetcherTick::T1 {
                    self.tick = FetcherTick::T2;
                } else {
                    self.tile_data_high =
                        self.read_tile_data(window_line_counter, regs, video, vram, true);
                    self.tick = FetcherTick::T1;
                    self.step = FetcherStep::Idle;
                }
            }
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
