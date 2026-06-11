use crate::ppu::{DffLatch, NorLatch, PipelineRegisters, PpuModel, VideoControl};

use super::fetch_cascade::FetchCascade;
use super::fetcher::TileFetcher;
use super::fine_scroll::FineScroll;

/// WY-match SARY/REJO/REPU, WX-match capture chain (PYCO → NUNU → PYNU → NOPA), RYDY/PUKU
/// NOR-latch, and the WAZY/VYNO window-line counter clocked by `wy_clk = NOT(PYNU)`.
///
/// Each DFF captures on its hardware-correct edge:
/// - SARY captures `wy_match` on master rise (hclk rising).
/// - PYCO captures NUKO on PPU rise (ROCO is TYFA/SEGU-derived).
/// - NOPA captures PYNU on PPU rise.
/// - NUNU captures PYCO on PPU fall (MEHE).
/// - PYNU nor_latch: S=NUNU, R=XOFO; re-evaluated on both edges.
/// - REJO nor_latch: S=SARY.q, R=REPU (vblank); re-evaluated on both edges.
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
    /// SARY: hclk-clocked DFF sampling `wy_match = LCDC.5 ∧ (LY == WY)`.
    sary: DffLatch,
    /// REJO WY-match frame latch. Set by SARY.q; reset by REPU = vblank (mode1).
    rejo: NorLatch,
    /// REJO.q as PYCO's ROCO edge sees it: sampled before this fall's hclk/SARY→REJO update,
    /// since ROCO precedes the late hclk edge within the PX==WX pixel.
    rejo_at_roco: bool,
    /// CGB WY/LCDC.5 as the WY-match decode sees them: register cells cross
    /// into the PPU domain at the CPU M-boundary, the same crossing as the
    /// STAT register file. Unused on DMG (the decode reads the cells live).
    synced_wy: u8,
    synced_window_enabled: bool,
    /// REPU (vblank) holds the crossing transparent: a capture during vblank
    /// — and the first one after it — reads the cells live.
    vblank_at_last_capture: bool,
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
            sary: DffLatch::new(0),
            rejo: NorLatch::new(false),
            rejo_at_roco: false,
            synced_wy: 0,
            synced_window_enabled: false,
            vblank_at_last_capture: true,
        }
    }

    pub(in crate::ppu) fn capture_register_sync(&mut self, wy: u8, enabled: bool) {
        self.synced_wy = wy;
        self.synced_window_enabled = enabled;
    }

    pub(in crate::ppu) fn init_nuko_wx(&mut self, wx: u8) {
        self.nuko_wx = wx;
    }

    pub(in crate::ppu) fn update_nuko_wx(&mut self, wx: u8) {
        self.nuko_wx = wx;
    }

    fn capture_sary(&mut self, regs: &PipelineRegisters, video: &VideoControl, synced: bool) {
        let (wy, enabled) = if synced && !self.vblank_at_last_capture {
            (self.synced_wy, self.synced_window_enabled)
        } else {
            (regs.window.y, regs.control.window_enabled())
        };
        let wy_match = enabled && video.ly() == wy;
        self.vblank_at_last_capture = video.vblank();
        self.sary.write(if wy_match { 1 } else { 0 });
        self.sary.tick();
    }

    fn update_rejo(&mut self, video: &VideoControl) {
        if video.vblank() {
            self.rejo.clear();
        } else if self.sary.output() != 0 {
            self.rejo.set();
        }
    }

    /// REJO re-evaluates against current SARY + vblank on every PPU rise (handles vblank↑).
    /// SARY itself only captures on TALU↑ — see `tick_wy_match_falling`.
    pub(in crate::ppu) fn update_rejo_on_rise(&mut self, video: &VideoControl) {
        self.update_rejo(video);
    }

    /// TALU↑ (hclk rising) lands on a PPU fall in the emulator's clock model. SARY captures
    /// wy_match on that edge; REJO re-evaluates on every fall to handle vblank↓.
    pub(in crate::ppu) fn tick_wy_match_falling(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        talu_rising: bool,
        register_sync: bool,
    ) {
        self.rejo_at_roco = self.rejo.output();
        if talu_rising {
            self.capture_sary(regs, video, register_sync);
        }
        self.update_rejo(video);
    }

    /// Releases the window-hit latch (RYDY) when the fetcher-idle DFF (PORY) signals the
    /// cascade restart. Returns true on the RYDY 1→0 edge — SUZU fires, which triggers TEVO's
    /// load-window pulse.
    pub(in crate::ppu) fn release_window_hit_on_fetcher_reset(
        &mut self,
        fetcher_reset: bool,
    ) -> bool {
        if fetcher_reset && self.rydy.output() {
            self.rydy.clear();
            true
        } else {
            false
        }
    }

    fn compute_nuko(&self, pixel_counter: u8) -> bool {
        self.rejo_at_roco && pixel_counter == self.nuko_wx
    }

    fn nuny(&self) -> bool {
        self.pynu.output() && self.nopa.output() == 0
    }

    /// Live NUKO (pixel_counter == WX). Two netlist consumers: PYCO (this chain) and PANY
    /// (drain-detector input). PANY's tile-boundary high window is where a same-dot hit lands
    /// as the cascade slip.
    pub(in crate::ppu) fn window_x_reached(&self, pixel_counter: u8) -> bool {
        self.compute_nuko(pixel_counter)
    }

    /// XOFO during rendering simplifies to NOT(LCDC.5).
    fn compute_xofo(&self, regs: &PipelineRegisters) -> bool {
        !regs.control.window_enabled()
    }

    /// PPU rise: NOPA captures prior-fall PYNU; PYNU re-evaluates; MOSU↑ fires if NUNY rises.
    /// Catches the deferred-completion case (LCDC.5 restore drops XOFO while NUNU=1 from prior fall).
    pub(in crate::ppu) fn tick_rising<P: PpuModel>(
        &mut self,
        fetcher: &mut TileFetcher<P>,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        regs: &PipelineRegisters,
    ) -> bool {
        // NOPA captures BEFORE the PYNU update so it observes PYNU's prior-fall value.
        self.nopa.write(if self.pynu.output() { 1 } else { 0 });
        self.nopa.tick();

        self.update_pynu_and_check_mosu(regs, fetcher, cascade, fine_scroll)
    }

    /// PPU fall: PYCO and NUNU both capture on this edge (ROCO and MEHE are both NOT(ALET)-phase).
    /// NUNU captures the just-written PYCO value.
    pub(in crate::ppu) fn tick_falling<P: PpuModel>(
        &mut self,
        fetcher: &mut TileFetcher<P>,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        pixel_counter: u8,
        fetcher_ready: bool,
        fepo: bool,
        regs: &PipelineRegisters,
    ) -> bool {
        let nuko = self.compute_nuko(pixel_counter);

        // PYCO holds when FEPO=1: VYBO=NOR3(MYVO,FEPO,WODU)=0 → TYFA=0 halts ROCO, one ALET before TAKA latches.
        if fetcher_ready && !fepo {
            self.pyco.write(if nuko { 1 } else { 0 });
            self.pyco.tick();
        }

        // NUNU captures the just-written PYCO.
        self.nunu.write(self.pyco.output());
        self.nunu.tick();

        self.update_pynu_and_check_mosu(regs, fetcher, cascade, fine_scroll)
    }

    /// PYNU/NUNY/MOSU update. Runs on every edge since PYNU is combinational on NUNU/XOFO.
    fn update_pynu_and_check_mosu<P: PpuModel>(
        &mut self,
        regs: &PipelineRegisters,
        fetcher: &mut TileFetcher<P>,
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

        let nuny = self.nuny();
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
    }

    /// Models ATEJ↑'s XOFO pulse on PYNU: clear briefly, re-set from NUNU carryover, NOPA captures.
    pub(in crate::ppu) fn reset_scanline(&mut self) {
        self.rydy.clear();
        if self.pynu.output() && self.window_rendered {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            self.window_rendered = false;
        }
        self.pynu.clear();
        if self.nunu.output() != 0 {
            self.pynu.set();
        }
        self.nopa.write(if self.pynu.output() { 1 } else { 0 });
        self.nopa.tick();
        self.prev_nuny = self.nuny();
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
