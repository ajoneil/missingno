use crate::dma::OamBusOwner;
use crate::ppu::{
    PpuModel,
    memory::{Oam, Vram},
};

use super::super::scan::oam_scan::SpriteStoreEntry;
use super::super::types::sprites::{self, SpriteId, SpriteSize};
use super::super::types::tiles::{TileAddressMode, TileIndex};

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

    /// Resolves the tile row from `sprite_size` — live LCDC.2 on DMG, the
    /// crossing-lagged obj-size on CGB, so a mid-fetch 8x8↔8x16 change splits the
    /// low (counter-2) and high (counter-4) bitplane reads across tile rows.
    fn read_tile_data<P: PpuModel>(
        &mut self,
        model: &P,
        sprite_size: SpriteSize,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &P::Vram,
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

        let tile_index = if sprite_size == SpriteSize::Double {
            TileIndex(tile.0 & 0xFE)
        } else {
            tile
        };
        let (block_id, mapped_idx) = TileAddressMode::Block0Block1.tile(tile_index);

        let flipped_y = if attributes.flip_y() {
            (sprite_size.height() as i16 - 1 - self.entry.line_offset as i16) as u8
        } else {
            self.entry.line_offset
        };

        let (final_block, final_idx, final_y) = match sprite_size {
            SpriteSize::Single => (block_id, mapped_idx, flipped_y & 0x07),
            SpriteSize::Double if flipped_y < 8 => (block_id, mapped_idx, flipped_y),
            SpriteSize::Double => (block_id, TileIndex(mapped_idx.0 + 1), flipped_y - 8),
        };

        // CGB objects select their tile-data VRAM bank from OAM attr bit 3.
        let block = vram
            .bank(model.obj_data_bank(self.attributes))
            .tile_block(final_block);
        block.data[final_idx.0 as usize * 16 + final_y as usize * 2 + high as usize]
    }

    /// Returns true on completion (counter==5).
    pub(in crate::ppu) fn advance<P: PpuModel>(
        &mut self,
        model: &P,
        sprite_size: SpriteSize,
        oam: &Oam,
        oam_bus: OamBusOwner,
        vram: &P::Vram,
    ) -> bool {
        match self.fetch_counter {
            2 => {
                self.tile_data_low =
                    self.read_tile_data(model, sprite_size, oam, oam_bus, vram, false);
            }
            4 => {
                self.tile_data_high =
                    self.read_tile_data(model, sprite_size, oam, oam_bus, vram, true);
            }
            5 => {
                return true;
            }
            _ => {}
        }
        self.fetch_counter += 1;
        false
    }

    /// Merge fetched bytes into the model's OBJ FIFO (transparency-gated; X-flip
    /// reverses bits). The model extracts the per-pixel attribute and resolves the
    /// overlap (DMG fetch-order / CGB OAM-index) — the FIFO is opaque here.
    pub(in crate::ppu) fn merge_into<P: PpuModel>(&self, model: &P, fifo: &mut P::ObjFifo) {
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

        let attr = model.obj_attr(self.attributes);
        model.obj_merge(fifo, sprite_low, sprite_high, attr, self.slot_index);
    }
}

/// FEPO (sprite X match) freezes SACU; the fetch runs; SACU resumes on the next dot.
pub(in crate::ppu) enum SpriteState {
    Idle,
    Fetching(SpriteFetch),
}
