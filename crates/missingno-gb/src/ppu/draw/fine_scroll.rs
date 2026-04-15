// --- Fine scroll (ROXY pixel clock gate) ---

/// ROXY NOR latch state. On hardware, ROXY gates the pixel clock
/// (SACU = or2(SEGU, ROXY)) until the fine scroll counter matches
/// SCX & 7. SET between lines (PAHA_RENDERINGn), RESET on fine
/// scroll match (POVA fine match trigger). One-shot per line.
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
pub(in crate::ppu) struct FineScroll {
    /// 3-bit counter (0–7).
    pub(in crate::ppu) count: u8,
    /// ROXY NOR latch — gates SACU until fine scroll match fires.
    roxy: Roxy,
    /// NYZE DFF — captures PUXA (fine scroll match) from previous
    /// phase. Used for POVA rising-edge detection: POVA = AND2(PUXA,
    /// !NYZE). Fires once per PUXA 0→1 transition.
    nyze: bool,
    /// POHU comparator result, computed on falling (alet rises).
    /// PUXA captures this on the next rising (alet falls) when ROXO fires.
    pohu: bool,
}

impl FineScroll {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            count: 0,
            roxy: Roxy::Gating,
            nyze: false,
            pohu: false,
        }
    }

    /// Whether the pixel clock is active (SACU ungated).
    pub(in crate::ppu) fn pixel_clock_active(&self) -> bool {
        self.roxy == Roxy::Done
    }

    /// Advance the fine counter by one dot (PECU clock).
    /// Self-stops at 7 (ROZE gate: nand3(CNT2, CNT1, CNT0)).
    pub(in crate::ppu) fn tick(&mut self) {
        if self.count < 7 {
            self.count += 1;
        }
    }

    /// TEVO → PASO: reset the fine counter to 0.
    pub(in crate::ppu) fn reset_counter(&mut self) {
        self.count = 0;
    }

    /// Falling phase: compute POHU comparator result from current count.
    /// On hardware, POHU is combinational (count == SCX & 7), and ROXO
    /// captures it into PUXA on the falling edge. We store it for the
    /// next rising phase's capture step.
    pub(in crate::ppu) fn compare_falling(&mut self, scx: u8) {
        self.pohu = self.count == (scx & 7);
    }

    /// Rising phase: capture PUXA from stored POHU, edge-detect POVA,
    /// and clear ROXY if matched.
    ///
    /// Only call when TYFA is active (ROXO fires).
    /// Returns POVA = AND2(PUXA, !NYZE).
    pub(in crate::ppu) fn capture_rising(&mut self) -> bool {
        let puxa = self.pohu;
        let pova = puxa && !self.nyze;
        self.nyze = puxa;

        if self.roxy == Roxy::Gating && puxa {
            self.roxy = Roxy::Done;
        }

        pova
    }

    /// Reset for window trigger — counter resets, gating clears
    /// (window has no fine scroll).
    pub(in crate::ppu) fn reset_for_window(&mut self) {
        self.count = 0;
        self.roxy = Roxy::Done;
        self.pohu = false;
    }
}
