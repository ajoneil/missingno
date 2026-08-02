//! Bus-settling holds after a PPU mode-signal onset. Each watched signal's
//! 0→1 drives a contended bus whose PRE (pre-onset) value stands until the
//! driver resolves; a double-speed CPU read landing in that window reads PRE.

use bitflags::bitflags;

bitflags! {
    /// The mode signals a contending CPU read resolves against, sampled
    /// together once per master half-edge.
    #[derive(Clone, Copy, PartialEq, Eq, Default)]
    pub(super) struct OnsetSignals: u8 {
        /// not_if1's mode-2 bit (WUGA): OAM scan (ACYL) or rendering (XYMU).
        const MODE2_BIT = 1 << 0;
        /// XYMU — mode 3.
        const RENDERING = 1 << 1;
        /// The OAM read lock, from the RUTU onset through ACYL/XYMU.
        const OAM_LOCK = 1 << 2;
        /// POPU — mode 1.
        const VERTICAL_BLANK = 1 << 3;
    }
}

/// Master half-edges the mode-2 `not_if1` bus holds PRE after a BESU 0→1 — the
/// slow companion-driver contention resolves within ~1 dot.
const MODE2_BIT_SETTLE: u8 = 2;

/// The mode-3 (XYMU) bus holds PRE after a rendering 0→1 (AVAP↑/XYMU↓) onset —
/// the symmetric counterpart to `not_if1`, for the mode-2→3 onset contention.
const RENDERING_SETTLE: u8 = 2;

/// The mode-1 (POPU) bit reaches the STAT read view slower than ROPO's
/// coincidence clear, so an onset-window read sees the pre-onset mode with the
/// live coincidence.
const VERTICAL_BLANK_SETTLE: u8 = 2;

/// The OAM read lock holds PRE (accessible) after the RUTU onset before ACYL
/// settles the gate closed — the OAM analogue of the not_if1 hold.
const OAM_LOCK_SETTLE: u8 = 4;

/// The onset contention holds, advanced together once per master half-edge:
/// the master half-edges each contended bus has left to hold its PRE value.
#[derive(Default)]
pub(super) struct OnsetSettles {
    mode2_bit: u8,
    rendering: u8,
    oam_lock: u8,
    vertical_blank: u8,
    /// The signals as of the previous half-edge, for onset detection.
    previous: OnsetSignals,
    /// The STAT mode bits just before the mode-3 onset — XYMU's rise drives
    /// both OR-tree bits through the contended driver, so an onset-window read
    /// sees this snapshot (mode 2 on a scanned line, mode 0 on the enable line).
    rendering_onset_pre_stat: u8,
}

impl OnsetSettles {
    /// Advance one master half-edge: a signal's 0→1 (re)arms its hold, an armed
    /// hold otherwise drains toward zero. With no signal changed and every bus
    /// already settled there is nothing to arm and nothing to drain.
    pub(super) fn advance(&mut self, live: OnsetSignals) {
        if live == self.previous && self.all_settled() {
            return;
        }
        let previous = self.previous;
        if live.contains(OnsetSignals::RENDERING) && !previous.contains(OnsetSignals::RENDERING) {
            self.rendering_onset_pre_stat = ((previous.contains(OnsetSignals::MODE2_BIT) as u8)
                << 1)
                | previous.contains(OnsetSignals::VERTICAL_BLANK) as u8;
        }
        let onsets = live - previous;
        advance_hold(
            &mut self.mode2_bit,
            onsets.contains(OnsetSignals::MODE2_BIT),
            MODE2_BIT_SETTLE,
        );
        advance_hold(
            &mut self.rendering,
            onsets.contains(OnsetSignals::RENDERING),
            RENDERING_SETTLE,
        );
        advance_hold(
            &mut self.oam_lock,
            onsets.contains(OnsetSignals::OAM_LOCK),
            OAM_LOCK_SETTLE,
        );
        advance_hold(
            &mut self.vertical_blank,
            onsets.contains(OnsetSignals::VERTICAL_BLANK),
            VERTICAL_BLANK_SETTLE,
        );
        self.previous = live;
    }

    fn all_settled(&self) -> bool {
        (self.mode2_bit | self.rendering | self.oam_lock | self.vertical_blank) == 0
    }

    pub(super) fn mode2_bit_settled(&self) -> bool {
        self.mode2_bit == 0
    }

    pub(super) fn in_rendering_onset(&self) -> bool {
        self.rendering > 0
    }

    pub(super) fn in_oam_lock_onset(&self) -> bool {
        self.oam_lock > 0
    }

    pub(super) fn in_vertical_blank_onset(&self) -> bool {
        self.vertical_blank > 0
    }

    pub(super) fn rendering_onset_pre_stat(&self) -> u8 {
        self.rendering_onset_pre_stat
    }
}

fn advance_hold(remaining: &mut u8, onset: bool, reload: u8) {
    if onset {
        *remaining = reload;
    } else if *remaining > 0 {
        *remaining -= 1;
    }
}
