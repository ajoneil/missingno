//! The CPU read latch: the pre-ALET samples a pending read's `data_phase_n↑`
//! edge saw, and the value they resolve that read to.

use crate::Cgb;

impl Cgb {
    /// Sample XYMU before this dot's ALET rise.
    pub(crate) fn set_pre_ppu_clock_rendering(&mut self, rendering: bool) {
        if self.double_speed {
            self.pre_ppu_clock_rendering = rendering;
        }
    }

    /// Sample the read lock before this dot's ALET rise.
    pub(crate) fn set_pre_ppu_clock_lock(&mut self, lock: Option<bool>) {
        if self.double_speed {
            self.pre_ppu_clock_lock = lock;
        }
    }

    pub(crate) fn set_read_drive_lock(&mut self, oam_lock: Option<bool>) {
        self.read_drive_oam_lock = oam_lock;
    }

    pub(crate) fn set_ff44_ripple_old(&mut self, ly: Option<u8>) {
        self.ff44_ripple_old = ly;
    }

    pub(crate) fn take_ff44_ripple(&mut self) -> Option<u8> {
        self.ff44_ripple_old.take()
    }

    pub(crate) fn resolve_latched_read(
        &self,
        address: u16,
        value: u8,
        latch_lock: Option<bool>,
    ) -> u8 {
        match address {
            // Double-speed STAT mode bits: the read's data_phase_n↑ latches
            // before this dot's ALET edge, where VOGA clears XYMU (mode 3→0).
            // So a read taken while the PPU was rendering just before that edge
            // reads mode 3 even though the post-edge live mode has already
            // fallen to 0. This is the CGB CPU↔ALET half-dot phase — distinct
            // from the DMG, whose lockstep timing lands the latch after the edge.
            0xFF41 if self.double_speed => {
                if self.pre_ppu_clock_rendering {
                    value | 0b11
                } else {
                    value
                }
            }
            // Single speed: OR-of-accessibility over the drive-enable grant
            // sample and the latch-edge lock — the bus keeps the byte OAM
            // drove while addressed and unlocked. (The earlier address-phase
            // grant is double-speed-only; a single-speed onset between the
            // address phase and tobe↑ still floats the read.)
            0xFE00..=0xFEFF if !self.double_speed => match (self.read_drive_oam_lock, latch_lock) {
                (Some(false), _) => value,
                (_, Some(true)) => 0xFF,
                _ => value,
            },
            // Double-speed VRAM/OAM lock: data_phase_n↑ latches before this dot's
            // ALET edge — the same CGB CPU↔ALET half-dot phase as the STAT mode bits.
            // The read floats if it was locked at the pre-ALET view OR at the latch
            // edge, so a mode-3→0 release landing between them still floats. OR like
            // the single-speed OAM grant/latch arm — never removes a lock the latch sees.
            0x8000..=0x9FFF | 0xFE00..=0xFEFF if self.double_speed => {
                if self.pre_ppu_clock_lock == Some(true) || latch_lock == Some(true) {
                    0xFF
                } else {
                    value
                }
            }
            // An HDMA idle claim (a wake-tenure-consumed entry whose block is
            // owed but unserviced) holds the VRAM select without driving
            // data: an unlocked CPU VRAM read captures the undriven bus.
            0x8000..=0x9FFF if latch_lock != Some(true) && self.vram_dma.arb.idle_claim => 0x00,
            // A seized block tenure owns the VRAM select against the PPU: a
            // CPU VRAM read during a wake drain (the only tenure outside
            // mode 0) sees the actual byte, not the mode-3 float.
            0x8000..=0x9FFF
                if latch_lock == Some(true)
                    && self.vram_dma.block.remaining > 0
                    && self.vram_dma.cursor.remaining > 0 =>
            {
                value
            }
            _ if latch_lock == Some(true) => 0xFF,
            _ => value,
        }
    }
}
