// --- Sprite store and OAM scanner ---

use crate::game_boy::ppu::{PipelineRegisters, memory::Oam};

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

/// OAM scan counter (YFEL-FONY) with Y comparison logic.
/// Processes one OAM entry every 2 dots during Mode 2, reading Y and X
/// from OAM, comparing Y against LY, and writing matches into the
/// sprite store.
///
/// The scan counter is clocked by GAVA = OR(FETO_SCAN_DONE, XUPY).
/// On hardware, GAVA provides rising edges every 2 dots (via XUPY).
/// When FETO fires (counter == 39), GAVA stays high — no more rising
/// edges, counter frozen. The current entry's comparison still
/// completes because the comparison logic runs on separate clocks
/// (COTA, WUDA) that aren't gated by FETO.
///
/// At the dot level, each XUPY tick compares the current entry and
/// then increments the counter. The caller gates ticks on XUPY
/// rising and !scan_done() (modeling GAVA freeze).
pub(super) struct ScanCounter {
    /// 6-bit scan counter (YFEL-FONY). Drives OAM address and indexes
    /// the current entry for comparison. Range 0-39, frozen at 39
    /// once FETO fires.
    entry: u8,
    /// Models GAVA held high by FETO (OR gate output latched). Once
    /// FETO fires (counter == 39), GAVA stays permanently high — no
    /// more rising edges reach the counter, freezing it.
    frozen: bool,
}

impl ScanCounter {
    pub(super) fn new() -> Self {
        Self {
            entry: 0,
            frozen: false,
        }
    }

    /// Reset the scan counter to 0 (ANOM_LINE_RST). Called at scanline
    /// boundaries — the counter is never destroyed, just reset.
    pub(super) fn reset(&mut self) {
        self.entry = 0;
        self.frozen = false;
    }

    /// Process one scan tick. On hardware, GAVA clocks the counter
    /// and COTA latches OAM data on the same sub-phase (A/E), but
    /// COTA latches the *previous* tick's data (pipeline delay).
    /// At dot granularity this collapses to: compare current entry,
    /// then increment the counter for the next tick.
    ///
    /// The caller must gate calls on XUPY rising. FETO freezes
    /// only the counter increment, not the comparison — entry 39
    /// is still compared even after FETO fires.
    ///
    /// Only bytes 0–1 (Y, X) are read from OAM during scanning — the
    /// hardware's 16-bit OAM bus provides both in a single access.
    /// Tile index and attributes (bytes 2–3) are not accessed until
    /// Mode 3.
    /// Advance the scan counter clock (GAVA). On hardware, the counter
    /// is clocked by XUPY gated only by !VID_RST, not by BESU (scanning
    /// latch). The counter runs whenever the LCD is enabled, including
    /// the LCD-on first line where scanning never starts.
    pub(super) fn tick_clock(&mut self) {
        // GAVA freeze: once FETO fires, latch frozen=true so the
        // counter never increments again this scanline. On hardware,
        // FETO feeds back into GAVA's OR gate, holding the clock high.
        if self.scan_done() {
            self.frozen = true;
        }

        // Counter increment (GAVA), gated by frozen. Once FETO has
        // fired and set frozen=true, no more rising edges reach the
        // counter — it stays at 39 for the rest of the scanline.
        if !self.frozen {
            self.entry += 1;
        }
    }

    /// Y comparison and sprite store write (COTA/WUDA). Only runs
    /// when scanning is active — OAM access requires BESU. The counter
    /// tick (`tick_clock`) must be called separately.
    pub(super) fn compare_and_store(
        &mut self,
        line_number: u8,
        sprites: &mut SpriteStore,
        regs: &PipelineRegisters,
        oam: &Oam,
    ) {
        if (sprites.count as usize) < MAX_SPRITES_PER_LINE {
            let (y_plus_16, x_plus_8) = oam.sprite_position(SpriteId(self.entry));

            let delta = line_number.wrapping_add(16).wrapping_sub(y_plus_16);
            let height = regs.control.sprite_size().height();
            let is_match = delta < height;

            if is_match {
                let line_offset = delta;
                sprites.entries[sprites.count as usize] = SpriteStoreEntry {
                    oam_index: self.entry,
                    line_offset,
                    x: x_plus_8,
                };
                sprites.count += 1;
            }
        }
    }

    /// Hardware FETO_SCAN_DONE signal (combinational). AND4 of scan
    /// counter bits 0, 1, 2, and 5 — fires when counter == 39
    /// (0b100111). On hardware this is true as soon as the counter
    /// reaches 39, before entry 39's comparison completes.
    pub(super) fn scan_done(&self) -> bool {
        self.entry & 0b100111 == 0b100111
    }

    /// Current scan counter entry (0-39).
    pub(super) fn entry(&self) -> u8 {
        self.entry
    }

    /// The byte address the scanner is currently driving on the OAM bus.
    /// Hardware: OAM_A[7:2] = scan_counter, OAM_A[1:0] = 0.
    pub(super) fn oam_address(&self) -> u8 {
        self.entry * 4
    }
}
