//! Sprite fetch trigger pipeline.
//!
//! SOBU captures TEKY on TAVA rising (= LAPE falling = ALET rising).
//! SUDA captures SOBU on the complementary edge (LAPE rising = ALET
//! falling). The methods are named for the DFFs they clock.

/// Sprite fetch trigger pipeline: TEKY → SOBU → {RYCE → TAKA, SUDA}.
///
/// Collapses the hardware chain:
///
///   SOBU (dffr, clk=TAVA/clk5) captures TEKY (=AND4 of FEPO, !RYDY,
///   LYRY, !TAKA)
///   SUDA (dffr, clk=LAPE/clk4) captures SOBU one edge later
///   RYCE = AND2(!SUDA, SOBU) — one-shot pulse at SOBU rise, clears
///   when SUDA captures
///   TAKA = nand_latch, set via SECA (RYCE-derived), cleared via
///   VEKU = NOR2(WUTY, TAVE); output freezes SACU for 6 dots via VYBO
///   halt path
///
/// SOWO = NOT(TAKA) is the local feedback inverter that blocks TEKY
/// re-trigger while a fetch is active — handled by the `!taka()`
/// term at TEKY's emulator call site.
///
/// SOBU and SUDA have no reset (r_n tied high via vypo); TAKA carries
/// over across scanline boundaries until VEKU clears it.
///
/// Consumers:
///   - `taka()` → gates SACU via TYFA suppression
///   - `capture_sobu()` return → triggers sprite fetch start
pub(in crate::ppu) struct SpriteTrigger {
    /// SOBU: captures TEKY on TAVA rising (= ALET rising).
    sobu: bool,
    /// SUDA: captures SOBU on LAPE rising (= ALET falling).
    suda: bool,
    /// TAKA NAND latch: sprite fetch running. Set by RYCE (= AND2
    /// of SOBU, !SUDA — one-shot); cleared by VEKU = NOR2(WUTY, TAVE).
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

    /// SOBU captures TEKY on TAVA rising (= ALET rising). Returns
    /// true if RYCE fires — the one-shot AND2(!SUDA, SOBU) pulse at
    /// SOBU's rise sets TAKA via SECA.
    pub(in crate::ppu) fn capture_sobu(&mut self, teky: bool) -> bool {
        self.sobu = teky;
        let ryce = self.sobu && !self.suda;
        if ryce {
            self.taka = true;
        }
        ryce
    }

    /// SUDA captures SOBU on LAPE rising (= ALET falling). The capture
    /// clears RYCE (= AND2(!SUDA, SOBU)) — ending the one-shot pulse.
    pub(in crate::ppu) fn capture_suda(&mut self) {
        self.suda = self.sobu;
    }

    /// Clear TAKA via VEKU = NOR2(WUTY, TAVE). Called from both arms:
    /// WUTY (sprite-fetch-done) and TAVE (cascade-startup carry-over clear).
    pub(in crate::ppu) fn clear_taka(&mut self) {
        self.taka = false;
    }

    pub(in crate::ppu) fn taka(&self) -> bool {
        self.taka
    }
}
