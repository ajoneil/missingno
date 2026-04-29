use crate::ppu::{PipelineRegisters, VideoControl};

use super::fetch_cascade::FetchCascade;
use super::fetcher::TileFetcher;
use super::fine_scroll::FineScroll;

/// Window control: window-hit latch, WX/WY match comparators + frame
/// latch, window-armed latch, window-line counter, DMG zero-pixel
/// reactivation flag.
///
/// Per-dot `check_trigger` collapses the hardware WX-match capture
/// pipeline (NUKO → PYCO → NUNU → PYNU → NUNY → PUKU → RYDY) into a
/// single evaluation gated on PYGO; the PYCO/NUNU half-cycle pipeline
/// latency collapses to one dot via the RisingPhaseInputs snapshot.
/// Observation-equivalent at half-dot resolution.
///
/// The window-hit signal (RYDY) drives two consumer chains in
/// `rendering.rs`: it blocks sprite triggers (hardware TUKU input to
/// TEKY's AND4) and halts the pixel clock (hardware SOCY input to
/// TYFA's AND3). Hardware reaches both sites via triple-inversion of
/// RYDY through SYLO/TOMU/{TUKU,SOCY}; the emulator collapses each
/// chain to one negation at the consumer call site.
pub(in crate::ppu) struct WindowControl {
    /// Window-hit signal (hardware RYDY `nor_latch`). When high, the
    /// pixel clock halts and sprite triggers are blocked until the
    /// window's startup tile fetch completes. SET by `check_trigger`
    /// at the WX-match dot; CLEAR by PORY rising during the BG fetch
    /// cascade restart (`clear_rydy_on_pory`).
    ///
    /// 1-dot delay: `check_trigger` sets this at the end of
    /// mode3_rising, AFTER the RisingPhaseInputs snapshot. The
    /// snapshot on the NEXT dot sees the new value, giving 1-dot
    /// NUKO-to-TYFA latency that mirrors hardware's PYCO/NUNU
    /// capture cadence.
    rydy: bool,
    /// Window-armed latch (hardware PYNU `nor_latch`). Set by the
    /// WX-match pulse — hardware path NUKO → PYCO → NUNU → PYNU.s,
    /// collapsed in the emulator to per-dot `check_trigger`. Reset by
    /// `apply_xofo` when LCDC.5 (WIN_EN) goes low — hardware path
    /// XOFO = NAND3(LCDC.5, NOT(atej), ppu_reset_n) → PYNU.r.
    ///
    /// Mid-scanline WX register changes clear this flag to allow
    /// re-evaluation with a new WX value (compensates for the
    /// collapsed PYCO/NUNU pipeline that on hardware would naturally
    /// re-fire on the new NUKO match).
    wx_triggered: bool,
    /// Whether the window has rendered at least one pixel on the
    /// current line — used to gate WAZY (window-line-counter) advance
    /// at the scanline boundary.
    window_rendered: bool,
    /// Previous-dot WX register value, used to detect mid-scanline WX
    /// changes that should clear `wx_triggered` for re-evaluation.
    last_wx_value: u8,
    /// Cached WX register value for the WX comparator (hardware NUKO
    /// reads the WX register's DFF8 slave output, which lags the
    /// master by one ALET edge). Updated unconditionally at the end
    /// of every mode3_rising from the live register output;
    /// `check_trigger` reads this instead of the live register,
    /// providing the 1-dot lag on mid-scanline WX writes.
    nuko_wx: u8,
    /// Window internal line counter (hardware WAZY). Advances at the
    /// scanline boundary when the window rendered on the line.
    /// Consumed by the fetcher for the window tilemap address.
    window_line_counter: u8,
    /// WY-match frame latch (hardware REJO `nor_latch`). Set when
    /// LY==WY is first observed in the frame (sampled every TALU
    /// edge via SARY); reset only at VBlank (REPU). Once set, stays
    /// set for the remainder of the frame — a mid-scanline WY change
    /// cannot retroactively arm or disarm the window.
    wy_matched: bool,
    /// Window reactivation zero pixel (DMG-specific quirk; not in
    /// spec reference block). Set when WX re-matches while the
    /// window is already active with specific fetcher/FIFO
    /// conditions; causes the next pixel output to use bg_color=0
    /// without popping the BG shifter (OBJ shifter pops normally).
    window_zero_pixel: bool,
}

impl WindowControl {
    pub(in crate::ppu) fn new() -> Self {
        WindowControl {
            rydy: false,
            wx_triggered: false,
            window_rendered: false,
            last_wx_value: 0xFF,
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

    /// Update NUKO's WX input from the live DFF8 output. Called
    /// unconditionally at the end of every mode3_rising so the cache
    /// tracks the DFF output even during sprite fetch.
    pub(in crate::ppu) fn update_nuko_wx(&mut self, wx: u8) {
        self.nuko_wx = wx;
    }

    /// Sample the REJO NOR latch (WY==LY match). On hardware, SARY
    /// samples ROGE on every TALU edge — this runs every dot in all
    /// modes, not just mode 3. The latch is idempotent: once set, it
    /// stays set until VBlank.
    pub(in crate::ppu) fn sample_wy_match(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) {
        if !self.wy_matched && regs.control.window_enabled() && video.ly() == regs.window.y {
            self.wy_matched = true;
        }
    }

    /// Model the XOFO combinational gate. XOFO = nand3(WIN_EN,
    /// LINE_RSTn, VID_RSTn). When WIN_EN is low, XOFO goes high and
    /// resets PYNU (wx_triggered). If PYNU was high (window was
    /// active), the falling edge clocks WAZY (window line counter
    /// increments). Called every dot during mode 3.
    pub(in crate::ppu) fn apply_xofo(&mut self, window_enabled: bool) {
        if !window_enabled {
            if self.wx_triggered {
                self.window_line_counter += 1;
                self.window_rendered = false;
            }
            self.wx_triggered = false;
        }
    }

    /// PORY clears RYDY: on hardware, PORY is a reset input to the
    /// RYDY NOR latch (NOR3(PUKU, PORY, VID_RST)). When PORY goes
    /// high while RYDY is set, RYDY clears — producing the SUZU
    /// falling-edge signal (AND2(!RYDY_new, SOVY)).
    ///
    /// Returns true if RYDY transitioned 1→0 (SUZU fires), signaling
    /// the caller to load window tile data and reset the fine counter.
    pub(in crate::ppu) fn clear_rydy_on_pory(&mut self, pory: bool) -> bool {
        if pory && self.rydy {
            self.rydy = false;
            true
        } else {
            false
        }
    }

    /// MOSU↑ arming: NUKO match → PYCO → NUNU → PYNU set → MOSU pulse →
    /// NYXU async-reset of the BG fetch counter and the
    /// NYKA/PORY/PYGO/POKY cascade. Runs BEFORE the fetcher's
    /// falling-edge VRAM read so AMUV/VEVY tri-states see
    /// `fetching_window=true` on the counter=0 tile-index read.
    ///
    /// On hardware, the NUKO comparator reads pix_count DFF Q-outputs
    /// combinationally (pre-SACU value). The PYCO DFF captures the NUKO
    /// match on ROCO, which derives from TYFA and requires POKY (modeled
    /// as `pygo`). The `pixel_counter` parameter must be the pre-SACU
    /// value (from `RisingPhaseInputs`) to model this correctly.
    pub(in crate::ppu) fn check_trigger_arming(
        &mut self,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        pixel_counter: u8,
        pygo: bool,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) {
        // SARY/REJO: sample WY==LY latch. Now handled by sample_wy_match()
        // which runs every dot in all modes; call here is redundant but
        // harmless (idempotent latch).
        self.sample_wy_match(regs, video);

        if !regs.control.window_enabled() {
            return;
        }
        if !self.wy_matched {
            return;
        }

        // Detect mid-scanline WX changes to clear the trigger suppression latch.
        if self.nuko_wx != self.last_wx_value {
            self.wx_triggered = false;
            self.last_wx_value = self.nuko_wx;
        }

        if pixel_counter != regs.window.x_plus_7.output() {
            return;
        }

        // PYGO gate: PYCO is clocked by ROCO (derived from TYFA), which
        // requires POKY (pygo) to be set. Without POKY, ROCO has no edges
        // and PYCO cannot capture the NUKO match. This prevents WX=0 from
        // triggering before the initial BG fetch completes.
        if !pygo {
            return;
        }

        // Window already active — reactivation handled post-pipeline.
        if fetcher.fetching_window {
            return;
        }

        // WX already matched this line — suppress the comparator.
        if self.wx_triggered {
            return;
        }

        // Window trigger: reset fine scroll, restart fetcher, and reset
        // cascade DFFs so a new startup fetch begins. The BG/OBJ shifters
        // are NOT cleared — hardware doesn't clear them. MOSU loads stale
        // tile_temp into the BG pipe (never visible since the pixel clock
        // freezes), and SUZU/TEVO later overwrites with window tile data.
        self.wx_triggered = true;
        fine_scroll.reset_for_window();
        self.rydy = true;
        fetcher.reset_for_window();
        // NAFY: window mode trigger always resets NYKA and PORY, forcing the
        // startup cascade (NYKA→PORY→PYGO) to re-propagate after the window
        // tile fetch completes before the pixel clock can resume.
        cascade.reset_window();
        self.window_rendered = true;
    }

    /// DMG window reactivation zero-pixel quirk. Runs AFTER the pixel
    /// pipeline so it inspects post-fetch state. When WX re-matches
    /// while the window is already rendering and the fetcher is still
    /// in its first two counter steps with RYDY clear, the next pixel
    /// outputs bg_color=0 without popping the BG shifter.
    pub(in crate::ppu) fn check_trigger_reactivation(
        &mut self,
        rydy_snapshot: bool,
        fetcher: &TileFetcher,
        pixel_counter: u8,
        pygo: bool,
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
        if !pygo {
            return;
        }

        if fetcher.fetching_window && !rydy_snapshot && fetcher.fetch_counter < 2 {
            self.window_zero_pixel = true;
        }
    }

    /// Reset for a new frame. Zeroes the window line counter (WLY),
    /// which accumulates across scanlines but resets at frame start.
    pub(in crate::ppu) fn reset_frame(&mut self) {
        self.window_line_counter = 0;
        self.window_rendered = false;
        self.wy_matched = false;
    }

    /// Reset per-scanline state.
    pub(in crate::ppu) fn reset_scanline(&mut self) {
        if self.window_rendered {
            self.window_line_counter += 1;
        }
        self.rydy = false;
        self.window_rendered = false;
        self.window_zero_pixel = false;
        self.wx_triggered = false;
        self.last_wx_value = 0xFF;
        self.nuko_wx = 0xFF;
    }

    // --- Accessors ---

    pub(in crate::ppu) fn rydy(&self) -> bool {
        self.rydy
    }

    pub(in crate::ppu) fn wx_triggered(&self) -> bool {
        self.wx_triggered
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

    /// Consume the window zero pixel flag (set to false). Used during
    /// pre-visible TYFA cycles when TOBA doesn't fire.
    pub(in crate::ppu) fn consume_window_zero_pixel(&mut self) {
        self.window_zero_pixel = false;
    }
}
