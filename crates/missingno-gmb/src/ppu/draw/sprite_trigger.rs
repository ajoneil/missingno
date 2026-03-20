/// Sprite fetch trigger pipeline: TEKY → SOBU → SUDA → RYCE → TAKA.
///
/// A DFF chain that propagates the sprite fetch request signal (TEKY)
/// through two pipeline stages, with an edge detector (RYCE) that sets
/// a latch (TAKA) to freeze the pixel clock during sprite data fetch.
/// Same pattern as `FetchCascade` — clock and query.
///
/// Hardware clock ordering (from mode3-clock-domains.md):
///   SOBU: DFF17, captures TEKY on TAVA falling edge (depth 7)
///   SUDA: DFF17, captures SOBU on LAPE rising edge (depth 6)
///   RYCE: combinational edge detect (SOBU && !SUDA)
///   TAKA: NAND latch, set by RYCE, cleared by VEKU (fetch done)
///
/// TAVA is 2 inverters deeper than ALET in the clock chain. This is
/// deliberate: SOBU must capture TEKY_old, which depends on LYRY —
/// and LYRY depends on the fetcher counter, clocked by LEBO (gated
/// by ALET). The extra delay ensures the counter has settled.
///
/// Race pair data (mode3-race-pairs.md):
///   SECA (fetch start): depth 13, diff 13 (~65-195ns)
///   SOBU: clocked at depth 8-9, diff 7-8
///   SUDA: clocked at depth 7, diff 7
///
/// Consumers:
///   - `taka()` → gates pixel clock (TYFA suppressed while taka is set),
///     gates sprite fetch advance in mode3_rising
///   - `ryce()` return from `fall()` → triggers sprite fetch start
pub(in crate::ppu) struct SpriteTrigger {
    /// SOBU_SFETCH_REQp_evn: DFF17, latches on TAVA falling edge.
    sobu: bool,
    /// SUDA_SFETCH_REQp_odd: DFF17, latches on LAPE rising edge.
    suda: bool,
    /// TAKA NAND latch: sprite fetch running. Set by RYCE (SOBU
    /// rising edge detect), cleared by VEKU (sprite fetch done).
    taka: bool,
}

impl SpriteTrigger {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            sobu: false,
            suda: false,
            taka: false,
        }
    }

    /// Falling edge (TAVA clock): SOBU captures TEKY from the preceding
    /// rising phase. Returns true if RYCE fires (sprite fetch should start).
    ///
    /// TEKY is passed in as a parameter — it's combinational, computed
    /// during the preceding rising phase and bridged to falling. Same
    /// pattern as `FetchCascade::fall(lyry)`.
    ///
    /// RYCE is a combinational rising-edge detect: SOBU just went high,
    /// SUDA still holds the value from the previous rising phase.
    pub(in crate::ppu) fn fall(&mut self, teky: bool) -> bool {
        self.sobu = teky;
        let ryce = self.sobu && !self.suda;
        if ryce {
            self.taka = true;
        }
        ryce
    }

    /// Rising edge (LAPE clock): SUDA captures SOBU.
    pub(in crate::ppu) fn rise(&mut self) {
        self.suda = self.sobu;
    }

    /// Clear TAKA when sprite fetch completes (VEKU).
    pub(in crate::ppu) fn clear_taka(&mut self) {
        self.taka = false;
    }

    /// Reset all state at scanline boundary.
    pub(in crate::ppu) fn reset(&mut self) {
        self.sobu = false;
        self.suda = false;
        self.taka = false;
    }

    pub(in crate::ppu) fn taka(&self) -> bool {
        self.taka
    }
}
