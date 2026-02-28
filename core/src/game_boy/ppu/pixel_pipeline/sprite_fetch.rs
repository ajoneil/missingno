// --- Sprite fetch ---

use crate::game_boy::ppu::{
    PipelineRegisters,
    memory::{Oam, Vram},
};

use super::super::sprites::{self, SpriteId, SpriteSize};
use super::super::tiles::{TileAddressMode, TileIndex};
use super::fetcher::FetcherTick;
use super::oam_scan::SpriteStoreEntry;
use super::shifters::ObjShifter;

/// The phases of a sprite fetch on real hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteFetchPhase {
    /// The BG fetcher continues advancing through its normal steps.
    /// The wait ends when the fetcher has completed GetTileDataHigh
    /// (reached Load) AND the BG shifter is non-empty — both conditions
    /// must be true simultaneously. The variable sprite penalty (0-5
    /// dots) emerges from how many fetcher steps this phase consumes.
    WaitingForFetcher,
    /// The BG fetcher is frozen at its current position. Sprite tile
    /// data is read through the SpriteStep state machine (6 dots total).
    FetchingData,
    /// Data fetch complete. Pixel clock still frozen (hardware: state_old.FEPO=1).
    /// Sprite data is merged into the OBJ shifter on this dot.
    /// Transitions to Idle on the next dot.
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpriteStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
}

pub(super) struct SpriteFetch {
    /// The sprite store entry that triggered this fetch.
    pub(super) entry: SpriteStoreEntry,
    pub(super) phase: SpriteFetchPhase,
    step: SpriteStep,
    tick: FetcherTick,
    tile_data_low: u8,
    tile_data_high: u8,
}

impl SpriteFetch {
    pub(super) fn new(entry: SpriteStoreEntry) -> Self {
        Self {
            entry,
            phase: SpriteFetchPhase::WaitingForFetcher,
            step: SpriteStep::GetTile,
            tick: FetcherTick::T1,
            tile_data_low: 0,
            tile_data_high: 0,
        }
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
    /// the fetch is complete (GetTileDataHigh T2 has fired).
    pub(super) fn advance(&mut self, regs: &PipelineRegisters, oam: &Oam, vram: &Vram) -> bool {
        match self.step {
            SpriteStep::GetTile => {
                if self.tick == FetcherTick::T1 {
                    self.tick = FetcherTick::T2;
                } else {
                    // Tile index comes from OAM via the sprite store's oam_index
                    self.tick = FetcherTick::T1;
                    self.step = SpriteStep::GetTileDataLow;
                }
            }
            SpriteStep::GetTileDataLow => {
                if self.tick == FetcherTick::T1 {
                    self.tick = FetcherTick::T2;
                } else {
                    self.tile_data_low = self.read_tile_data(regs, oam, vram, false);
                    self.tick = FetcherTick::T1;
                    self.step = SpriteStep::GetTileDataHigh;
                }
            }
            SpriteStep::GetTileDataHigh => {
                if self.tick == FetcherTick::T1 {
                    self.tick = FetcherTick::T2;
                } else {
                    self.tile_data_high = self.read_tile_data(regs, oam, vram, true);
                    return true;
                }
            }
        }
        false
    }

    /// Merge fetched sprite pixels into the OBJ shifter.
    pub(super) fn merge_into(&self, obj_shifter: &mut ObjShifter, oam: &Oam) {
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
pub(super) enum SpriteState {
    /// No sprite activity. Pixel clock runs normally.
    Idle,
    /// Sprite fetch in progress (wait + data phases).
    Fetching(SpriteFetch),
}
