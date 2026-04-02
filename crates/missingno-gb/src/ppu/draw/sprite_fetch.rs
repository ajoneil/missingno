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

/// Active sprite data fetch. Models the 3-bit counter (TOXE/TULY/TESE)
/// that counts 0-5 over 6 dots, performing OAM reads (counter 0-1) and
/// VRAM reads (counter 3 and 5) for sprite tile data.
pub(in crate::ppu) struct SpriteFetch {
    /// The sprite store entry that triggered this fetch.
    pub(in crate::ppu) entry: SpriteStoreEntry,
    /// Hardware counter (TOXE/TULY/TESE): 0-5 (6 dots).
    /// VRAM reads at counter 3 (tile data low) and 5 (tile data high).
    /// Self-stops at 5 via TAME clock gating.
    fetch_counter: u8,
    tile_data_low: u8,
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
    /// On the die, the sprite fetcher (page 29) uses the OAM index
    /// from the sprite store to look up the tile index and attributes,
    /// then generates a VRAM address from the tile index, line offset,
    /// and flip flags. The VRAM interface (page 25) performs the read.
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

    /// Merge fetched sprite pixels into the OBJ shifter.
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
