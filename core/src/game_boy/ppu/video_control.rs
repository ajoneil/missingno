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

    /// LY register (MUWY-LAFO, page 21). Written by the pixel pipeline
    /// at the RUTU line-end event (dot 452). Read by CPU at FF44.
    pub(super) ly: u8,

    /// LYC register (FF45). CPU-writable comparison value.
    pub(super) lyc: u8,

    /// Latched LY==LYC comparison result (ROPO_LY_MATCH_SYNCp, page 21).
    /// Updated each M-cycle by `latch_ly_comparison()`. The STAT register
    /// read and STAT interrupt LYC condition both use this latched value.
    /// Frozen when the PPU is off (comparison clock stops).
    pub(super) ly_eq_lyc: bool,

    /// STAT interrupt enable flags (FF41 bits 3-6).
    pub(super) stat_flags: InterruptFlags,

    /// Previous STAT line state for rising-edge detection.
    pub(super) stat_line_was_high: bool,

    /// First scanline after LCD enable. The video clock divider
    /// (WUVU/VENA) starts in a misaligned phase, shortening this line
    /// to 448 dots (vs normal 456). Set on LCD enable, cleared when
    /// `advance_dot()` wraps the first line.
    pub(super) lcd_on_first_line: bool,
}

impl VideoControl {
    pub fn dot(&self) -> u32 {
        self.dot
    }

    pub fn ly(&self) -> u8 {
        self.ly
    }

    pub fn ly_eq_lyc(&self) -> bool {
        self.ly_eq_lyc
    }

    pub fn write_ly(&mut self, value: u8) {
        self.ly = value;
    }

    pub fn latch_ly_comparison(&mut self) {
        self.ly_eq_lyc = self.ly == self.lyc;
    }

    /// Advance the scanline dot counter by one. At RUTU_LINE_END_DOT (452),
    /// fires the RUTU event: LY increments (or wraps 153→0). At
    /// SCANLINE_TOTAL_DOTS (456), resets dot to 0 and returns true.
    ///
    /// On the first line after LCD enable, the video clock divider starts
    /// misaligned, shortening the line to 448 dots (RUTU at 444, wrap at
    /// 448). The `lcd_on_first_line` flag is self-clearing: consumed when
    /// the shortened line wraps.
    pub fn advance_dot(&mut self) -> bool {
        self.dot += 1;

        let (rutu_dot, wrap_dot) = if self.lcd_on_first_line {
            (
                pixel_pipeline::RUTU_LINE_END_DOT - 8,
                pixel_pipeline::SCANLINE_TOTAL_DOTS - 8,
            )
        } else {
            (
                pixel_pipeline::RUTU_LINE_END_DOT,
                pixel_pipeline::SCANLINE_TOTAL_DOTS,
            )
        };

        if self.dot == rutu_dot {
            // RUTU line-end event: clock the LY ripple counter.
            if self.ly == 153 {
                self.ly = 0;
            } else {
                self.ly += 1;
            }
        }

        if self.dot == wrap_dot {
            self.dot = 0;
            self.lcd_on_first_line = false;
            return true;
        }

        false
    }
}
