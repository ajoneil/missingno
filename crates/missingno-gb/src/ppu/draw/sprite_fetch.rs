// --- Sprite fetch ---

use crate::ppu::{
    PipelineRegisters,
    memory::{Oam, Vram},
};

use super::super::scan::oam_scan::SpriteStoreEntry;
use super::super::types::sprites::{self, SpriteId, SpriteSize};
use super::super::types::tiles::{TileAddressMode, TileIndex};
use super::shifters::ObjShifter;

/// The phases of a sprite fetch on real hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteFetchPhase {
    /// The BG fetcher is frozen at its current position. Sprite tile
    /// data is read through the SpriteStep state machine (6 dots total).
    /// On hardware, the 3-bit counter (TOXE/TULY/TESE) counts 0-5,
    /// self-stopping at 5 via TAME clock gating. WUTY (fetch done)
    /// fires on the rising phase of counter=5, the same dot as the
    /// tile data HIGH read and sprite pixel merge. There is no separate
    /// "done" dot — TAKA clears on the same dot.
    FetchingData,
}

/// Active sprite data fetch. Collapses the 3-bit ripple counter
/// (TOXE/TULY/TESE, clocked by SABE = NAND2(LAPE, TAME)) into a u8
/// counting 0–5. Also collapses the fetch-done decode (WUTY =
/// NOT(VUSA), VUSA = OR2(TYNO, TYFO_n), TYNO = NAND3(SEBA, TOXE,
/// VONU) + TYFO = dffr(TOXE, LAPE)) into the `return true` at
/// counter=5 — both hardware decode branches fire at the same
/// counter-terminal state that the u8 check detects directly.
///
/// Also collapses the sprite temp-latch layer — 16 dlatch_ee_q cells
/// (plane A: PEFO/ROKA/MYTU/RAMU/SELE/SUTO/RAMA/RYDU enabled by
/// `latch_sp_bp_a`; plane B: REWO/PEBA/MOFO/PUDU/SAJA/SUNY/SEMO/SEGA
/// enabled by `latch_sp_bp_b`) that capture the VRAM data bus
/// (sp_d0..sp_d7) during each plane's data cycle. The emulator stores
/// the captured byte directly into `tile_data_low` / `tile_data_high`
/// at counter = 3 / 5 — the same dots at which hardware's latch enables
/// fire — and holds it there until the wuty pulse consumes it via
/// `merge_into`. Observation-equivalent at dot granularity; the
/// intermediate bus-to-latch-to-NAND2-pair chain is not separately
/// representable in the emulator's integer-dot timing.
pub(in crate::ppu) struct SpriteFetch {
    /// The sprite store entry that triggered this fetch.
    pub(in crate::ppu) entry: SpriteStoreEntry,
    /// Hardware counter (TOXE/TULY/TESE): 0-5 (6 dots).
    /// VRAM reads at counter 3 (tile data low) and 5 (tile data high).
    /// Self-stops at 5 via TAME clock gating.
    fetch_counter: u8,
    /// Plane-A sprite tile byte. Collapses the 8 plane-A temp latches
    /// (PEFO d0 / ROKA d1 / MYTU d2 / RAMU d3 / SELE d4 / SUTO d5 /
    /// RAMA d6 / RYDU d7) — captured at counter = 3 (latch_sp_bp_a
    /// window), held through counter = 5 until the wuty merge fires.
    tile_data_low: u8,
    /// Plane-B sprite tile byte. Collapses the 8 plane-B temp latches
    /// (REWO d0 / PEBA d1 / MOFO d2 / PUDU d3 / SAJA d4 / SUNY d5 /
    /// SEMO d6 / SEGA d7) — captured at counter = 5 (latch_sp_bp_b
    /// window, same dot as wuty fires and the merge consumes both).
    tile_data_high: u8,
}

impl SpriteFetch {
    /// Start the 6-dot sprite data fetch. The variable 0-5 dot penalty
    /// is handled by TEKY/SOBU staying low until the BG fetcher is done,
    /// not by a separate waiting state.
    pub(in crate::ppu) fn new_fetching(entry: SpriteStoreEntry) -> Self {
        Self {
            entry,
            fetch_counter: 0,
            tile_data_low: 0,
            tile_data_high: 0,
        }
    }

    pub(in crate::ppu) fn tile_data(&self) -> (u8, u8) {
        (self.tile_data_low, self.tile_data_high)
    }

    pub(in crate::ppu) fn fetch_counter(&self) -> u8 {
        self.fetch_counter
    }

    /// Read one byte of sprite tile data (low or high bitplane).
    ///
    /// Collapses the gejy → famu → ~ma4 chain: hardware's gejy AO22
    /// (XYMO-controlled mux — xuso_n for 8×8, wago = XOR(sprite_y_store_b3,
    /// wuky) for 8×16) drives the famu tri-state inverter onto bus:~ma4
    /// when abon = NOR2(tuly, vonu) OR NOT(mode3) goes low — the
    /// (tuly OR vonu) sprite tile-data fetch window during Mode 3.
    /// Emulator collapses to indexed VRAM access using live
    /// `regs.control.sprite_size()` for tile_index masking (8×16:
    /// tile.0 & 0xFE) and the row-within-sprite computation (flipped_y
    /// → final_block / final_idx). Live sprite_size read at fetch time
    /// matches gejy's combinational live-XYMO sampling; the famu enable
    /// window is implicit via the fetch_counter==3/5 read positions
    /// (called only inside SpriteState::Fetching).
    fn read_tile_data(&self, regs: &PipelineRegisters, oam: &Oam, vram: &Vram, high: bool) -> u8 {
        let sprite = oam.sprite(SpriteId(self.entry.oam_index));
        let tile_index = if regs.control.sprite_size() == SpriteSize::Double {
            TileIndex(sprite.tile.0 & 0xFE)
        } else {
            sprite.tile
        };
        let (block_id, mapped_idx) = TileAddressMode::Block0Block1.tile(tile_index);

        let flipped_y = if sprite.attributes.flip_y() {
            (regs.control.sprite_size().height() as i16 - 1 - self.entry.line_offset as i16) as u8
        } else {
            self.entry.line_offset
        };

        let (final_block, final_idx, final_y) = if flipped_y < 8 {
            (block_id, mapped_idx, flipped_y)
        } else {
            (block_id, TileIndex(mapped_idx.0 + 1), flipped_y - 8)
        };

        let block = vram.tile_block(final_block);
        block.data[final_idx.0 as usize * 16 + final_y as usize * 2 + high as usize]
    }

    /// Advance the sprite fetch pipeline by one dot. Returns `true` when
    /// the fetch is complete (fetch_counter == 5, tile data high read).
    pub(in crate::ppu) fn advance(
        &mut self,
        regs: &PipelineRegisters,
        oam: &Oam,
        vram: &Vram,
    ) -> bool {
        match self.fetch_counter {
            3 => {
                // Tile data low VRAM read.
                self.tile_data_low = self.read_tile_data(regs, oam, vram, false);
            }
            5 => {
                // Tile data high VRAM read. Fetch complete.
                self.tile_data_high = self.read_tile_data(regs, oam, vram, true);
                return true;
            }
            _ => {
                // GetTile wait (0, 1) and data wait (2, 4): no VRAM action.
            }
        }
        self.fetch_counter += 1;
        false
    }

    /// Parallel-load the sprite temp-latch content into the OBJ shifter.
    ///
    /// Called on the wuty pulse (fetch complete at counter = 5). Hardware
    /// wuty = NOT(vusa) rises, xefy = NOT(wuty) falls, the per-stage
    /// sprite_onN gates assert at transparent shifter positions, and the
    /// NAND2 pair at each dffsr's s_n / r_n drives the parallel-load from
    /// the temp-latch content. Merge is transparency-conditional per stage
    /// (see `ObjShifter::merge`).
    ///
    /// X-flip collapses hardware's bit-order reversal during load: the
    /// shifter always reads its stage-7 Q as the leftmost pixel, so flipped
    /// sprites must have their bit-order reversed going into the parallel-
    /// load path. `reverse_bits` on the temp-latch bytes is the emulator's
    /// equivalent of that reversal at the load boundary.
    ///
    /// Palette and priority bits come directly from the sprite's OAM
    /// attributes (DEPO captures the priority bit from OAM-A bit 7), not
    /// from the shift-register chain — they are broadcast uniformly to
    /// all 8 stages that the transparency gate allows to load.
    pub(in crate::ppu) fn merge_into(&self, obj_shifter: &mut ObjShifter, oam: &Oam) {
        let sprite = oam.sprite(SpriteId(self.entry.oam_index));

        // X-flip: hardware reverses the bit order when loading the shift
        // register. For normal sprites, MSB shifts out first (leftmost pixel).
        // For flipped sprites, LSB shifts out first — achieved by reversing
        // the byte's bit order before loading.
        let sprite_low = if sprite.attributes.flip_x() {
            self.tile_data_low.reverse_bits()
        } else {
            self.tile_data_low
        };
        let sprite_high = if sprite.attributes.flip_x() {
            self.tile_data_high.reverse_bits()
        } else {
            self.tile_data_high
        };

        let palette_bit = if sprite.attributes.contains(sprites::Attributes::PALETTE) {
            1
        } else {
            0
        };
        let priority_bit = if sprite.attributes.contains(sprites::Attributes::PRIORITY) {
            1
        } else {
            0
        };

        obj_shifter.merge(sprite_low, sprite_high, palette_bit, priority_bit);
    }
}

/// Sprite fetch lifecycle. On hardware, FEPO (sprite X match) freezes
/// the pixel clock, the fetch runs, then the pixel clock resumes
/// normally on the next dot (state_old.FEPO=0).
pub(in crate::ppu) enum SpriteState {
    /// No sprite activity. Pixel clock runs normally.
    Idle,
    /// Sprite fetch in progress (wait + data phases).
    Fetching(SpriteFetch),
}
