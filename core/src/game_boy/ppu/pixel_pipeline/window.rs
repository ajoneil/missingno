// --- Window trigger logic ---

use crate::game_boy::ppu::{PipelineRegisters, VideoControl};

use super::fetcher::{FetcherStep, FetcherTick, TileFetcher};
use super::fine_scroll::FineScroll;
use super::shifters::BgShifter;

/// Check if the window should start rendering at the current pixel position.
/// Also detects window reactivation zero pixel conditions when the window
/// is already active.
///
/// On hardware, the NUKO comparator reads post-increment pixel_counter
/// and DFF8 slave WX output on DELTA_ODD. The PYCO DFF captures the
/// match on the next DELTA_EVEN.
pub(super) fn check_window_trigger(
    rydy: bool,
    rydy_pending: &mut bool,
    fetcher: &mut TileFetcher,
    nyka: &mut bool,
    pory: &mut bool,
    bg_shifter: &mut BgShifter,
    fine_scroll: &mut FineScroll,
    window_zero_pixel: &mut bool,
    wx_triggered: &mut bool,
    window_rendered: &mut bool,
    pixel_counter: u8,
    last_wx_value: &mut u8,
    nuko_wx: u8,
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
    if nuko_wx != *last_wx_value {
        *wx_triggered = false;
        *last_wx_value = nuko_wx;
    }

    if pixel_counter != nuko_wx {
        return;
    }

    // Window already active -- check for reactivation zero pixel (DMG only).
    // The hardware condition is GetTile T1 (first tick). Our WX check
    // runs in mode3_odd after SACU but before the ODD fetcher advance,
    // so the fetcher has been ticked once (in mode3_even): what was
    // dot=0 (T1) is now dot=1. So we check dot=1.
    // Reactivation requires the initial window fetch to have completed
    // (RYDY=0), modeling hardware's !window_is_being_fetched.
    if fetcher.fetching_window {
        if !rydy
            && !bg_shifter.is_empty()
            && fetcher.step == FetcherStep::GetTile
            && fetcher.tick == FetcherTick::T2
            && !bg_shifter.is_empty()
        {
            *window_zero_pixel = true;
        }
        return;
    }

    // WX already matched this line — suppress the comparator.
    if *wx_triggered {
        return;
    }

    // Window trigger: reset fine scroll, restart fetcher, and reset
    // cascade DFFs so a new startup fetch begins. The BG/OBJ shifters
    // are NOT cleared — hardware doesn't clear them. MOSU loads stale
    // tile_temp into the BG pipe (never visible since the pixel clock
    // freezes), and SUZU/TEVO later overwrites with window tile data.
    //
    // RYDY SET is deferred to mode3_even of the next dot (via
    // rydy_pending), modeling the PYCO->NUNU pipeline delay. TYFA
    // reads old-RYDY=0 on this dot and the next, allowing one more
    // SACU fire before the pixel clock freezes.
    *wx_triggered = true;
    fine_scroll.reset_for_window();
    *rydy_pending = true;
    fetcher.reset_for_window();
    // NAFY: window mode trigger always resets NYKA and PORY, forcing the
    // startup cascade (NYKA→PORY→PYGO) to re-propagate after the window
    // tile fetch completes before the pixel clock can resume.
    *nyka = false;
    *pory = false;
    *window_rendered = true;
}
