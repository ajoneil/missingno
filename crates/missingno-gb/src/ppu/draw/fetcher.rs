use crate::ppu::{PipelineRegisters, VideoControl, memory::Vram};

use super::super::types::tiles::{TileBlockId, TileIndex};
use super::shifters::BgShifter;

pub(in crate::ppu) struct TileFetcher {
    /// LAXU/MESU/NYVA 3-bit ripple counter (0-5). Clocked by LEBO on PPU rise; saturates at 5 (MOCE freezes LEBO).
    /// VRAM reads happen on the PPU fall at counter 0/2/4. Reset by TAVE (pipe load) or window trigger.
    pub(in crate::ppu) fetch_counter: u8,
    /// win_x.map: increments per window tile fetched.
    pub(in crate::ppu) window_tile_x: u8,
    tile_index: u8,
    tile_data_low: u8,
    tile_data_high: u8,
    /// Resampled from PYNU at counter=0 and held through the cycle so all VRAM accesses see the same selection.
    pub(in crate::ppu) fetching_window: bool,
    /// Retained for debugger visibility.
    vram_address: u16,
}

fn tile_map_offset(map_id_index: u8, map_x: u8, map_y: u8) -> u16 {
    let base: u16 = if map_id_index == 0 { 0x1800 } else { 0x1C00 };
    base + map_y as u16 * 32 + map_x as u16
}

fn tile_data_offset(block_id: TileBlockId, mapped_idx: TileIndex, fine_y: u8, high: bool) -> u16 {
    let base: u16 = block_id.0 as u16 * 0x800;
    base + mapped_idx.0 as u16 * 16 + fine_y as u16 * 2 + high as u16
}

impl TileFetcher {
    /// LYRY = NOT(MOCE) = counter >= 5 (combinational). True when the BG tile fetch is ready
    /// to load into the shifter on the next NYXU.
    pub(in crate::ppu) fn bg_fetch_done(&self) -> bool {
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

    /// Counter + window tracking reset; tile_temp latches persist across scanlines per hardware.
    pub(in crate::ppu) fn reset_scanline(&mut self) {
        self.fetch_counter = 0;
        self.window_tile_x = 0;
        self.tile_index = 0;
        self.fetching_window = false;
        self.vram_address = 0;
    }

    /// +1 on PX models the within-counter=0 SACU advance (suppressed while ROXY gates SACU).
    fn bg_tilemap_coords(
        &self,
        pixel_counter: u8,
        sacu_active: bool,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> (u8, u8) {
        let scx = regs.background_viewport.x.output();
        let scy = regs.background_viewport.y.output();
        let effective_pix = if sacu_active {
            pixel_counter.wrapping_add(1)
        } else {
            pixel_counter
        };
        (
            ((effective_pix.wrapping_add(scx)) >> 3) & 31,
            (video.ly().wrapping_add(scy) / 8) & 31,
        )
    }

    fn window_tilemap_coords(&self, window_line_counter: u8) -> (u8, u8) {
        (self.window_tile_x, window_line_counter / 8)
    }

    /// Reads SCX/SCY and tilemap-select bits live each fetch (mirrors AMUV/VEVY live arbitration).
    fn tile_index_address(
        &self,
        pixel_counter: u8,
        sacu_active: bool,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> u16 {
        let (map_x, map_y) = if self.fetching_window {
            self.window_tilemap_coords(window_line_counter)
        } else {
            self.bg_tilemap_coords(pixel_counter, sacu_active, regs, video)
        };
        let map_id = if self.fetching_window {
            regs.control.window_tile_map()
        } else {
            regs.control.background_tile_map()
        };
        tile_map_offset(map_id.0, map_x, map_y)
    }

    fn bg_fine_y(regs: &PipelineRegisters, video: &VideoControl) -> u8 {
        video.ly().wrapping_add(regs.background_viewport.y.output()) % 8
    }

    fn window_fine_y(window_line_counter: u8) -> u8 {
        window_line_counter % 8
    }

    /// Reads LCDC.4 (TILE_SEL) live each fetch.
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

    /// PPU fall: VRAM reads at counter 0/2/4 (no counter increment — LEBO only fires on rise).
    pub(in crate::ppu) fn advance_falling(
        &mut self,
        pixel_counter: u8,
        sacu_active: bool,
        window_line_counter: u8,
        window_mode_active: bool,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &Vram,
    ) {
        match self.fetch_counter {
            0 => {
                // BAFY/WUKO arming: latch live PYNU for the cycle; held through counters 1..5.
                self.fetching_window = window_mode_active;
                self.vram_address = self.tile_index_address(
                    pixel_counter,
                    sacu_active,
                    window_line_counter,
                    regs,
                    video,
                );
                self.tile_index = vram.read_byte(self.vram_address);
            }
            2 => {
                self.vram_address = self.tile_data_address(window_line_counter, regs, video, false);
                self.tile_data_low = vram.read_byte(self.vram_address);
            }
            4 => {
                self.vram_address = self.tile_data_address(window_line_counter, regs, video, true);
                self.tile_data_high = vram.read_byte(self.vram_address);
            }
            _ => {}
        }
    }

    /// LEBO counter increment; saturates at 5. Caller gates out on the AVAP-reaction rise so the counter stays at 0.
    pub(in crate::ppu) fn advance_rising(&mut self) {
        if self.fetch_counter < 5 {
            self.fetch_counter += 1;
        }
    }

    /// NYXU pipe load — bg shifter parallel-load + counter reset.
    pub(in crate::ppu) fn load_into(&mut self, bg_shifter: &mut BgShifter) {
        bg_shifter.load(self.tile_data_low, self.tile_data_high);
        if self.fetching_window {
            self.window_tile_x = self.window_tile_x.wrapping_add(1);
        }
        self.fetch_counter = 0;
    }

    /// Window-trigger reset. Runs after advance_rising on the same dot, so the next rise proceeds 0→1.
    /// `fetching_window` is resampled by the immediately-following counter=0 fall (MOSU↑ dot).
    pub(in crate::ppu) fn reset_for_window(&mut self) {
        self.fetch_counter = 0;
        self.window_tile_x = 0;
    }
}
