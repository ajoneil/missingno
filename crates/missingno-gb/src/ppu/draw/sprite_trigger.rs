//! TEKY → SOBU → {RYCE → TAKA, SUDA}. Sprite fetch trigger pipeline.
//!
//! RYCE = AND2(!SUDA, SOBU); TAKA is the sprite-fetch-running NAND-latch.
//! TAKA carries over across scanlines until VEKU clears it.
pub(in crate::ppu) struct SpriteTrigger {
    /// SOBU captures TEKY on ALET rising.
    sobu: bool,
    /// SUDA captures SOBU on ALET falling.
    suda: bool,
    /// TAKA: SECA=NOR3(RYCE, ROSY, ATEJ) sets, VEKU=NOR2(WUTY, TAVE) clears.
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

    /// Returns true if RYCE fires (one-shot pulse at SOBU rise sets TAKA via SECA).
    pub(in crate::ppu) fn capture_sobu(&mut self, teky: bool) -> bool {
        self.sobu = teky;
        let ryce = self.sobu && !self.suda;
        if ryce {
            self.taka = true;
        }
        ryce
    }

    /// SUDA captures SOBU on the falling edge; the capture clears RYCE.
    pub(in crate::ppu) fn capture_suda(&mut self) {
        self.suda = self.sobu;
    }

    /// VEKU clear path: WUTY (fetch-done) or TAVE (startup carry-over).
    pub(in crate::ppu) fn clear_taka(&mut self) {
        self.taka = false;
    }

    /// SECA's ATEJ arm — line-end pulse re-asserts TAKA at each scanline boundary.
    pub(in crate::ppu) fn set_taka(&mut self) {
        self.taka = true;
    }

    pub(in crate::ppu) fn taka(&self) -> bool {
        self.taka
    }
}
