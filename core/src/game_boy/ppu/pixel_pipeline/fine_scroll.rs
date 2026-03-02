// --- Window hit (RYDY pixel clock gate) ---

/// RYDY NOR latch state — the window hit signal.
///
/// On hardware, RYDY is SET when the window X match fires (NUKO_WX_MATCHp)
/// and RESET when the window fetch completes (SUZU/MOSU path clears it).
/// While active, RYDY gates TYFA (via SOCY_WIN_HITn = not1(TOMU_WIN_HITp)),
/// freezing the entire pixel clock chain:
///   TYFA=0 → ROXO=0 (fine counter clock frozen)
///           → SEGU=1 → SACU=1 (pixel counter clock frozen)
///
/// The BG fetcher is NOT gated — it runs on LEBO (the half-cycle clock),
/// independent of TYFA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowHit {
    /// RYDY=0: no active window fetch stall. The pixel clock chain
    /// runs normally (subject to other gates like ROXY and POKY).
    Inactive,
    /// RYDY just activated this tick. `clkpipe_gate` reads
    /// state_old.RYDY=0, so the pixel counter advances once more.
    /// Transitions to Active at end of mode3_odd.
    Activating,
    /// RYDY=1: window fetch in progress. The pixel clock chain is
    /// frozen — fine counter and pixel counter do not advance.
    /// Cleared when the fetcher reaches Idle (SUZU fires).
    Active,
    /// RYDY just cleared (SUZU fired): pipe is loaded with window tile data,
    /// but `clkpipe_gate` still reads the old RYDY=1 value. Pixel clock
    /// remains frozen for this 1 tick. Transitions to Inactive on next tick.
    Clearing,
}

// --- Fine scroll (ROXY pixel clock gate) ---

/// ROXY NOR latch state. On hardware, ROXY gates the pixel clock
/// (SACU = or2(SEGU, ROXY)) until the fine scroll counter matches
/// SCX & 7. SET between lines (PAHA_RENDERINGn), RESET on fine
/// scroll match (POVA_FINE_MATCH_TRIGp_evn). One-shot per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Roxy {
    /// ROXY=1: pixel clock gated. The fine counter is still counting
    /// toward the SCX & 7 target.
    Gating,
    /// ROXY=0: pixel clock active. Fine scroll discard is complete
    /// for this line.
    Done,
}

/// Hardware fine scroll counter (RYKU/ROGA/RUBU) and pixel clock
/// gate (ROXY). The ROXY latch gates the pixel clock (SACU) until
/// the counter matches SCX & 7, implementing sub-tile fine scrolling.
pub(super) struct FineScroll {
    /// 3-bit counter (0–7).
    pub(super) count: u8,
    /// ROXY NOR latch — gates SACU until fine scroll match fires.
    roxy: Roxy,
}

impl FineScroll {
    pub(super) fn new() -> Self {
        Self {
            count: 0,
            roxy: Roxy::Gating,
        }
    }

    /// Whether the pixel clock is active (SACU ungated).
    pub(super) fn pixel_clock_active(&self) -> bool {
        self.roxy == Roxy::Done
    }

    /// Advance the fine counter by one dot (PECU clock).
    /// Self-stops at 7 (ROZE gate: nand3(CNT2, CNT1, CNT0)).
    pub(super) fn tick(&mut self) {
        if self.count < 7 {
            self.count += 1;
        }
    }

    /// TEVO → PASO: reset the fine counter to 0.
    pub(super) fn reset_counter(&mut self) {
        self.count = 0;
    }

    /// Check and clear the gating latch if count matches SCX & 7.
    /// One-shot: once cleared, stays cleared for the rest of the line.
    pub(super) fn check_scroll_match(&mut self, scx: u8) {
        if self.roxy == Roxy::Gating && self.count == (scx & 7) {
            self.roxy = Roxy::Done;
        }
    }

    /// Reset for window trigger — counter resets, gating clears
    /// (window has no fine scroll).
    pub(super) fn reset_for_window(&mut self) {
        self.count = 0;
        self.roxy = Roxy::Done;
    }
}
