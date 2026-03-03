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
    /// NYZE DFF — captures PUXA (fine scroll match) from previous
    /// phase. Used for POVA rising-edge detection: POVA = AND2(PUXA,
    /// !NYZE). Fires once per PUXA 0→1 transition.
    nyze: bool,
}

impl FineScroll {
    pub(super) fn new() -> Self {
        Self {
            count: 0,
            roxy: Roxy::Gating,
            nyze: false,
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

    /// Check fine scroll match (PUXA) and compute POVA trigger.
    ///
    /// PUXA is the combinational match (count == SCX & 7). If ROXY is
    /// still gating and PUXA fires, ROXY clears (one-shot per line).
    /// Returns POVA = AND2(PUXA, !NYZE) — the rising-edge trigger that
    /// fires once per PUXA 0→1 transition. POVA generates one extra
    /// LCD clock pulse via SEMU = OR2(TOBA, POVA).
    pub(super) fn check_scroll_match(&mut self, scx: u8) -> bool {
        let puxa = self.count == (scx & 7);
        let pova = puxa && !self.nyze;
        self.nyze = puxa;

        if self.roxy == Roxy::Gating && puxa {
            self.roxy = Roxy::Done;
        }

        pova
    }

    /// Reset for window trigger — counter resets, gating clears
    /// (window has no fine scroll).
    pub(super) fn reset_for_window(&mut self) {
        self.count = 0;
        self.roxy = Roxy::Done;
    }
}
