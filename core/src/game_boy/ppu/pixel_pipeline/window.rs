// --- Window trigger logic ---

use crate::game_boy::ppu::{PipelineRegisters, VideoControl};

use super::fetcher::{FetcherStep, FetcherTick, StartupFetch, TileFetcher};
use super::fine_scroll::{FineScroll, WindowHit};
use super::shifters::BgShifter;

/// Check if the window should start rendering at the current pixel position.
/// Also detects window reactivation zero pixel conditions when the window
/// is already active.
///
/// On hardware, the PYCO_WIN_MATCHp signal fires on DELTA_EVEN.
pub(super) fn check_window_trigger(
    window_hit: &mut WindowHit,
    fetcher: &mut TileFetcher,
    startup_fetch: &mut Option<StartupFetch>,
    bg_shifter: &mut BgShifter,
    fine_scroll: &mut FineScroll,
    window_zero_pixel: &mut bool,
    wx_triggered: &mut bool,
    window_rendered: &mut bool,
    pixel_counter: u8,
    last_wx_value: &mut u8,
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
    let current_wx = regs.window.x_plus_7.output();
    if current_wx != *last_wx_value {
        *wx_triggered = false;
        *last_wx_value = current_wx;
    }

    if pixel_counter != current_wx {
        return;
    }

    // Window already active — check for reactivation zero pixel (DMG only).
    // The hardware condition is GetTile T1 (first tick). Since our WX check
    // runs after advance_bg_fetcher in mode3_even, the fetcher has already
    // been ticked: what was dot=0 (T1) is now dot=1. So we check dot=1.
    // Reactivation requires the initial window fetch to have completed
    // (window_hit == Inactive), modeling hardware's !window_is_being_fetched.
    if fetcher.fetching_window {
        if *window_hit == WindowHit::Inactive
            && startup_fetch.is_none()
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
    *wx_triggered = true;
    fine_scroll.reset_for_window();
    *window_hit = WindowHit::Activating;
    fetcher.reset_for_window();
    if startup_fetch.is_some() {
        *startup_fetch = Some(StartupFetch::FirstTile);
    }
    *window_rendered = true;
}
