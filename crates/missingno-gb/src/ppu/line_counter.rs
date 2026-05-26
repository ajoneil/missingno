//! LX 7-bit ripple (TALU-clocked) cascades into LY 8-bit ripple (RUTU-clocked).
//!
//! `y.value` is the internal counter (0-153); `y.value_register()` is CPU-visible $FF44 (MYTA-smoothed).

use crate::ppu::line_end_pipeline::LineEndEdge;

/// LX value SANU decodes as scanline-end (113 = last dot before the RUTU pulse).
const SANU_DECODE_LX: u8 = 113;

pub struct LineCounter {
    pub x: LineCounterX,
    pub y: LineCounterY,
}

pub struct LineCounterX {
    pub(in crate::ppu) value: u8,
    pub(in crate::ppu) line_end_detected: bool,
    pub(in crate::ppu) line_end_active: bool,
}

pub struct LineCounterY {
    pub(in crate::ppu) value: u8,
    pub(in crate::ppu) vblank: bool,
    pub(in crate::ppu) frame_end_reset: bool,
}

impl LineCounter {
    pub(in crate::ppu) fn on_lx_counter_clock_rise(&mut self, line_end_edge: LineEndEdge) {
        match line_end_edge {
            LineEndEdge::Rising => self.y.capture_vblank_on_nype_rise(),
            LineEndEdge::Falling => self.y.capture_frame_end_on_line_end_fall(),
            LineEndEdge::None => {}
        }
        self.x.advance();
        self.x.detect_line_end();
    }

    /// RUTU captures SANU each TALU-fall; pulse spans one TALU cycle.
    /// MUDE = NOR2(RUTU, reset) holds LX at 0 while RUTU=1.
    /// Returns true on the RUTU rising edge — i.e. when the scanline boundary is just reached.
    pub(in crate::ppu) fn on_lx_counter_clock_fall(&mut self) -> bool {
        let prior_rutu = self.x.line_end_active;
        let new_rutu = self.x.line_end_detected;
        self.x.line_end_active = new_rutu;

        if new_rutu {
            // MUDE async reset: LX held at 0 while RUTU=1; clear SANU for next decode.
            self.x.value = 0;
            self.x.line_end_detected = false;
        }

        if new_rutu && !prior_rutu {
            self.y.advance_or_wrap();
            true
        } else {
            false
        }
    }

    pub(in crate::ppu) fn ly(&self) -> u8 {
        self.y.value_register()
    }
    pub(in crate::ppu) fn ly_hardware(&self) -> u8 {
        self.y.value
    }
    pub(in crate::ppu) fn vblank(&self) -> bool {
        self.y.vblank
    }
    pub(in crate::ppu) fn line_end_active(&self) -> bool {
        self.x.line_end_active
    }
    pub(in crate::ppu) fn dot_position(&self) -> u8 {
        self.x.value
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.x.vid_rst();
        self.y.vid_rst();
    }
}

impl LineCounterX {
    /// MUDE = NOR2(RUTU, reset) holds LX at 0 for the full RUTU pulse.
    pub(in crate::ppu) fn advance(&mut self) {
        if !self.line_end_active {
            self.value += 1;
        }
    }

    /// SANU = LX==113 decode; cached for RUTU on next falling edge.
    pub(in crate::ppu) fn detect_line_end(&mut self) {
        self.line_end_detected = self.value == SANU_DECODE_LX;
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.value = 0;
        self.line_end_detected = false;
        self.line_end_active = false;
    }
}

impl LineCounterY {
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            value: 153,
            vblank: true,
            frame_end_reset: true,
        }
    }

    /// Returns true if a 153→0 wrap occurred. POPU drops on the next TALU↑ via
    /// `capture_vblank_on_nype_rise` once LY has wrapped to 0; the wrap itself does
    /// not touch POPU (spec §8.5.1 — POPU.q↓ at TALU↑ + 1,536 ps, on the same TALU
    /// edge that NYPE.q↑).
    pub(in crate::ppu) fn advance_or_wrap(&mut self) -> bool {
        if self.value >= 153 {
            self.value = 0;
            self.frame_end_reset = false;
            true
        } else {
            self.value += 1;
            false
        }
    }

    /// POPU VBlank capture on NYPE rising.
    pub(in crate::ppu) fn capture_vblank_on_nype_rise(&mut self) {
        self.vblank = self.value >= 144;
    }

    /// MYTA FRAME_END capture on NYPE falling — one TALU after POPU. Sets `frame_end_reset` for LY=0 smoothing.
    pub(in crate::ppu) fn capture_frame_end_on_line_end_fall(&mut self) {
        if self.value == 153 {
            self.frame_end_reset = true;
        }
    }

    /// $FF44 read. MYTA drives LAMA low on line 153, so register reads as 0 while internal counter is still 153.
    pub(in crate::ppu) fn value_register(&self) -> u8 {
        if self.frame_end_reset { 0 } else { self.value }
    }

    pub(in crate::ppu) fn write_ly(&mut self, value: u8) {
        self.value = value;
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.value = 0;
        self.vblank = false;
        self.frame_end_reset = false;
    }
}
