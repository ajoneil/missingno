use crate::dma::OamBusOwner;
use crate::ppu::{
    PipelineRegisters,
    memory::{Oam, Vram},
};

use super::super::scan::oam_scan::SpriteStoreEntry;
use super::super::types::sprites::{self, SpriteId, SpriteSize};
use super::super::types::tiles::{TileAddressMode, TileIndex};
use super::shifters::ObjShifter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteFetchPhase {
    /// BG fetcher frozen; 6-dot sprite data read. WUTY fires at counter=5 on the same dot as tile data HIGH read.
    FetchingData,
}

/// 6-dot sprite data fetch. Collapses the 3-bit ripple counter (TOXE/TULY/TESE) into a u8,
/// the fetch-done decode (WUTY) into the counter==5 return, and the 16 sprite temp-latch cells
/// into `tile_data_low` / `tile_data_high`.
pub(in crate::ppu) struct SpriteFetch {
    pub(in crate::ppu) entry: SpriteStoreEntry,
    /// Used to set the per-slot fetched-flag at WUTY↑.
    pub(in crate::ppu) slot_index: u8,
    /// 0-5; self-stops at 5 via TAME clock gating. VRAM reads at 2 (low) and 4 (high).
    fetch_counter: u8,
    /// Plane-A tile byte captured at counter=2.
    tile_data_low: u8,
    /// Plane-B tile byte captured at counter=4.
    tile_data_high: u8,
    /// OAM bank-A attributes latched at fetch time and held through the merge dot.
    attributes: sprites::Attributes,
    /// (tile-index, attribute) = (OAM[K*4+2], OAM[K*4+3]) — the byte-pair this
    /// fetch latches into the Stage-1 dlatches (XYKY/YDYV) shared with the
    /// Mode-2 scan; becomes the held value into a following DMA-overlapped Mode 2.
    stage1_capture: (u8, u8),
}

impl SpriteFetch {
    /// The variable 0-5 dot penalty is implicit in TEKY/SOBU staying low until BG fetch is done.
    pub(in crate::ppu) fn new_fetching(entry: SpriteStoreEntry, slot_index: u8) -> Self {
        Self {
            entry,
            slot_index,
            fetch_counter: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            attributes: sprites::Attributes(0),
            stage1_capture: (0, 0),
        }
    }

    pub(in crate::ppu) fn tile_data(&self) -> (u8, u8) {
        (self.tile_data_low, self.tile_data_high)
    }

    /// The (tile-index, attribute) OAM byte-pair this fetch latched into Stage-1.
    pub(in crate::ppu) fn stage1_capture(&self) -> (u8, u8) {
        self.stage1_capture
    }

    pub(in crate::ppu) fn fetch_counter(&self) -> u8 {
        self.fetch_counter
    }

    /// Reads `sprite_size` live at fetch time, matching the combinational gejy/XYMO path on hardware.
    fn read_tile_data(
        &mut self,
        regs: &PipelineRegisters,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &Vram,
        high: bool,
    ) -> u8 {
        let (tile, attributes) = match oam_bus {
            OamBusOwner::Ppu => {
                let sprite = oam.sprite(SpriteId(self.entry.oam_index));
                (sprite.tile, sprite.attributes)
            }
            OamBusOwner::Dma(addr) => {
                let aligned = addr & 0xFE;
                (
                    TileIndex(oam.oam_byte(aligned)),
                    sprites::Attributes(oam.oam_byte(aligned | 0x01)),
                )
            }
        };
        self.attributes = attributes;
        self.stage1_capture = (tile.0, attributes.0);

        let tile_index = if regs.control.sprite_size() == SpriteSize::Double {
            TileIndex(tile.0 & 0xFE)
        } else {
            tile
        };
        let (block_id, mapped_idx) = TileAddressMode::Block0Block1.tile(tile_index);

        let flipped_y = if attributes.flip_y() {
            (regs.control.sprite_size().height() as i16 - 1 - self.entry.line_offset as i16) as u8
        } else {
            self.entry.line_offset
        };

        let (final_block, final_idx, final_y) = match regs.control.sprite_size() {
            SpriteSize::Single => (block_id, mapped_idx, flipped_y & 0x07),
            SpriteSize::Double if flipped_y < 8 => (block_id, mapped_idx, flipped_y),
            SpriteSize::Double => (block_id, TileIndex(mapped_idx.0 + 1), flipped_y - 8),
        };

        let block = vram.tile_block(final_block);
        block.data[final_idx.0 as usize * 16 + final_y as usize * 2 + high as usize]
    }

    /// Returns true on completion (counter==5).
    pub(in crate::ppu) fn advance(
        &mut self,
        regs: &PipelineRegisters,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &Vram,
    ) -> bool {
        match self.fetch_counter {
            2 => {
                self.tile_data_low = self.read_tile_data(regs, oam, oam_bus, vram, false);
            }
            4 => {
                self.tile_data_high = self.read_tile_data(regs, oam, oam_bus, vram, true);
            }
            5 => {
                return true;
            }
            _ => {}
        }
        self.fetch_counter += 1;
        false
    }

    /// Merge fetched bytes into ObjShifter via sprite_onN transparency gating; X-flip reverses bits.
    pub(in crate::ppu) fn merge_into(&self, obj_shifter: &mut ObjShifter) {
        let sprite_low = if self.attributes.flip_x() {
            self.tile_data_low.reverse_bits()
        } else {
            self.tile_data_low
        };
        let sprite_high = if self.attributes.flip_x() {
            self.tile_data_high.reverse_bits()
        } else {
            self.tile_data_high
        };

        let palette_bit = if self.attributes.contains(sprites::Attributes::PALETTE) {
            1
        } else {
            0
        };
        let priority_bit = if self.attributes.contains(sprites::Attributes::PRIORITY) {
            1
        } else {
            0
        };

        obj_shifter.merge(sprite_low, sprite_high, palette_bit, priority_bit);
    }
}

/// FEPO (sprite X match) freezes SACU; the fetch runs; SACU resumes on the next dot.
pub(in crate::ppu) enum SpriteState {
    Idle,
    Fetching(SpriteFetch),
}
