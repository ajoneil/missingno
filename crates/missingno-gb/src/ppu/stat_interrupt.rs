//! STAT register state and LALU edge-detection.
//! LYC pipeline: PALY (LY==LYC) → ROPO (TALU-rising DFF) → RUPO (transparent NOR latch).

use bitflags::bitflags;

bitflags! {
    #[derive(Copy, Clone)]
    pub struct InterruptFlags: u8 {
        const DUMMY                = 0b10000000;
        const CURRENT_LINE_COMPARE = 0b01000000;
        const OAM_SCAN             = 0b00100000;
        const VERTICAL_BLANK       = 0b00010000;
        const HORIZONTAL_BLANK     = 0b00001000;
    }
}

pub struct StatInterrupt {
    /// LYC register ($FF45).
    pub(in crate::ppu) lyc: u8,
    /// PALY combinational comparator; recomputed at TALU fall and on LYC writes.
    pub(in crate::ppu) comparison_pending: bool,
    /// ROPO DFF — latched LY==LYC. Reset only by SYS_RST (not VID_RST). Drives STAT bit 2 via transparent RUPO.
    pub(in crate::ppu) comparison_latched: bool,
    /// FF41 bits 3-6 enables + DUMMY pull-up on bit 7.
    pub(in crate::ppu) enables: InterruptFlags,
    /// LALU previous STAT line state.
    pub(in crate::ppu) line_was_high: bool,
}

impl StatInterrupt {
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            lyc: 0,
            comparison_pending: true,
            comparison_latched: true,
            enables: InterruptFlags::DUMMY,
            line_was_high: false,
        }
    }

    /// PALY recompute: `pending = (ly == lyc)`.
    pub(in crate::ppu) fn update_comparison(&mut self, ly: u8) {
        self.comparison_pending = ly == self.lyc;
    }

    /// ROPO captures comparison_pending on TALU rising.
    pub(in crate::ppu) fn latch_comparison(&mut self) {
        self.comparison_latched = self.comparison_pending;
    }

    /// ROPO.Q — drives STAT bit 2 via the transparent RUPO latch, and also feeds the
    /// LYC-match arm of the STAT-interrupt edge detector.
    pub(in crate::ppu) fn ly_eq_lyc(&self) -> bool {
        self.comparison_latched
    }

    pub(in crate::ppu) fn lyc(&self) -> u8 {
        self.lyc
    }

    pub(in crate::ppu) fn enables(&self) -> InterruptFlags {
        self.enables
    }

    pub(in crate::ppu) fn line_was_high(&self) -> bool {
        self.line_was_high
    }

    /// Used by the STAT write glitch path to install the transient all-bits-high state.
    pub(in crate::ppu) fn set_enables(&mut self, flags: InterruptFlags) {
        self.enables = flags;
    }

    pub(in crate::ppu) fn write_lyc(&mut self, value: u8, ly: u8) {
        self.lyc = value;
        self.update_comparison(ly);
    }

    pub(in crate::ppu) fn write_stat_bits(&mut self, value: u8) {
        self.enables = InterruptFlags::from_bits_truncate(value);
    }

    /// LALU edge detect: returns true on rising edge, updates `line_was_high`.
    pub(in crate::ppu) fn detect_line_edge(&mut self, stat_line_high: bool) -> bool {
        let edge = stat_line_high && !self.line_was_high;
        self.line_was_high = stat_line_high;
        edge
    }

    /// Prime the edge detector at LCD-enable to avoid a spurious first edge.
    pub(in crate::ppu) fn set_line_was_high(&mut self, value: bool) {
        self.line_was_high = value;
    }
}
