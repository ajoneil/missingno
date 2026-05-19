use crate::dma::OamBusOwner;
use crate::ppu::{PipelineRegisters, memory::Oam};

use crate::ppu::types::sprites::SpriteId;

pub(in crate::ppu) const MAX_SPRITES_PER_LINE: usize = 10;

#[derive(Clone, Copy)]
pub(in crate::ppu) struct SpriteStoreEntry {
    /// OAM sprite number (0-39).
    pub(in crate::ppu) oam_index: u8,
    /// Row of the sprite on this scanline (0-15); precomputed during Mode 2.
    pub(in crate::ppu) line_offset: u8,
    /// x from OAM byte 1; collapses the per-slot BODE-clocked X dlatches into one u8.
    pub(in crate::ppu) x: u8,
}

pub(in crate::ppu) struct SpriteStore {
    pub(in crate::ppu) entries: [SpriteStoreEntry; MAX_SPRITES_PER_LINE],
    pub(in crate::ppu) count: u8,
    /// Bit N = slot N fetched (EBOJ-FONO per-slot reset flag); reset at line start.
    pub(in crate::ppu) fetched: u16,
}

impl SpriteStore {
    pub(in crate::ppu) fn new() -> Self {
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

    /// True iff ≥5 sprites share an X position where `(X + scx) mod 8 == 0` —
    /// the `(K+S) mod 8 = 0` worst-case sprite-fetch alignment. Used at AVAP
    /// to detect the DMG-CPU-08 startup-acceleration quirk.
    pub(in crate::ppu) fn has_worst_case_stacked_cluster_at(&self, scx: u8) -> bool {
        let entries = &self.entries[..self.count as usize];
        entries.iter().any(|entry| {
            entry.x.wrapping_add(scx) & 7 == 0
                && entries.iter().filter(|other| other.x == entry.x).count() >= 5
        })
    }
}

/// YFEL-FONY 6-bit scan counter with combinational Y comparator. Clocked by GAVA = OR2(XUPY, FETO);
/// freezes at 39 when FETO holds GAVA high.
pub(in crate::ppu) struct ScanCounter {
    entry: u8,
    /// GAVA held high by FETO; counter frozen at 39.
    frozen: bool,
    /// Stage-1 Y-byte latch on bank-B OAM data bus.
    stage1_y: u8,
    /// Stage-1 X-byte latch on bank-A OAM data bus.
    stage1_x: u8,
}

impl ScanCounter {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            entry: 0,
            frozen: false,
            stage1_y: 0,
            stage1_x: 0,
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            entry: 39,
            frozen: true,
            stage1_y: 0,
            stage1_x: 0,
        }
    }

    /// ANOM_LINE_RST.
    pub(in crate::ppu) fn reset(&mut self) {
        self.entry = 0;
        self.frozen = false;
    }

    /// GAVA tick. Counter runs whenever LCD is enabled (gated by !VID_RST, not BESU); freezes at 39.
    pub(in crate::ppu) fn tick_clock(&mut self) {
        if self.scan_done() {
            self.frozen = true;
        }

        if !self.frozen {
            self.entry += 1;
        }
    }

    /// Y compare + sprite-store write. Combinational on hardware (8-stage carry chain + NAND6);
    /// collapsed here to an arithmetic predicate. Caller gates on `scanning`.
    pub(in crate::ppu) fn compare_and_store(
        &mut self,
        line_number: u8,
        sprites: &mut SpriteStore,
        regs: &PipelineRegisters,
        oam: &Oam,
        oam_bus: OamBusOwner,
    ) {
        // Stage-1 latch holds when bus is DMA-owned (oam_data_latch gated off).
        if let OamBusOwner::Ppu = oam_bus {
            let (y, x) = oam.sprite_position(SpriteId(self.entry));
            self.stage1_y = y;
            self.stage1_x = x;
        }

        if (sprites.count as usize) < MAX_SPRITES_PER_LINE {
            let delta = line_number.wrapping_add(16).wrapping_sub(self.stage1_y);
            let height = regs.control.sprite_size().height();
            let is_match = delta < height;

            if is_match {
                let line_offset = delta;
                sprites.entries[sprites.count as usize] = SpriteStoreEntry {
                    oam_index: self.entry,
                    line_offset,
                    x: self.stage1_x,
                };
                sprites.count += 1;
            }
        }
    }

    /// FETO_SCAN_DONE: AND4 of scan counter bits 0/1/2/5 — fires at entry==39 (0b100111).
    pub(in crate::ppu) fn scan_done(&self) -> bool {
        self.entry & 0b100111 == 0b100111
    }

    pub(in crate::ppu) fn entry(&self) -> u8 {
        self.entry
    }

    /// OAM_A[7:2] = scan_counter, OAM_A[1:0] = 0.
    pub(in crate::ppu) fn oam_address(&self) -> u8 {
        self.entry * 4
    }
}
