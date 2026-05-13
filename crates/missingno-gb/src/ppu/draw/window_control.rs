use crate::ppu::{DffLatch, NorLatch, PipelineRegisters, VideoControl};

use super::fetch_cascade::FetchCascade;
use super::fetcher::TileFetcher;
use super::fine_scroll::FineScroll;

/// Window control: §6.12 WX-match capture pipeline modelled as explicit
/// DFFs (PYCO → NUNU → PYNU → NOPA), the RYDY/PUKU NOR-latch, the
/// WY-match frame latch (REJO), and the WAZY/VYNO window-line ripple
/// counter clocked by `wy_clk = NOT(PYNU)`.
///
/// # Edge discipline (mirrors `FetchCascade`)
///
/// Each DFF captures on its hardware-correct PPU clock edge:
///
/// - **PYCO** (dffr): clocked by ROCO (SEGU-derived, pixel-clock-derived).
///   Captures NUKO on our PPU clock rise (= ALET rising — closest available
///   edge to spec's "+0.5 dots after NUKO↑").
/// - **NOPA** (dffr): clocked by ALET rising — our PPU clock **rise**.
///   Captures PYNU's prior-fall value.
/// - **NUNU** (dffr): clocked by MEHE rising = ALET falling — our PPU
///   clock **fall**. Captures PYCO from this dot's earlier rise.
/// - **PYNU** (nor_latch): level-sensitive S=NUNU, R=XOFO (§6.12 l.2033).
///   Re-evaluated on both edges since NUNU updates on fall and XOFO
///   (combinational from LCDC.5) can change on rise via the bus-write
///   site.
/// - **NUNY** (and2): combinational PYNU & NOPA_n. MOSU↑ side-effects
///   fire on whichever edge NUNY transitions 0→1.
///
/// The pipeline turns combinational `NUKO = (pix_count == WX)` into the
/// MOSU↑ pulse that drives NYXU (BG fetch counter reset).
///
/// RYDY consumers (sprite-trigger block via TUKU; CLKPIPE halt via SOCY;
/// SUZU window-restart via SOVY) read `rydy()`. The collapsed
/// triple-inversion through SYLO/TOMU is folded into one negation at
/// each consumer call site in `rendering.rs`.
pub(in crate::ppu) struct WindowControl {
    /// Window-hit signal (hardware RYDY `nor3` with PUKU feedback).
    /// Set when NUNY rises (PUKU drops, RYDY rises). Cleared by PORY
    /// rising during the BG fetch cascade restart.
    rydy: NorLatch,
    /// PYCO `dffr`. Captures NUKO on ROCO rising — TYFA-derived
    /// pixel-clock edge gated by POKY=1. Modelled as captured on our
    /// PPU clock rise (closest available edge to spec's "+0.5 dots
    /// after NUKO↑").
    pyco: DffLatch,
    /// NUNU `dffr`. Captures PYCO on MEHE rising (= ALET falling =
    /// master-clock falling = our PPU clock **fall**). One half-dot
    /// after PYCO captures.
    nunu: DffLatch,
    /// PYNU `nor_latch`. Level-sensitive: S=NUNU, R=XOFO (§6.12 l.2033).
    /// Q sets when NUNU=1 with XOFO=0; clears when XOFO=1; holds
    /// otherwise. Re-evaluated on both edges (NUNU updates on fall;
    /// XOFO can change on rise via LCDC.5 bus writes).
    pynu: NorLatch,
    /// NOPA `dffr`. Captures PYNU on ALET rising = our PPU clock
    /// **rise**. NOPA_n drives NUNY's AND2 low gate.
    nopa: DffLatch,
    /// Previous-dot NUNY for MOSU rising-edge detection (carried across
    /// both rise and fall ticks).
    prev_nuny: bool,
    /// Whether the window has rendered at least one pixel on the
    /// current line — drives the WAZY-equivalent end-of-mode-3 flag.
    window_rendered: bool,
    /// Cached WX register value for the WX comparator (hardware NUKO
    /// reads the WX register's DFF8 slave output, which lags the
    /// master by one ALET edge).
    nuko_wx: u8,
    /// Window internal line counter (hardware WAZY → VYNO ripple).
    /// Clocked by `wy_clk = NOT(PYNU)` rising — i.e., increments on
    /// every PYNU 1→0 transition during rendering.
    window_line_counter: u8,
    /// WY-match frame latch (hardware REJO `nor_latch`).
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

    /// Initialize the NUKO WX cache on Mode 3 entry.
    pub(in crate::ppu) fn init_nuko_wx(&mut self, wx: u8) {
        self.nuko_wx = wx;
    }

    /// Update NUKO's WX input from the live DFF8 output.
    pub(in crate::ppu) fn update_nuko_wx(&mut self, wx: u8) {
        self.nuko_wx = wx;
    }

    /// Sample the REJO NOR latch (WY==LY match). Idempotent.
    pub(in crate::ppu) fn sample_wy_match(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) {
        if !self.wy_matched && regs.control.window_enabled() && video.ly() == regs.window.y {
            self.wy_matched = true;
        }
    }

    /// PORY clears RYDY: NOR3 reset path. Returns true if RYDY
    /// transitioned 1→0 (SUZU fires, signaling SUZU/TEVO load-window).
    pub(in crate::ppu) fn clear_rydy_on_pory(&mut self, pory: bool) -> bool {
        if pory && self.rydy.output() {
            self.rydy.clear();
            true
        } else {
            false
        }
    }

    /// Compute combinational NUKO (PX==WX decode). NOGY is a NAND5 of PX/WX
    /// bits only; LCDC.5 gates the chain downstream via XOFO → PYNU.r.
    fn compute_nuko(&self, pixel_counter: u8, _regs: &PipelineRegisters) -> bool {
        self.wy_matched && pixel_counter == self.nuko_wx
    }

    /// Live NUKO read for consumers outside the §6.12 capture chain.
    /// Hardware NUKO has two netlist consumers: PYCO (this module's
    /// capture chain, gated on NOPA_n once the window is active) and
    /// PANY (the §6.1 drain-detector input, `pany = NOR2(roze, wxy_match)`).
    /// The PANY consumer is what produces the WX-rewrite cascade slip
    /// when NUKO=1 lands inside PANY's tile-boundary high window.
    pub(in crate::ppu) fn nuko(&self, pixel_counter: u8, regs: &PipelineRegisters) -> bool {
        self.compute_nuko(pixel_counter, regs)
    }

    /// Compute combinational XOFO. NAND3(LCDC.5, NOT(atej), ppu_reset_n);
    /// during rendering atej=0 and ppu_reset_n=1, so XOFO simplifies to
    /// NOT(LCDC.5).
    fn compute_xofo(&self, regs: &PipelineRegisters) -> bool {
        !regs.control.window_enabled()
    }

    /// PPU clock rise tick. Captures NOPA (ALET-rising-clocked) from
    /// prior-fall PYNU, re-evaluates the level-sensitive PYNU nor_latch,
    /// and fires MOSU↑ side-effects if NUNY rises on this edge —
    /// catches the deferred-completion case where the LCDC.5 restore
    /// CUPA drops XOFO while NUNU=1 is held from the prior fall.
    pub(in crate::ppu) fn tick_rising(
        &mut self,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> bool {
        self.sample_wy_match(regs, video);

        // NOPA captures PYNU on ALET rising (§6.12 l.1919). Captured
        // BEFORE the level-sensitive PYNU update in
        // update_pynu_and_check_mosu, so NOPA observes PYNU's pre-rise
        // value — i.e., the value held since the prior fall's update.
        self.nopa.write(if self.pynu.output() { 1 } else { 0 });
        self.nopa.tick();

        self.update_pynu_and_check_mosu(regs, fetcher, cascade, fine_scroll)
    }

    /// PPU clock fall tick. Captures PYCO (ROCO ≈ SEGU-derived,
    /// rises on MYVO rising = ALET falling = our PPU fall) and NUNU
    /// (MEHE rising = ALET falling = our PPU fall), then re-evaluates
    /// the level-sensitive PYNU nor_latch and checks for MOSU↑.
    ///
    /// PYCO and NUNU share the PPU-fall edge per spec (ROCO and MEHE
    /// are both NOT(ALET)-phase). NUNU captures the just-written PYCO
    /// value (in-fall ordering).
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
        let nuko = self.compute_nuko(pixel_counter, regs);

        // PYCO captures NUKO on ROCO rising. ROCO derives from TYFA, so
        // any TYFA halt freezes the clock and PYCO holds. Two halts
        // matter: POKY=0 (data not ready) and TAKA=1 (sprite fetch
        // active — FEPO=1 → VYBO=0 → TYFA=0). When sprite_x_plus_8 ≤ WX,
        // TAKA fires at PX==sprite_x and freezes ROCO before PYCO can
        // capture the WX match; the cascade fires ~6 dots later when
        // TAKA clears. Naturally drops to 0 when NUKO drops.
        if poky && !taka {
            self.pyco.write(if nuko { 1 } else { 0 });
            self.pyco.tick();
        }

        // NUNU captures PYCO on MEHE rising = ALET falling = our PPU
        // clock fall (§6.12 l.1898, l.1913). Captures the just-written
        // PYCO value.
        self.nunu.write(self.pyco.output());
        self.nunu.tick();

        self.update_pynu_and_check_mosu(regs, fetcher, cascade, fine_scroll)
    }

    /// Shared edge update: level-sensitive PYNU nor_latch, NUNY/MOSU
    /// rising-edge detection, PYNU↓ → WAZY increment, MOSU↑ side-effects.
    /// Runs on every edge (after rise or fall stages have captured their
    /// DFFs) since PYNU is combinational on NUNU/XOFO and both can
    /// transition on either PPU edge.
    fn update_pynu_and_check_mosu(
        &mut self,
        regs: &PipelineRegisters,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
    ) -> bool {
        let xofo = self.compute_xofo(regs);
        let prev_pynu_q = self.pynu.output();

        // PYNU nor_latch (§6.12 l.2033): Q sets when NUNU=1 with XOFO=0;
        // clears when XOFO=1; holds otherwise.
        if xofo {
            self.pynu.clear();
        } else if self.nunu.output() != 0 {
            self.pynu.set();
        }

        // NUNY = AND2(PYNU, NOPA_n). MOSU rising-edge detection.
        let nuny = self.pynu.output() && self.nopa.output() == 0;
        let mosu_rising = nuny && !self.prev_nuny;
        self.prev_nuny = nuny;

        // PYNU↓ → wy_clk rises → WAZY increment (one increment per PYNU↓
        // during rendering; happens at LCDC.5↓ mid-mode-3 or at
        // end-of-mode-3 ATEJ↑).
        if prev_pynu_q && !self.pynu.output() && self.window_rendered {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            self.window_rendered = false;
        }

        // MOSU↑ side-effects (NYXU restart cascade).
        if mosu_rising {
            fine_scroll.reset_for_window();
            self.rydy.set();
            fetcher.reset_for_window();
            cascade.reset_window();
            self.window_rendered = true;
        }

        mosu_rising
    }

    /// Reset for a new frame.
    pub(in crate::ppu) fn reset_frame(&mut self) {
        self.window_line_counter = 0;
        self.window_rendered = false;
        self.wy_matched = false;
    }

    /// Reset per-scanline state. PYCO/NUNU/PYNU/NOPA persist across
    /// scanline boundaries on hardware (mode-3-bound clocking but no
    /// per-scanline reset path). The end-of-mode-3 ATEJ↑ asserts XOFO
    /// which clears PYNU and triggers the wy_clk rising edge that
    /// increments WAZY — that increment now happens via the PYNU↓
    /// edge-detect in tick_pipeline, so reset_scanline no longer
    /// directly bumps the counter.
    pub(in crate::ppu) fn reset_scanline(&mut self) {
        self.rydy.clear();
        // Force PYNU↓ if it was high at scanline end (models ATEJ↑
        // asserting XOFO at end-of-mode-3 for tests that don't have
        // an explicit LCDC.5 toggle). The increment fires via the
        // edge-detect on the next tick if window was rendered.
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

    // --- Accessors ---

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
