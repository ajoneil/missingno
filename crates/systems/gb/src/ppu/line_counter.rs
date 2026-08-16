//! LX 7-bit ripple (TALU-clocked) cascades into LY 8-bit ripple (RUTU-clocked).
//!
//! `y.value` is the internal counter (0-153); `y.value_register()` is CPU-visible $FF44 (MYTA-smoothed).

use crate::ppu::DffBit;
use crate::ppu::dividers::Dividers;
use crate::ppu::line_end_pipeline::LineEndEdge;

/// LX value SANU decodes as scanline-end (113 = last dot before the RUTU pulse).
const SANU_DECODE_LX: u8 = 113;

pub struct LineCounter {
    pub x: LineCounterX,
    pub y: LineCounterY,
}

pub struct LineCounterX {
    pub(in crate::ppu) value: u8,
    /// RUTU DFF. D = SANU (LX==113 decode); Q captured each TALU-fall, holds LX
    /// at 0 via MUDE while high.
    pub(in crate::ppu) line_end: DffBit,
}

pub struct LineCounterY {
    pub(in crate::ppu) value: u8,
    pub(in crate::ppu) vblank: bool,
    pub(in crate::ppu) frame_end_reset: bool,
}

impl LineCounter {
    pub(in crate::ppu) fn on_lx_counter_clock_rise(&mut self, line_end_edge: LineEndEdge) {
        match line_end_edge {
            LineEndEdge::Rising => self.y.capture_vblank_on_line_end_rise(),
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
        let prior_line_end = self.x.line_end.output();
        let now_line_end = self.x.line_end.tick();

        if now_line_end {
            // MUDE async reset: LX held at 0 while RUTU=1; clear SANU for next decode.
            self.x.value = 0;
            self.x.line_end.write(false);
        }

        if now_line_end && !prior_line_end {
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
        self.x.line_end.output()
    }
    pub(in crate::ppu) fn dot_position(&self) -> u8 {
        self.x.value
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.x.vid_rst();
        self.y.vid_rst();
    }

    /// Dots until RUTU captures a set SANU. LX advances one per TALU↑, which is
    /// the fall leaving WUVU low and VENA high; SANU decodes it, and the TALU↓
    /// two dots later captures it. Prediction only — nothing here advances.
    pub(in crate::ppu) fn dots_to_line_end(&self, dividers: &Dividers) -> u32 {
        let to_lx_clock_fall = 4 - dividers.half_mcycle as u32 - 2 * dividers.mcycle as u32;
        if self.x.line_end.pending() {
            debug_assert!(
                to_lx_clock_fall <= 2,
                "SANU decoded with its capture edge more than an M-cycle away"
            );
            return to_lx_clock_fall;
        }
        // TALU's rise sits two dots off its fall, in the same 1..=4 window.
        let to_lx_clock_rise = (to_lx_clock_fall + 1) % 4 + 1;
        let advances = u32::from(SANU_DECODE_LX).saturating_sub(1 + u32::from(self.x.value));
        to_lx_clock_rise + 4 * advances + 2
    }
}

impl LineCounterX {
    /// MUDE = NOR2(RUTU, reset) holds LX at 0 for the full RUTU pulse.
    pub(in crate::ppu) fn advance(&mut self) {
        if !self.line_end.output() {
            self.value += 1;
        }
    }

    /// SANU = LX==113 decode; cached for RUTU on next falling edge.
    pub(in crate::ppu) fn detect_line_end(&mut self) {
        self.line_end.write(self.value == SANU_DECODE_LX);
    }

    /// Run the ripple forward over `rises` TALU↑ edges in one step. MUDE is
    /// released throughout — RUTU's rise is the edge a span ends on — so every
    /// rise advances, and the last one leaves SANU decoded.
    pub(in crate::ppu) fn advance_rises(&mut self, rises: u32) {
        debug_assert!(!self.line_end.output(), "MUDE held LX across a deferral");
        if rises == 0 {
            return;
        }
        self.value += rises as u8;
        self.detect_line_end();
    }

    pub(in crate::ppu) fn vid_rst(&mut self) {
        self.value = 0;
        self.line_end = DffBit::new(false, false);
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

    /// Mid-VBlank handoff on line `ly`: FF44 reads the counter directly.
    pub(in crate::ppu) fn vblank_handoff(ly: u8) -> Self {
        Self {
            value: ly,
            vblank: true,
            frame_end_reset: false,
        }
    }

    /// Returns true on 153→0 wrap. POPU drop is sequenced by the next NYPE capture,
    /// not by this wrap.
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
    pub(in crate::ppu) fn capture_vblank_on_line_end_rise(&mut self) {
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
