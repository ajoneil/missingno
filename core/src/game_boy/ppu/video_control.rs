use bitflags::bitflags;

use super::pixel_pipeline;

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
    /// Scanline dot counter (XODO-XYNY flip-flop chain, page 21).
    /// Counts 0–455 every scanline, in both active display and VBlank.
    /// Drives RUTU (line-end event that clocks LY) at dot 452.
    pub(super) dot: u32,

    /// LY counter (MUWY-LAFO ripple counter, page 21). Clocked by RUTU
    /// at dot 452, counting 0–153 and wrapping. On line 153, MYTA
    /// (frame-end DFF, clocked by NYPE one half-cycle after RUTU) drives
    /// LAMA low, resetting all LY bits to 0. The CPU sees LY=153 only
    /// during the first M-cycle (dots 0–3); from dot 4 onward, `ly()`
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
    pub fn dot(&self) -> u32 {
        self.dot
    }

    /// CPU-visible LY value. On line 153, MYTA (frame-end DFF clocked
    /// by NYPE) drives LAMA low, resetting all LY bits to 0 after
    /// dot 4. The CPU sees LY=153 only during the first M-cycle
    /// (dots 0–3); from dot 4 onward, `ly()` returns 0. The internal
    /// counter remains at 153 until RUTU at dot 452 naturally wraps
    /// it 153→0.
    pub fn ly(&self) -> u8 {
        if self.ly == 153 && self.dot >= 4 && self.dot < pixel_pipeline::RUTU_LINE_END_DOT {
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
        self.ly_match_pending = self.ly() == self.lyc;
    }

    /// Advance the scanline dot counter by one. At RUTU_LINE_END_DOT (452),
    /// fires the RUTU event: the LY ripple counter increments, wrapping
    /// naturally from 153→0. At SCANLINE_TOTAL_DOTS (456), resets dot to
    /// 0 and returns true.
    pub fn advance_dot(&mut self) -> bool {
        self.dot += 1;

        if self.dot == pixel_pipeline::RUTU_LINE_END_DOT {
            // RUTU line-end event: clock the LY ripple counter.
            if self.ly >= 153 {
                self.ly = 0;
            } else {
                self.ly += 1;
            }
        }

        if self.dot == pixel_pipeline::SCANLINE_TOTAL_DOTS {
            self.dot = 0;
            return true;
        }

        false
    }
}
