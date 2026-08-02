use crate::ppu::{PipelineRegisters, PpuModel, VideoControl, memory::Vram};

use super::super::types::tiles::{TileBlockId, TileIndex, TileMapId};
use super::shifters::BgShifter;

#[derive(Hash)]
pub(in crate::ppu) struct TileFetcher<P: PpuModel> {
    /// LAXU/MESU/NYVA 3-bit ripple counter (0-5). Clocked by LEBO on PPU rise; saturates at 5 (MOCE freezes LEBO).
    /// VRAM reads happen on the PPU fall at counter 0/2/4. Reset by TAVE (pipe load) or window trigger.
    pub(in crate::ppu) fetch_counter: u8,
    /// win_x.map: increments per window tile fetched.
    pub(in crate::ppu) window_tile_x: u8,
    tile_index: TileIndex,
    /// The BG map attribute fetched alongside the tile index at counter 0; held
    /// through the cycle so the data reads and the shifter load see one cell.
    bg_cell: P::BgCell,
    /// LCDC byte the tile-map-select read sampled at counter 0 (live on DMG, a
    /// stale snapshot on CGB); held so the index fetch picks one map per cycle.
    tile_map_byte: u8,
    tile_data_low: u8,
    tile_data_high: u8,
    /// Resampled from PYNU at counter=0 and held through the cycle so all VRAM accesses see the same selection.
    pub(in crate::ppu) fetching_window: bool,
    /// Retained for debugger visibility.
    vram_address: u16,
    /// Armed by a mid-fetch LCDC.4 SET observed at an odd counter; the next
    /// bitplane read returns the frozen glitch source (the set glitch).
    set_glitch_armed: bool,
    /// The real VRAM byte each BG/OBJ fetch drives onto the tile-data bus (the
    /// physical byte, not the reset-glitch substitute).
    bus_low: u8,
    bus_high: u8,
    /// The set glitch's source: the bus value as of the last TILE_SEL reset,
    /// per plane (the most-recent sprite high, else the BG tile fetched right
    /// after the last LCDC.4 clear). Consumed by an armed set glitch.
    glitch_src_low: u8,
    glitch_src_high: u8,
    /// A TILE_SEL clear arms the next completed BG fetch to snapshot the bus
    /// into the glitch source.
    glitch_capture_armed: bool,
}

fn tile_map_offset(map_id: TileMapId, map_x: u8, map_y: u8) -> u16 {
    let base: u16 = if map_id.0 == 0 { 0x1800 } else { 0x1C00 };
    base + map_y as u16 * 32 + map_x as u16
}

fn tile_data_offset(block_id: TileBlockId, mapped_idx: TileIndex, fine_y: u8, high: bool) -> u16 {
    let base: u16 = block_id.0 as u16 * 0x800;
    base + mapped_idx.0 as u16 * 16 + fine_y as u16 * 2 + high as u16
}

impl<P: PpuModel> TileFetcher<P> {
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
            tile_index: TileIndex(0),
            bg_cell: P::BgCell::default(),
            tile_map_byte: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
            vram_address: 0,
            set_glitch_armed: false,
            bus_low: 0,
            bus_high: 0,
            glitch_src_low: 0,
            glitch_src_high: 0,
            glitch_capture_armed: false,
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            fetch_counter: 5,
            window_tile_x: 0,
            tile_index: TileIndex(0),
            bg_cell: P::BgCell::default(),
            tile_map_byte: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
            vram_address: 0,
            set_glitch_armed: false,
            bus_low: 0,
            bus_high: 0,
            glitch_src_low: 0,
            glitch_src_high: 0,
            glitch_capture_armed: false,
        }
    }

    /// Counter + window tracking reset; tile_temp latches persist across scanlines per hardware.
    pub(in crate::ppu) fn reset_scanline(&mut self) {
        self.fetch_counter = 0;
        self.window_tile_x = 0;
        self.tile_index = TileIndex(0);
        self.bg_cell = P::BgCell::default();
        self.fetching_window = false;
        self.vram_address = 0;
        self.set_glitch_armed = false;
        self.glitch_capture_armed = false;
        // bus_low/high and glitch_src persist across scanlines like the tile_temp
        // latches: a set glitch on a line's first fetch reads the byte left on the
        // tile-data bus by the previous row's interrupted fetch.
    }

    /// A mid-fetch LCDC.4 SET arms the set glitch when observed at an odd counter
    /// (1 or 3), just before the bitplane read it corrupts: counter 1 → the
    /// counter-2 low read, counter 3 → the counter-4 high read.
    pub(in crate::ppu) fn arm_set_glitch(&mut self) {
        if self.fetch_counter == 1 || self.fetch_counter == 3 {
            self.set_glitch_armed = true;
        }
    }

    /// A mid-fetch LCDC.4 CLEAR (TILE_SEL reset) snapshots the tile-data bus of
    /// the fetch in progress into the glitch source. If that fetch has already
    /// driven its high plane (past counter 4), the bus already holds it — capture
    /// now; otherwise arm the upcoming counter-4 to capture it.
    pub(in crate::ppu) fn arm_glitch_capture(&mut self) {
        if self.fetch_counter >= 4 {
            self.glitch_src_low = self.bus_low;
            self.glitch_src_high = self.bus_high;
        } else {
            self.glitch_capture_armed = true;
        }
    }

    /// An OBJ high fetch drives the sprite's high byte onto the tile-data bus and,
    /// as the most-recent draw, becomes the glitch source for both planes (the
    /// glitch reads bitplane-1 from a sprite).
    pub(in crate::ppu) fn drive_bus_from_sprite(&mut self, low: u8, high: u8) {
        self.bus_low = low;
        self.bus_high = high;
        self.glitch_src_low = high;
        self.glitch_src_high = high;
    }

    /// +1 on PX models the within-counter=0 SACU advance (suppressed while ROXY gates SACU).
    fn bg_tilemap_coords(
        &self,
        pixel_counter: u8,
        sacu_active: bool,
        synced_scx: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> (u8, u8) {
        // DMG samples SCX live here (after the CUPA capture relative to a mid-Mode-3
        // write); CGB reads it through the SCX register-file crossing — the same synced
        // snapshot the fine-scroll discard (ROXO↑) uses.
        let scx = if P::SCX_CROSSING.is_synced() {
            synced_scx
        } else {
            regs.background_viewport.x.live()
        };
        let effective_pix = if sacu_active {
            pixel_counter.wrapping_add(1)
        } else {
            pixel_counter
        };
        // CGB crosses the BG tile-column boundary one pixel later than the (PX+SCX) adder,
        // so a boundary-aligned fetch reads the lower column — except the start-of-line
        // tile, which reads SCX>>3.
        let dividend = effective_pix.wrapping_add(scx);
        let dividend = if P::SCX_CROSSING.is_synced() && pixel_counter != 0 {
            dividend.wrapping_sub(1)
        } else {
            dividend
        };
        ((dividend >> 3) & 31, Self::bg_map_row(regs, video))
    }

    /// The BG map row (vertical tile index). Reads SCY live; on CGB the SCY cell
    /// itself lags the mid-Mode-3 write (the SCY crossing's register-path
    /// offset), so the map-row and the fine-Y reads all see the same delayed
    /// value.
    fn bg_map_row(regs: &PipelineRegisters, video: &VideoControl) -> u8 {
        (video.ly().wrapping_add(regs.background_viewport.y.output()) / 8) & 31
    }

    fn window_tilemap_coords(&self, window_line_counter: u8) -> (u8, u8) {
        (self.window_tile_x, window_line_counter / 8)
    }

    /// Reads SCX/SCY live each fetch (mirrors AMUV/VEVY live arbitration); the
    /// tilemap-select bit comes from `tile_map_byte`, captured at counter 0.
    fn tile_index_address(
        &self,
        pixel_counter: u8,
        sacu_active: bool,
        synced_scx: u8,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> u16 {
        let (map_x, map_y) = if self.fetching_window {
            self.window_tilemap_coords(window_line_counter)
        } else {
            self.bg_tilemap_coords(pixel_counter, sacu_active, synced_scx, regs, video)
        };
        let map_select_bit = if self.fetching_window { 6 } else { 3 };
        let map_id = TileMapId((self.tile_map_byte >> map_select_bit) & 1);
        tile_map_offset(map_id, map_x, map_y)
    }

    fn bg_fine_y(regs: &PipelineRegisters, video: &VideoControl) -> u8 {
        video.ly().wrapping_add(regs.background_viewport.y.output()) % 8
    }

    fn window_fine_y(window_line_counter: u8) -> u8 {
        window_line_counter % 8
    }

    /// Samples LCDC.4 (TILE_SEL) through the tile-data-select cell — live on DMG,
    /// crossing-lagged on CGB. Returns the VRAM bank (the CGB attribute may
    /// redirect the tile data to bank 1) and the byte offset.
    fn tile_data_address(
        &self,
        window_line_counter: u8,
        regs: &PipelineRegisters,
        video: &VideoControl,
        high: bool,
    ) -> (u8, u16) {
        let (block_id, mapped_idx) = regs.tile_data_address_mode().tile(self.tile_index);
        let raw_fine_y = if self.fetching_window {
            Self::window_fine_y(window_line_counter)
        } else {
            Self::bg_fine_y(regs, video)
        };
        let (bank, fine_y) = P::bg_tile_source(self.bg_cell, raw_fine_y);
        (bank, tile_data_offset(block_id, mapped_idx, fine_y, high))
    }

    /// CGB TILE_SEL reset glitch: a bitplane read on the crossing-capture dot
    /// of an LCDC.4-clearing write returns the tile index byte instead.
    fn tile_sel_glitched_bitplane(&self, glitch_active: bool) -> Option<u8> {
        (glitch_active && self.tile_index.0 < 0x80).then_some(self.tile_index.0)
    }

    /// PPU fall: VRAM reads at counter 0/2/4 (no counter increment — LEBO only fires on rise).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::ppu) fn advance_falling(
        &mut self,
        pixel_counter: u8,
        sacu_active: bool,
        synced_scx: u8,
        window_line_counter: u8,
        window_mode_active: bool,
        regs: &PipelineRegisters,
        video: &VideoControl,
        vram: &P::Vram,
        tile_sel_glitch_active: bool,
    ) {
        match self.fetch_counter {
            0 => {
                // BAFY/WUKO arming: latch live PYNU for the cycle; held through counters 1..5.
                self.fetching_window = window_mode_active;
                self.tile_map_byte = regs.tile_map_select_byte();
                self.vram_address = self.tile_index_address(
                    pixel_counter,
                    sacu_active,
                    synced_scx,
                    window_line_counter,
                    regs,
                    video,
                );
                // CGB reads the tile index (bank 0) and the map attribute (bank 1)
                // at the same offset on the same dot.
                self.tile_index = TileIndex(vram.bank(0).read_byte(self.vram_address));
                self.bg_cell = P::bg_attribute(vram, self.vram_address);
            }
            2 => {
                let (bank, address) =
                    self.tile_data_address(window_line_counter, regs, video, false);
                self.vram_address = address;
                // The real VRAM byte drives the tile-data bus even when the fetcher
                // latches a glitch substitute.
                self.bus_low = vram.bank(bank).read_byte(address);
                self.tile_data_low = if P::TILE_SEL_SET_GLITCH && self.set_glitch_armed {
                    self.set_glitch_armed = false;
                    self.glitch_src_low
                } else {
                    self.tile_sel_glitched_bitplane(tile_sel_glitch_active)
                        .unwrap_or(self.bus_low)
                };
            }
            4 => {
                let (bank, address) =
                    self.tile_data_address(window_line_counter, regs, video, true);
                self.vram_address = address;
                self.bus_high = vram.bank(bank).read_byte(address);
                self.tile_data_high = if P::TILE_SEL_SET_GLITCH && self.set_glitch_armed {
                    self.set_glitch_armed = false;
                    self.glitch_src_high
                } else {
                    self.tile_sel_glitched_bitplane(tile_sel_glitch_active)
                        .unwrap_or(self.bus_high)
                };
                // A TILE_SEL clear armed this fetch to snapshot the bus as the
                // glitch source (the BG tile as of the reset).
                if self.glitch_capture_armed {
                    self.glitch_src_low = self.bus_low;
                    self.glitch_src_high = self.bus_high;
                    self.glitch_capture_armed = false;
                }
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

    /// NYXU pipe load — bg shifter parallel-load + counter reset. The model
    /// applies the CGB X-flip before the planes enter the shifter.
    pub(in crate::ppu) fn load_into(&mut self, bg_shifter: &mut BgShifter<P::BgCell>) {
        let (low, high) = P::flip_bg_planes(self.bg_cell, self.tile_data_low, self.tile_data_high);
        bg_shifter.load(low, high, self.bg_cell);
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
