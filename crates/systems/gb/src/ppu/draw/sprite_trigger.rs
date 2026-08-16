//! TEKY → SOBU → {RYCE → TAKA, SUDA}. Sprite fetch trigger pipeline.
//!
//! RYCE = AND2(!SUDA, SOBU); TAKA is the sprite-fetch-running NAND-latch.
//! TAKA carries over across scanlines until VEKU clears it.
pub(in crate::ppu) struct SpriteTrigger {
    /// SOBU captures TEKY on ALET rising.
    fetch_request: bool,
    /// SUDA captures SOBU on ALET falling.
    fetch_request_delayed: bool,
    /// TAKA: SECA=NOR3(RYCE, ROSY, ATEJ) sets, VEKU=NOR2(WUTY, TAVE) clears.
    fetch_running: bool,
}

impl SpriteTrigger {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            fetch_request: false,
            fetch_request_delayed: false,
            fetch_running: false,
        }
    }

    /// ALET rise: SOBU captures TEKY. Returns true if RYCE fires (one-shot at SOBU rise sets
    /// the fetch-running latch via SECA).
    pub(in crate::ppu) fn tick_trigger_on_rise(&mut self, x_match_trigger: bool) -> bool {
        self.fetch_request = x_match_trigger;
        let fetch_request_fired = self.fetch_request && !self.fetch_request_delayed;
        if fetch_request_fired {
            self.fetch_running = true;
        }
        fetch_request_fired
    }

    /// ALET fall: SUDA captures SOBU; this clears RYCE on the next rise.
    pub(in crate::ppu) fn tick_trigger_on_fall(&mut self) {
        self.fetch_request_delayed = self.fetch_request;
    }

    /// VEKU clear path: WUTY (fetch-done) or TAVE (startup carry-over).
    pub(in crate::ppu) fn clear_fetch_running(&mut self) {
        self.fetch_running = false;
    }

    /// SECA's ATEJ arm — line-end pulse re-asserts the fetch-running latch at each scanline boundary.
    pub(in crate::ppu) fn arm_at_line_end(&mut self) {
        self.fetch_running = true;
    }

    pub(in crate::ppu) fn fetch_running(&self) -> bool {
        self.fetch_running
    }
}
