use crate::ppu::{DffLatch, NorLatch, PipelineRegisters, VideoControl};

use super::fetch_cascade::FetchCascade;
use super::fetcher::TileFetcher;
use super::fine_scroll::FineScroll;

/// Window control: §6.12 WX-match capture pipeline modelled as explicit
/// DFFs (PYCO → NUNU → PYNU → NOPA), the RYDY/PUKU NOR-latch, the
/// WY-match frame latch (REJO), and the WAZY/VYNO window-line ripple
/// counter clocked by `wy_clk = NOT(PYNU)`.
///
/// The pipeline turns combinational `NUKO = (pix_count == WX)` into the
/// MOSU↑ pulse that drives NYXU (BG fetch counter reset). NOPA captures
/// PYNU on ALET rising; NOPA_n then gates NUNY = AND2(PYNU, NOPA_n) low
/// for the rest of mode 3, so a second activation per scanline requires
/// PYNU to fall (via XOFO clear when LCDC.5↓ or ATEJ↑) and re-rise on a
/// fresh NUKO match (typically following a CPU WX rewrite).
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
    /// PYCO `dffr`. Captures NUKO on ROCO rising — TYFA-derived clock
    /// gated by POKY=1. Naturally falls when NUKO drops (which happens
    /// when PX moves past WX, or on a CPU WX rewrite that moves the
    /// match point past PX).
    pyco: DffLatch,
    /// NUNU `dffr`. Captures PYCO on MEHE rising (= ALET falling =
    /// master-clock rising). One half-dot after PYCO captures.
    nunu: DffLatch,
    /// PYNU `nor_latch`. Set by NUNU rising edge; reset by XOFO
    /// (NAND3(LCDC.5, NOT(atej), ppu_reset_n)) going high — i.e.,
    /// LCDC.5 transitioning low, ATEJ asserting, or PPU reset.
    pynu: NorLatch,
    /// NOPA `dffr`. Captures PYNU on ALET rising (= master-clock
    /// falling). NOPA_n drives NUNY's AND2 low gate.
    nopa: DffLatch,
    /// Previous-dot NUNU.q for set-edge detection on PYNU.
    prev_nunu_q: bool,
    /// Previous-dot PYNU.q for fall-edge detection on WAZY/VYNO.
    prev_pynu_q: bool,
    /// Previous-dot NUNY for MOSU rising-edge detection.
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
    /// Window reactivation zero pixel (DMG-specific quirk; spec §6.12
    /// declares this out-of-scope, so it stays as a separate flag
    /// driven by `check_trigger_reactivation`).
    window_zero_pixel: bool,
}

impl WindowControl {
    pub(in crate::ppu) fn new() -> Self {
        WindowControl {
            rydy: NorLatch::new(false),
            pyco: DffLatch::new(0),
            nunu: DffLatch::new(0),
            pynu: NorLatch::new(false),
            nopa: DffLatch::new(0),
            prev_nunu_q: false,
            prev_pynu_q: false,
            prev_nuny: false,
            window_rendered: false,
            nuko_wx: 0xFF,
            window_line_counter: 0,
            wy_matched: false,
            window_zero_pixel: false,
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

    /// Compute combinational NUKO (PX==WX decode).
    fn compute_nuko(&self, pixel_counter: u8, regs: &PipelineRegisters) -> bool {
        regs.control.window_enabled() && self.wy_matched && pixel_counter == self.nuko_wx
    }

    /// Compute combinational XOFO. NAND3(LCDC.5, NOT(atej), ppu_reset_n);
    /// during rendering atej=0 and ppu_reset_n=1, so XOFO simplifies to
    /// NOT(LCDC.5).
    fn compute_xofo(&self, regs: &PipelineRegisters) -> bool {
        !regs.control.window_enabled()
    }

    /// Tick the §6.12 capture pipeline and evaluate NUNY/MOSU.
    /// Called once per dot at the master-clock-falling phase (the same
    /// edge `check_trigger_arming` previously fired on). Returns
    /// `true` if MOSU rises this dot — caller fires the NYXU restart
    /// side-effects (fetcher reset, fine-scroll reset, cascade reset,
    /// RYDY set).
    ///
    /// Pipeline order within one fall edge:
    /// 1. NOPA captures the prior-fall PYNU (so the pre-set PYNU value
    ///    is observed; this is what allows multi-fire — after XOFO
    ///    clears PYNU, NOPA captures the cleared value and releases
    ///    NOPA_n).
    /// 2. PYCO captures NUKO. Gated on the ROCO clock — ROCO derives
    ///    from TYFA (= AND3(POKY, SOCY, VYBO)), so any TYFA halt
    ///    freezes ROCO. We model the two halts that affect this test
    ///    matrix: POKY (data not ready) and TAKA (sprite fetch active,
    ///    which sets FEPO=1 → VYBO=0 → TYFA=0). The SOCY/RYDY halt
    ///    leaves NUKO stable so re-capturing the same value is
    ///    harmless. Naturally drops to 0 when NUKO drops.
    /// 3. NUNU captures PYCO. Half-dot offset elided in this single-
    ///    edge model; same integer-dot result.
    /// 4. Apply XOFO to PYNU's reset (clear if XOFO asserted this dot).
    /// 5. NUNU↑ → PYNU.s pulse: detect rising edge; if XOFO didn't
    ///    just clear, set PYNU.
    /// 6. NUNY = AND2(PYNU.q, NOPA_n). On NUNY rising edge, MOSU↑.
    /// 7. Detect PYNU↓ (1→0 across this dot) → wy_clk rises → WAZY
    ///    increments window_line_counter. (Per spec §6.12 resolved gap:
    ///    one increment per PYNU↓; happens at LCDC.5↓ during render or
    ///    end-of-mode-3 ATEJ↑.)
    pub(in crate::ppu) fn tick_pipeline(
        &mut self,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        pixel_counter: u8,
        poky: bool,
        taka: bool,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) -> bool {
        self.sample_wy_match(regs, video);

        let nuko = self.compute_nuko(pixel_counter, regs);
        let xofo = self.compute_xofo(regs);

        // Save prior-dot values for edge detection.
        let prev_pynu_q = self.pynu.output();
        let prev_nunu_q = self.prev_nunu_q;

        // Step 1: NOPA captures prior-fall PYNU.
        self.nopa.write(if prev_pynu_q { 1 } else { 0 });
        self.nopa.tick();

        // Step 2: PYCO captures NUKO. ROCO derives from TYFA, so any
        // TYFA halt freezes the clock and PYCO holds. Two halts matter
        // for the WX-match cascade: POKY=0 (data not ready) and TAKA=1
        // (sprite fetch active — FEPO=1 → VYBO=0 → TYFA=0). When
        // sprite_x_plus_8 ≤ WX, TAKA fires at PX==sprite_x and freezes
        // ROCO before PYCO can capture the WX match; the cascade fires
        // ~6 dots later when TAKA clears.
        if poky && !taka {
            self.pyco.write(if nuko { 1 } else { 0 });
            self.pyco.tick();
        }

        // Step 3: NUNU captures PYCO.
        self.nunu.write(self.pyco.output());
        self.nunu.tick();

        // Step 4: Apply XOFO to PYNU's reset.
        if xofo {
            self.pynu.clear();
        }

        // Step 5: NUNU↑ → PYNU.s (only if XOFO is not asserting reset).
        let nunu_rising = self.nunu.output() != 0 && prev_nunu_q == false;
        if nunu_rising && !xofo {
            self.pynu.set();
        }

        // Step 6: NUNY = AND2(PYNU, NOPA_n). MOSU rising-edge detection.
        let nuny = self.pynu.output() && self.nopa.output() == 0;
        let mosu_rising = nuny && !self.prev_nuny;

        // Step 7: PYNU↓ → wy_clk rising → WAZY increment.
        let pynu_falling = prev_pynu_q && !self.pynu.output();
        if pynu_falling && self.window_rendered {
            self.window_line_counter = self.window_line_counter.wrapping_add(1);
            self.window_rendered = false;
        }

        // Update edge-detection state for next dot.
        self.prev_nunu_q = self.nunu.output() != 0;
        self.prev_nuny = nuny;

        // Side-effects on MOSU↑ (NYXU restart cascade).
        if mosu_rising {
            fine_scroll.reset_for_window();
            self.rydy.set();
            fetcher.reset_for_window();
            cascade.reset_window();
            self.window_rendered = true;
        }

        mosu_rising
    }

    /// DMG window reactivation zero-pixel quirk. Out-of-scope for
    /// §6.12 per resolved spec gap; fetcher-stage interaction.
    pub(in crate::ppu) fn check_trigger_reactivation(
        &mut self,
        rydy_snapshot: bool,
        fetcher: &TileFetcher,
        pixel_counter: u8,
        poky: bool,
        regs: &PipelineRegisters,
    ) {
        if !regs.control.window_enabled() {
            return;
        }
        if !self.wy_matched {
            return;
        }
        if pixel_counter != regs.window.x_plus_7.output() {
            return;
        }
        if !poky {
            return;
        }

        if fetcher.fetching_window && !rydy_snapshot && fetcher.fetch_counter < 2 {
            self.window_zero_pixel = true;
        }
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
        self.window_zero_pixel = false;
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
        self.prev_nunu_q = false;
        self.prev_pynu_q = false;
        self.prev_nuny = false;
        self.nuko_wx = 0xFF;
    }

    // --- Accessors ---

    pub(in crate::ppu) fn rydy(&self) -> bool {
        self.rydy.output()
    }

    pub(in crate::ppu) fn wx_triggered(&self) -> bool {
        self.pynu.output()
    }

    pub(in crate::ppu) fn window_rendered(&self) -> bool {
        self.window_rendered
    }

    pub(in crate::ppu) fn window_line_counter(&self) -> u8 {
        self.window_line_counter
    }

    pub(in crate::ppu) fn window_zero_pixel_mut(&mut self) -> &mut bool {
        &mut self.window_zero_pixel
    }

    pub(in crate::ppu) fn consume_window_zero_pixel(&mut self) {
        self.window_zero_pixel = false;
    }
}
