// --- Sprite store and OAM scanner ---

use crate::game_boy::ppu::{
    PipelineRegisters,
    memory::Oam,
};

use super::fetcher::FetcherTick;

use crate::game_boy::ppu::sprites::SpriteId;

pub(super) const MAX_SPRITES_PER_LINE: usize = 10;

/// One entry in the hardware's 10-slot sprite store register file.
/// Written during Mode 2 OAM scan, read during Mode 3 sprite fetch.
#[derive(Clone, Copy)]
pub(super) struct SpriteStoreEntry {
    /// OAM sprite number (0-39). The hardware stores this as a 6-bit
    /// value. Used during Mode 3 to look up tile index and attributes
    /// from OAM via the sprite fetcher.
    pub(super) oam_index: u8,
    /// Which row of the sprite falls on this scanline (0-15).
    /// Pre-computed during Mode 2 so the sprite fetcher can generate
    /// a VRAM tile address without re-reading OAM Y position.
    pub(super) line_offset: u8,
    /// X position (the raw x_plus_8 value from OAM byte 1).
    /// Compared against the pixel position counter by the X matchers
    /// during Mode 3.
    pub(super) x: u8,
}

/// The hardware's 10-entry sprite store. Populated during Mode 2 OAM scan,
/// consumed during Mode 3 by the X matchers and sprite fetcher.
pub(super) struct SpriteStore {
    pub(super) entries: [SpriteStoreEntry; MAX_SPRITES_PER_LINE],
    /// Number of entries written during this line's OAM scan (0-10).
    pub(super) count: u8,
    /// Bitmask of which store slots have been fetched during Mode 3.
    /// Bit N set = slot N already consumed. On hardware, each slot has
    /// an independent reset flag (EBOJ-FONO). Reset at line start.
    pub(super) fetched: u16,
}

impl SpriteStore {
    pub(super) fn new() -> Self {
        Self {
            entries: [SpriteStoreEntry {
                oam_index: 0,
                line_offset: 0,
                x: 0,
            }; MAX_SPRITES_PER_LINE],
            count: 0,
            fetched: 0,
        }
    }
}

// --- OAM scanner ---

/// Hardware OAM scanner (YFEL-FONY scan counter + comparison logic).
/// Processes one OAM entry every 2 dots during Mode 2, reading Y and X
/// from OAM, comparing Y against LY, and writing matches into the
/// sprite store.
pub(super) struct OamScanner {
    /// Which OAM entry to process next (0-39). Increments every 2 dots.
    entry: u8,
    /// Which half of the 2-dot scanner clock cycle we're in.
    tick: FetcherTick,
}

impl OamScanner {
    pub(super) fn new() -> Self {
        Self {
            entry: 0,
            tick: FetcherTick::T1,
        }
    }

    /// Process one dot of OAM scanning. On even dots, the scan counter
    /// drives the OAM address and OAM outputs data; on odd dots, the Y
    /// comparison fires and matches are written to the sprite store.
    ///
    /// Only bytes 0–1 (Y, X) are read from OAM during scanning — the
    /// hardware's 16-bit OAM bus provides both in a single access. Tile
    /// index and attributes (bytes 2–3) are not accessed until Mode 3.
    pub(super) fn scan_next_entry(
        &mut self,
        line_number: u8,
        sprites: &mut SpriteStore,
        regs: &PipelineRegisters,
        oam: &Oam,
    ) {
        if self.tick == FetcherTick::T1 {
            self.tick = FetcherTick::T2;
        } else {
            if (sprites.count as usize) < MAX_SPRITES_PER_LINE {
                // OAM bus read: only Y (byte 0) and X (byte 1).
                let (y_plus_16, x_plus_8) = oam.sprite_position(SpriteId(self.entry));

                // Y comparison (hardware subtractor ERUC–WUHU):
                // Computes delta = LY + 16 - sprite_Y using wrapping
                // arithmetic (matching the 8-bit hardware subtractor).
                // Match when delta < height (8 or 16 per LCDC.2).
                // Bits 0–3 of delta are the sprite line offset — the
                // same value drives the sprite store's line register.
                let delta = line_number.wrapping_add(16).wrapping_sub(y_plus_16);
                let height = regs.control.sprite_size().height();
                if delta < height {
                    let line_offset = delta;
                    sprites.entries[sprites.count as usize] = SpriteStoreEntry {
                        oam_index: self.entry,
                        line_offset,
                        x: x_plus_8,
                    };
                    sprites.count += 1;
                }
            }
            self.entry += 1;
            self.tick = FetcherTick::T1;
        }
    }

    /// Hardware FETO_SCAN_DONE signal. Fires when the scan counter
    /// has processed all 40 OAM entries.
    pub(super) fn done(&self) -> bool {
        self.entry >= 40
    }

    /// The byte address the scanner is currently driving on the OAM bus.
    /// Hardware: OAM_A[7:2] = scan_counter, OAM_A[1:0] = 0.
    pub(super) fn oam_address(&self) -> u8 {
        self.entry * 4
    }
}
