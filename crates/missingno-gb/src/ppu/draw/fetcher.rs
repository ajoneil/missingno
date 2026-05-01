// --- Background tile fetcher ---

use crate::ppu::{memory::Vram, PipelineRegisters, VideoControl};

use super::super::types::tiles::{TileBlockId, TileIndex};
use super::shifters::BgShifter;

pub(in crate::ppu) struct TileFetcher {
    /// Hardware fetcher counter (LAXU/MESU/NYVA): 3-bit ripple counter,
    /// values 0-5. Clocked by LEBO = NAND(ALET, MOCE), which fires when
    /// alet falls (master clock rises → rise() in the emulator). VRAM
    /// reads occur on the opposite edge (master falls → fall()) at
    /// counter values 0, 2, 4. Terminal value 5: MOCE = NAND(LAXU,
    /// NYVA) goes low, freezing the counter and firing LYRY. Reset to 0
    /// on TAVE (pipe load) or window trigger.
    pub(in crate::ppu) fetch_counter: u8,
    /// Window tile X counter (hardware's win_x.map). Increments per
    /// window tile fetched. Reset to 0 on window trigger.
    pub(in crate::ppu) window_tile_x: u8,
    /// Cached tile index from GetTile step.
    tile_index: u8,
    /// Cached low byte of tile row from GetTileDataLow step.
    tile_data_low: u8,
    /// Cached high byte of tile row from GetTileDataHigh step.
    tile_data_high: u8,
    /// Whether we're fetching from the window tilemap.
    pub(in crate::ppu) fetching_window: bool,
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
    /// LYRY: combinational decode of MOCE. MOCE = NAND(LAXU, NYVA),
    /// fires when bits 0 and 2 are set = counter value 5.
    /// LYRY = NOT(MOCE), true when counter >= 5.
    pub(in crate::ppu) fn lyry(&self) -> bool {
        self.fetch_counter >= 5
    }

    pub(in crate::ppu) fn tile_data_low(&self) -> u8 {
        self.tile_data_low
    }

    pub(in crate::ppu) fn tile_data_high(&self) -> u8 {
        self.tile_data_high
    }

    pub(in crate::ppu) fn new() -> Self {
        Self {
            fetch_counter: 0,
            window_tile_x: 0,
            tile_index: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
            vram_address: 0,
        }
    }

    /// Boot-ROM-handoff fetcher state (spec §11.1): BG fetch counter at
    /// terminal value 5 (LAXU=1, MESU=0, NYVA=1) with MOCE holding LEBO
    /// frozen. Tile-data and tile-index latches remain at 0 — §11.1
    /// classifies them as boot-ROM-residual, not observable on LD0/LD1
    /// after any subsequent LCDC.7 cycle.
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            fetch_counter: 5,
            window_tile_x: 0,
            tile_index: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
            vram_address: 0,
        }
    }

    /// Reset fetcher state for a new scanline. Only resets the counter
    /// and window tracking -- tile_data_low/tile_data_high (tile_temp)
    /// are NOT reset, matching hardware where these latches persist
    /// across scanlines.
    pub(in crate::ppu) fn reset_scanline(&mut self) {
        self.fetch_counter = 0;
        self.window_tile_x = 0;
        self.tile_index = 0;
        self.fetching_window = false;
        self.vram_address = 0;
    }

    // --- Address generation ---
    //
    // BG and window paths share the VRAM address interface but use
    // distinct coordinate generators: BG pulls scroll-adjusted (SCX, SCY,
    // LY) coordinates; window pulls (window_tile_x, window_line_counter)
    // with no scroll. Hardware AMUV/VEVY tri-states arbitrate which
    // tilemap base address (LCDC.3 BG_MAP via XAFO; LCDC.6 WIN_MAP via
    // WOKY) drives ~ma10 at counter=0, mutually exclusive per BAFY/WUKO
    // arming. Tile-data stages (counter=2 and 4) drive ~ma12 via VURY
    // gated by the NETA enable, with VUZA combining LCDC.4 (TILE_SEL via
    // WEXU) and tile-index bit 7 to implement the signed-vs-unsigned
    // tile-index arbitration.

    /// BG tilemap coordinate computation.
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

    /// Window tilemap coordinate computation.
    /// Uses the window's internal line counter, no scroll offset.
    fn window_tilemap_coords(&self, window_line_counter: u8) -> (u8, u8) {
        (self.window_tile_x, window_line_counter / 8)
    }

    /// Compute the VRAM offset for the current tilemap lookup.
    /// Reads SCX, SCY, and the active tilemap-select bit (LCDC.3
    /// BG_MAP via XAFO for BG; LCDC.6 WIN_MAP via WOKY for window)
    /// live each fetch — mirrors the hardware AMUV/VEVY tri-state
    /// drivers that arbitrate ~ma10 at counter=0 per BAFY/WUKO arming.
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

    /// Compute the VRAM offset for tile data.
    /// Reads LCDC.4 (TILE_SEL via WEXU) and SCY live each fetch, plus
    /// the cached tile-index byte. Hardware drives ~ma12 via the VURY
    /// tri-state, with VUZA combining WEXU and the tile-index bit 7
    /// capture (PYJU.q = bg_tile7) to implement the signed-vs-unsigned
    /// tile-block arbitration. Called twice per fetch (counter=2 low
    /// bitplane, counter=4 high bitplane) — mirrors hardware's NETA
    /// (`bp_cy`) enable asserting at both stages.
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

    /// Falling-edge advance: VRAM reads only (no counter increment).
    ///
    /// The hardware fetcher counter is clocked by LEBO = NAND(ALET, MOCE),
    /// which fires on the rising edge (alet falls) only. VRAM reads are
    /// driven by the counter value that settled after the preceding rise.
    /// Reads happen at counter values 0, 2, 4 on the falling edge.
    pub(in crate::ppu) fn advance_falling(
        &mut self,
        pixel_counter: u8,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
    ) {
        // Per-stage address drive matches hardware's tri-state arbitration:
        //   counter=0 → BAFY/WUKO arm (XAFO/WOKY → ~ma10 via AMUV/VEVY)
        //   counter=2 → NETA enable, low bitplane (VUZA → ~ma12 via VURY)
        //   counter=4 → NETA enable, high bitplane (VUZA → ~ma12 via VURY)
        // Counter values 1, 3, 5 are data-arrival stages with no LCDC
        // contribution to the VRAM address (hardware enables deassert).
        match self.fetch_counter {
            0 => {
                // Tilemap VRAM read.
                self.vram_address =
                    self.tile_index_address(pixel_counter, window_line_counter, regs, video);
                self.tile_index = vram.read_byte(self.vram_address);
            }
            2 => {
                // Tile data low VRAM read.
                self.vram_address = self.tile_data_address(window_line_counter, regs, video, false);
                self.tile_data_low = vram.read_byte(self.vram_address);
            }
            4 => {
                // Tile data high VRAM read.
                self.vram_address = self.tile_data_address(window_line_counter, regs, video, true);
                self.tile_data_high = vram.read_byte(self.vram_address);
            }
            _ => {}
        }
        // No counter increment on falling — LEBO only fires on rising.
    }

    /// Rising-edge advance: counter increment (LEBO clock).
    ///
    /// LEBO = NAND(ALET, MOCE) fires when alet falls (master clock
    /// rises). The counter increments 0→1→2→3→4→5 then saturates
    /// (MOCE goes low at 5, freezing LEBO). LYRY fires combinationally
    /// when counter reaches 5.
    ///
    /// On the AVAP-reaction rise, the caller gates out this advance
    /// (via `was_rendering`) so the counter stays at 0 — modeling
    /// NYXU's reset hold through the first LEBO edge after AVAP.
    pub(in crate::ppu) fn advance_rising(&mut self) {
        if self.fetch_counter < 5 {
            self.fetch_counter += 1;
        }
    }

    /// Load fetched tile data into the BG shifter and reset the fetcher to
    /// GetTile for the next tile. NYXU fires: counter resets to 0.
    pub(in crate::ppu) fn load_into(&mut self, bg_shifter: &mut BgShifter) {
        bg_shifter.load(self.tile_data_low, self.tile_data_high);
        if self.fetching_window {
            self.window_tile_x = self.window_tile_x.wrapping_add(1);
        }
        self.fetch_counter = 0;
    }

    /// Reset the fetcher for a window trigger. NYXU fires: counter resets
    /// to 0. Unlike AVAP, this runs AFTER advance_rising on the same dot,
    /// so the next dot's advance_rising proceeds normally (0→1).
    pub(in crate::ppu) fn reset_for_window(&mut self) {
        self.fetch_counter = 0;
        self.window_tile_x = 0;
        self.fetching_window = true;
    }
}
