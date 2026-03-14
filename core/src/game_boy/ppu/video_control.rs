use bitflags::bitflags;

bitflags! {
    pub struct InterruptFlags: u8 {
        const DUMMY                = 0b10000000;
        const CURRENT_LINE_COMPARE = 0b01000000;
        const OAM_SCAN             = 0b00100000;
        const VERTICAL_BLANK       = 0b00010000;
        const HORIZONTAL_BLANK     = 0b00001000;
    }
}

/// Video control (schematic page 21): the LY counter, LYC comparator,
/// ROPO comparison latch, and STAT interrupt enable flags. Bidirectional —
/// the pipeline writes LY, the CPU writes LYC and STAT flags, and the
/// interrupt logic reads the latched comparison result. These signals
/// sit together on the die's video control section.
pub struct VideoControl {
    /// LX counter (SAXO-TYRY ripple counter, page 21).
    /// Counts 0-113 per scanline. Clocked by TALU (every 4 dots).
    /// SANU fires at LX=113, clocking RUTU (line-end).
    pub(super) lx: u8,

    /// Sub-LX phase counter modeling WUVU/VENA divider state.
    /// Counts 0-3 within each LX value (one increment per dot).
    /// Phase 0 = TALU rising edge (LX increment point).
    pub(super) phase: u8,

    /// LY counter (MUWY-LAFO ripple counter, page 21). Clocked by RUTU
    /// at LX=113, counting 0–153 and wrapping. On line 153, MYTA
    /// (frame-end DFF, clocked by NYPE one half-cycle after RUTU) drives
    /// LAMA low, resetting all LY bits to 0. The CPU sees LY=153 only
    /// during the first M-cycle (LX=0); from LX=1 onward, `ly()`
    /// returns 0.
    pub(super) ly: u8,

    /// LYC register (FF45). CPU-writable comparison value.
    pub(super) lyc: u8,

    /// Raw LY==LYC comparison from the current M-cycle (PALY_LY_MATCHa).
    /// On the next M-cycle, this is promoted to `ly_eq_lyc` (the ROPO
    /// output). Models the `_old` input to ROPO DFF17 — ROPO always
    /// samples the previous cycle's comparison result.
    pub(super) ly_match_pending: bool,

    /// Latched LY==LYC comparison result (ROPO_LY_MATCH_SYNCp, page 21).
    pub(super) ly_eq_lyc: bool,

    /// STAT interrupt enable flags (FF41 bits 3-6).
    pub(super) stat_flags: InterruptFlags,

    /// Previous STAT line state for rising-edge detection.
    pub(super) stat_line_was_high: bool,
}

impl VideoControl {
    /// Scanline dot position, computed from LX and phase.
    /// Returns 0-455 on normal lines, 0-447 on the first line after LCD-on
    /// (which starts at LX=2).
    pub fn dot(&self) -> u32 {
        self.lx as u32 * 4 + self.phase as u32
    }

    /// CPU-visible LY value. On line 153, MYTA (frame-end DFF clocked
    /// by NYPE) drives LAMA low, resetting all LY bits to 0 after
    /// the first M-cycle. The CPU sees LY=153 only during LX=0;
    /// from LX=1 onward, `ly()` returns 0. The internal counter
    /// remains at 153 until RUTU at LX=113 naturally wraps it 153→0.
    pub fn ly(&self) -> u8 {
        if self.ly == 153 && self.lx >= 1 && self.lx < 113 {
            0
        } else {
            self.ly
        }
    }

    pub fn ly_eq_lyc(&self) -> bool {
        self.ly_eq_lyc
    }

    pub fn write_ly(&mut self, value: u8) {
        self.ly = value;
    }

    pub fn latch_ly_comparison(&mut self) {
        // ROPO DFF17 latches PALY_LY_MATCHa_old on TALU rising edge.
        // Promote the previous cycle's raw comparison to the STAT-visible
        // latch, then compute the fresh comparison for next cycle.
        self.ly_eq_lyc = self.ly_match_pending;
        self.ly_match_pending = self.ly == self.lyc;
    }

    /// Advance the scanline by one dot. Returns true at scanline boundary
    /// (LX wraps 113->0 at the end of the final phase).
    pub fn advance_dot(&mut self) -> bool {
        self.phase += 1;

        if self.phase < 4 {
            return false;
        }

        // Phase wrapped 3->0: TALU rising edge, increment LX.
        self.phase = 0;
        self.lx += 1;

        // RUTU fires when LX reaches 113 (SANU comparator).
        // Clock the LY ripple counter.
        if self.lx == 113 {
            if self.ly >= 153 {
                self.ly = 0;
            } else {
                self.ly += 1;
            }
        }

        // Line end: LX reaches 114, wrap to 0.
        if self.lx >= 114 {
            self.lx = 0;
            return true;
        }

        false
    }
}
