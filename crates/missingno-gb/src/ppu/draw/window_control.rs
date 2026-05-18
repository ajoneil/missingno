use crate::ppu::{DffLatch, NorLatch, PipelineRegisters, VideoControl};

use super::fetch_cascade::FetchCascade;
use super::fetcher::TileFetcher;
use super::fine_scroll::FineScroll;

/// WX-match capture chain (PYCO → NUNU → PYNU → NOPA), RYDY/PUKU NOR-latch, REJO WY-match,
/// and the WAZY/VYNO window-line counter clocked by `wy_clk = NOT(PYNU)`.
///
/// Each DFF captures on its hardware-correct edge:
/// - PYCO captures NUKO on PPU rise (ROCO is TYFA/SEGU-derived).
/// - NOPA captures PYNU on PPU rise.
/// - NUNU captures PYCO on PPU fall (MEHE).
/// - PYNU nor_latch: S=NUNU, R=XOFO; re-evaluated on both edges.
/// - NUNY = AND2(PYNU, NOPA_n). MOSU↑ fires on NUNY 0→1.
pub(in crate::ppu) struct WindowControl {
    /// Window-hit (RYDY nor3 + PUKU feedback). Set on NUNY rise; cleared by PORY during cascade restart.
    rydy: NorLatch,
    /// Captures NUKO on PPU rise (ROCO rising, gated by POKY=1).
    pyco: DffLatch,
    /// Captures PYCO on PPU fall (MEHE rising) — one half-dot after PYCO.
    nunu: DffLatch,
    /// Level-sensitive PYNU: sets when NUNU=1 with XOFO=0; clears when XOFO=1.
    pynu: NorLatch,
    /// Captures PYNU on PPU rise; NOPA_n drives NUNY's AND2 low gate.
    nopa: DffLatch,
    /// Previous-dot NUNY for MOSU rising-edge detection.
    prev_nuny: bool,
    /// Window has rendered at least one pixel on the current line (WAZY-equivalent flag).
    window_rendered: bool,
    /// WX register's DFF8 slave output (lags the master by one ALET edge).
    nuko_wx: u8,
    /// WAZY → VYNO ripple, clocked by PYNU 1→0 transitions during rendering.
    window_line_counter: u8,
    /// REJO WY-match frame latch.
    wy_matched: bool,
}

impl WindowControl {
    pub(in crate::ppu) fn new() -> Self {
        WindowControl {
            rydy: NorLatch::new(false),
            pyco: DffLatch::new(0),
            nunu: DffLatch::new(0),
            pynu: NorLatch::new(false),
            nopa: DffLatch::new(0),
            prev_nuny: false,
            window_rendered: false,
            nuko_wx: 0xFF,
            window_line_counter: 0,
            wy_matched: false,
        }
    }

    pub(in crate::ppu) fn init_nuko_wx(&mut self, wx: u8) {
        self.nuko_wx = wx;
    }

    pub(in crate::ppu) fn update_nuko_wx(&mut self, wx: u8) {
        self.nuko_wx = wx;
    }

    pub(in crate::ppu) fn sample_wy_match(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) {
        if !self.wy_matched && regs.control.window_enabled() && video.ly() == regs.window.y {
            self.wy_matched = true;
        }
    }

    /// Returns true on RYDY 1→0 (SUZU fires → load-window via TEVO).
    pub(in crate::ppu) fn clear_rydy_on_pory(&mut self, pory: bool) -> bool {
        if pory && self.rydy.output() {
            self.rydy.clear();
            true
        } else {
            false
        }
    }

    fn compute_nuko(&self, pixel_counter: u8) -> bool {
        self.wy_matched && pixel_counter == self.nuko_wx
    }

    /// Live NUKO. Two netlist consumers: PYCO (this chain) and PANY (drain-detector input).
    /// PANY's tile-boundary high window is where a same-dot NUKO=1 lands as the cascade slip.
    pub(in crate::ppu) fn nuko(&self, pixel_counter: u8) -> bool {
        self.compute_nuko(pixel_counter)
    }

    /// XOFO during rendering simplifies to NOT(LCDC.5).
    fn compute_xofo(&self, regs: &PipelineRegisters) -> bool {
        !regs.control.window_enabled()
    }

    /// PPU rise: NOPA captures prior-fall PYNU; PYNU re-evaluates; MOSU↑ fires if NUNY rises.
    /// Catches the deferred-completion case (LCDC.5 restore drops XOFO while NUNU=1 from prior fall).
    pub(in crate::ppu) fn tick_rising(
        &mut self,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> bool {
        self.sample_wy_match(regs, video);

        // NOPA captures BEFORE the PYNU update so it observes PYNU's prior-fall value.
        self.nopa.write(if self.pynu.output() { 1 } else { 0 });
        self.nopa.tick();

        self.update_pynu_and_check_mosu(regs, fetcher, cascade, fine_scroll)
    }

    /// PPU fall: PYCO and NUNU both capture on this edge (ROCO and MEHE are both NOT(ALET)-phase).
    /// NUNU captures the just-written PYCO value.
    pub(in crate::ppu) fn tick_falling(
        &mut self,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        pixel_counter: u8,
        poky: bool,
        taka: bool,
        regs: &PipelineRegisters,
    ) -> bool {
        let nuko = self.compute_nuko(pixel_counter);

        // PYCO holds when ROCO is halted (POKY=0 = data not ready, or TAKA=1 = sprite fetch with FEPO=1).
        if poky && !taka {
            self.pyco.write(if nuko { 1 } else { 0 });
            self.pyco.tick();
        }

        // NUNU captures the just-written PYCO.
        self.nunu.write(self.pyco.output());
        self.nunu.tick();

        self.update_pynu_and_check_mosu(regs, fetcher, cascade, fine_scroll)
    }

    /// PYNU/NUNY/MOSU update. Runs on every edge since PYNU is combinational on NUNU/XOFO.
    fn update_pynu_and_check_mosu(
        &mut self,
        regs: &PipelineRegisters,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
    ) -> bool {
        let xofo = self.compute_xofo(regs);
        let prev_pynu_q = self.pynu.output();

        if xofo {
            self.pynu.clear();
        } else if self.nunu.output() != 0 {
            self.pynu.set();
        }

        let nuny = self.pynu.output() && self.nopa.output() == 0;
        let mosu_rising = nuny && !self.prev_nuny;
        self.prev_nuny = nuny;

        // WAZY ticks on PYNU 1→0 (mid-mode-3 LCDC.5↓ or end-of-mode-3 ATEJ↑).
        if prev_pynu_q && !self.pynu.output() && self.window_rendered {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            self.window_rendered = false;
        }

        if mosu_rising {
            fine_scroll.reset_for_window();
            self.rydy.set();
            fetcher.reset_for_window();
            cascade.reset_window();
            self.window_rendered = true;
        }

        mosu_rising
    }

    pub(in crate::ppu) fn reset_frame(&mut self) {
        self.window_line_counter = 0;
        self.window_rendered = false;
        self.wy_matched = false;
    }

    /// PYCO/NUNU/PYNU/NOPA persist across scanlines on hardware. Force the WAZY increment
    /// for end-of-mode-3 ATEJ↑ when no explicit LCDC.5 toggle is fed.
    pub(in crate::ppu) fn reset_scanline(&mut self) {
        self.rydy.clear();
        if self.pynu.output() && self.window_rendered {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            self.window_rendered = false;
        }
        self.pynu.clear();
        self.pyco.write(0);
        self.pyco.tick();
        self.nunu.write(0);
        self.nunu.tick();
        self.nopa.write(0);
        self.nopa.tick();
        self.prev_nuny = false;
        self.nuko_wx = 0xFF;
    }

    pub(in crate::ppu) fn rydy(&self) -> bool {
        self.rydy.output()
    }

    pub(in crate::ppu) fn wx_triggered(&self, regs: &PipelineRegisters) -> bool {
        self.pynu.output() && !self.compute_xofo(regs)
    }

    pub(in crate::ppu) fn window_rendered(&self) -> bool {
        self.window_rendered
    }

    pub(in crate::ppu) fn window_line_counter(&self) -> u8 {
        self.window_line_counter
    }
}
