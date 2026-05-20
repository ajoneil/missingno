//! Composer of Dividers, StatInterrupt, LineCounter, and the NYPE LINE_END pipeline.

use crate::ppu::dividers::Dividers;
use crate::ppu::line_counter::LineCounter;
use crate::ppu::line_end_pipeline::{LineEndPipeline, NypeEdge};
use crate::ppu::stat_interrupt::StatInterrupt;

pub struct VideoControl {
    pub dividers: Dividers,
    pub lines: LineCounter,
    pub stat: StatInterrupt,
    /// NYPE LINE_END redistribution DFF — produces NypeEdge for POPU/MYTA dispatch.
    pub line_end: LineEndPipeline,
}

impl VideoControl {
    pub fn vid_rst(&mut self) {
        self.dividers.vid_rst();
        self.lines.vid_rst();
        self.line_end.vid_rst();
    }

    pub fn xupy(&self) -> bool {
        self.dividers.xupy()
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

    pub fn vblank_or_holdover(&self) -> bool {
        self.lines.vblank_or_holdover()
    }

    pub fn line_end_active(&self) -> bool {
        self.lines.line_end_active()
    }

    pub fn dot_position(&self) -> u8 {
        self.lines.dot_position()
    }

    pub fn write_ly(&mut self, value: u8) {
        self.lines.y.write_ly(value);
    }

    pub fn update_ly_comparison(&mut self) {
        let ly = self.lines.ly();
        self.stat.update_comparison(ly);
    }

    pub fn write_lyc(&mut self, value: u8) {
        let ly = self.lines.ly();
        self.stat.write_lyc(value, ly);
    }

    /// XOTA rising: toggle WUVU and clear vblank holdover. Returns previous WUVU.Q.
    pub fn tick_dot(&mut self) -> bool {
        let wuvu_was = self.dividers.tick_dot();
        self.lines.y.clear_vblank_holdover();
        wuvu_was
    }

    /// TALU rising: NYPE captures; LineCounter dispatches POPU (Rising) or MYTA (Falling); LX advances + SANU decodes.
    pub fn on_lx_counter_clock_rise(&mut self) {
        let nype_edge = self.line_end.capture();
        self.lines.on_lx_counter_clock_rise(nype_edge);
        if matches!(nype_edge, NypeEdge::Falling) {
            let neru = self.lines.y.value == 0;
            self.line_end.capture_meda(neru);
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
