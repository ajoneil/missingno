use crate::ppu::{PipelineRegisters, VideoControl};

use super::fetch_cascade::FetchCascade;
use super::fetcher::TileFetcher;
use super::fine_scroll::FineScroll;

/// Window control block (die page 27).
///
/// Owns the RYDY NOR latch (window hit signal), WX comparator state
/// (NUKO/PYCO), window line counter, and window zero pixel flag.
///
/// On the die, this block also contains the fine scroll counter and
/// tile fetch state machine (modeled separately as `FineScroll` and
/// `TileFetcher`). The window control signals gate TYFA (pixel clock)
/// via SOCY = NOT(RYDY).
///
/// Inputs: pixel counter (from page 24), PYGO (from cascade), PORY
/// (cascade clear signal), register values (WX, WY, LCDC).
/// Outputs: RYDY (gates TYFA), window_zero_pixel (to pixel mux),
/// window_line_counter (to fetcher for tile address).
pub(in crate::ppu) struct WindowControl {
    /// RYDY NOR latch — window hit signal. When high, gates TYFA
    /// (via SOCY_WIN_HITn = not1(TOMU_WIN_HITp)), freezing both the
    /// fine counter (PECU via ROXO) and pixel counter (SACU via SEGU)
    /// during a window fetch stall. SET by check_trigger (PYCO match),
    /// CLEAR by PORY (NYKA/PORY cascade after fetcher completes).
    ///
    /// 1-dot delay: check_trigger sets rydy at the end of mode3_rising,
    /// AFTER the RisingPhaseInputs snapshot. The snapshot on the NEXT
    /// dot sees rydy=true, giving 1-dot NUKO-to-TYFA latency.
    rydy: bool,
    /// WX comparator suppression latch. Models the hardware behavior
    /// where the RYDY latch prevents the WX comparator (PYCO) from
    /// re-firing after the window has already triggered on this
    /// scanline. Cleared when WX is written mid-scanline, allowing
    /// reactivation with a new WX value.
    wx_triggered: bool,
    /// Whether the window has been rendered on this line.
    window_rendered: bool,
    /// Last observed WX output value, used to detect mid-scanline WX
    /// changes that should clear the wx_triggered latch.
    last_wx_value: u8,
    /// Cached WX value for the NUKO comparator. On hardware, NUKO
    /// reads the DFF8 slave output, which lags the master by one clock
    /// edge. Updated unconditionally at the end of every mode3_rising
    /// from the live DFF output. check_trigger reads this instead of
    /// the live register, providing a 1-dot lag on mid-scanline WX writes.
    nuko_wx: u8,
    /// Window internal line counter. Incremented at scanline boundary
    /// when window_rendered is true. Used by the fetcher for tile
    /// map address generation.
    window_line_counter: u8,
    /// Window reactivation zero pixel (DMG only). Set when WX
    /// re-matches while the window is active with specific
    /// fetcher/FIFO conditions. Causes the next pixel output to use
    /// bg_color=0 without popping the BG shifter. The OBJ shifter is
    /// popped normally.
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

    /// Check if the window should start rendering at the current pixel
    /// position. Also detects window reactivation zero pixel conditions
    /// when the window is already active.
    ///
    /// On hardware, the NUKO comparator reads pix_count DFF Q-outputs
    /// combinationally (pre-SACU value). The PYCO DFF captures the NUKO
    /// match on ROCO, which derives from TYFA and requires POKY (modeled
    /// as `pygo`). The `pixel_counter` parameter must be the pre-SACU
    /// value (from `RisingPhaseInputs`) to model this correctly.
    ///
    /// `rydy_snapshot` is the phase-boundary snapshot (state_old) used
    /// for the reactivation check.
    pub(in crate::ppu) fn check_trigger(
        &mut self,
        rydy_snapshot: bool,
        fetcher: &mut TileFetcher,
        cascade: &mut FetchCascade,
        fine_scroll: &mut FineScroll,
        pixel_counter: u8,
        pygo: bool,
        regs: &PipelineRegisters,
        video: &VideoControl,
    ) {
        if !regs.control.window_enabled() {
            return;
        }
        if video.ly() < regs.window.y {
            return;
        }

        // Detect mid-scanline WX changes to clear the trigger suppression latch.
        if self.nuko_wx != self.last_wx_value {
            self.wx_triggered = false;
            self.last_wx_value = self.nuko_wx;
        }

        if pixel_counter != self.nuko_wx {
            return;
        }

        // PYGO gate: PYCO is clocked by ROCO (derived from TYFA), which
        // requires POKY (pygo) to be set. Without POKY, ROCO has no edges
        // and PYCO cannot capture the NUKO match. This prevents WX=0 from
        // triggering before the initial BG fetch completes.
        if !pygo {
            return;
        }

        // Window already active -- check for reactivation zero pixel (DMG only).
        // The hardware condition is GetTile T1 (first tick). Our WX check
        // runs in mode3_rising after SACU but before the rising fetcher advance,
        // so the fetcher has been ticked once (in mode3_falling): what was
        // dot=0 (T1) is now dot=1. So we check dot=1.
        // Reactivation requires the initial window fetch to have completed
        // (RYDY=0), modeling hardware's !window_is_being_fetched.
        if fetcher.fetching_window {
            if !rydy_snapshot && fetcher.fetch_counter < 2 {
                self.window_zero_pixel = true;
            }
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

    /// Reset for a new frame. Zeroes the window line counter (WLY),
    /// which accumulates across scanlines but resets at frame start.
    pub(in crate::ppu) fn reset_frame(&mut self) {
        self.window_line_counter = 0;
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
