//! Composer of Dividers, StatInterrupt, LineCounter, and the NYPE LINE_END pipeline.

use crate::ppu::dividers::Dividers;
use crate::ppu::line_counter::LineCounter;
use crate::ppu::line_end_pipeline::{LineEndEdge, LineEndPipeline};
use crate::ppu::stat_interrupt::{StatInterrupt, StatShadow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoControl {
    pub dividers: Dividers,
    pub lines: LineCounter,
    pub stat: StatInterrupt,
    /// NYPE LINE_END redistribution DFF — produces LineEndEdge for POPU/MYTA dispatch.
    pub line_end: LineEndPipeline,
}

/// What one dot fall's divider chain moved — the edges the PPU hangs the rest
/// of its fall on.
pub(in crate::ppu) struct DotAdvance {
    /// XUPY 0→1: the OAM scan chain's clock.
    pub scan_clock_rising: bool,
    /// TALU↑ ran, so ROPO relatched and LX advanced.
    pub talu_rising: bool,
    /// RUTU rose on this fall's TALU↓.
    pub scanline_boundary: bool,
    /// POPU 0→1 — the VBlank IF source.
    pub vblank_rose: bool,
}

impl VideoControl {
    pub fn vid_rst(&mut self) {
        self.dividers.vid_rst();
        self.lines.vid_rst();
        self.line_end.vid_rst();
    }

    pub fn scan_clock(&self) -> bool {
        self.dividers.scan_clock()
    }

    /// CPU-visible LY ($FF44). On line 153, MYTA drives LAMA low so register reads as 0.
    pub fn ly(&self) -> u8 {
        self.lines.ly()
    }

    /// Hardware-internal LY (0-153); bypasses MYTA smoothing.
    pub fn ly_hardware(&self) -> u8 {
        self.lines.ly_hardware()
    }

    pub fn vblank(&self) -> bool {
        self.lines.vblank()
    }

    pub fn line_end_active(&self) -> bool {
        self.lines.line_end_active()
    }

    pub(in crate::ppu) fn line_end_settled(&self) -> bool {
        self.line_end.settled()
    }

    /// Dots until the next line-end pulse — the edge no span survives.
    pub(in crate::ppu) fn dots_to_line_end(&self) -> u32 {
        self.lines.dots_to_line_end(&self.dividers)
    }

    pub fn dot_position(&self) -> u8 {
        self.lines.dot_position()
    }

    pub fn write_ly(&mut self, value: u8) {
        self.lines.y.write_ly(value);
    }

    pub fn update_ly_comparison(&mut self, shadow: &impl StatShadow) {
        let ly = self.lines.ly();
        self.stat.update_comparison(ly, shadow);
    }

    pub fn write_lyc(&mut self, value: u8, shadow: &mut impl StatShadow) {
        let ly = self.lines.ly();
        self.stat.write_lyc(value, ly, shadow);
    }

    /// XOTA rising: toggle WUVU. Returns previous WUVU.Q.
    pub fn tick_dot(&mut self) -> bool {
        self.dividers.tick_dot()
    }

    /// One dot fall of the divider chain and everything it clocks: WUVU
    /// toggles, VENA captures on the toggle that leaves WUVU low, and VENA's
    /// own edges drive NYPE, the LX/LY ripples and the LYC comparator.
    pub(in crate::ppu) fn advance_dot(&mut self, shadow: &impl StatShadow) -> DotAdvance {
        // XUPY = WUVU.Q; tick_dot returns previous WUVU.Q so scan_clock_rising = !was.
        let mut advance = DotAdvance {
            scan_clock_rising: !self.tick_dot(),
            talu_rising: false,
            scanline_boundary: false,
            vblank_rose: false,
        };
        if !self.dividers.half_mcycle_fell() {
            return advance;
        }

        let vena_was = self.dividers.tick_mcycle();
        let vena_now = self.dividers.mcycle();
        let popu_was = self.vblank();

        advance.talu_rising = !vena_was && vena_now;
        if advance.talu_rising {
            // VENA↑ = TALU↑: ROPO captures PALY; NYPE captures POPU/MYTA; LX advances.
            self.update_ly_comparison(shadow);
            self.stat.latch_comparison();
            self.on_lx_counter_clock_rise();
            self.update_ly_comparison(shadow);
        }
        if vena_was && !vena_now {
            // VENA↓ = SONO↑ = TALU↓: RUTU captures SANU; LY advances.
            advance.scanline_boundary = self.on_lx_counter_clock_fall();
            self.update_ly_comparison(shadow);
        }

        // POPU↑ → VYPU → LOPE: VBlank IF.
        advance.vblank_rose = self.vblank() && !popu_was;
        advance
    }

    /// Run the chain forward over the `dots` a span deferred. RUTU is low
    /// throughout a span and every capture the chain would run rewrites what is
    /// already there, so only WUVU, VENA, LX and SANU move — by count.
    pub(in crate::ppu) fn materialize_dots(&mut self, dots: u32) {
        let rises = self.dividers.advance_dots(dots);
        self.lines.x.advance_rises(rises);
    }

    /// TALU rising: NYPE captures; LineCounter dispatches POPU (Rising) or MYTA (Falling); LX advances + SANU decodes.
    pub fn on_lx_counter_clock_rise(&mut self) {
        let line_end_edge = self.line_end.capture();
        self.lines.on_lx_counter_clock_rise(line_end_edge);
        if matches!(line_end_edge, LineEndEdge::Falling) {
            let neru = self.lines.y.value == 0;
            self.line_end.capture_vsync(neru);
        }
    }

    /// TALU falling: RUTU fires (scanline boundary + LY advance); on boundary, signal NYPE feed.
    pub fn on_lx_counter_clock_fall(&mut self) -> bool {
        let scanline_boundary = self.lines.on_lx_counter_clock_fall();
        if scanline_boundary {
            self.line_end.signal_line_end();
        }
        scanline_boundary
    }
}
