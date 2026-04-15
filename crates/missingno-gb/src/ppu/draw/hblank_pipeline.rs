/// Hblank pipeline: FEPO → WODU → VOGA → WEGO → clears XYMU.
///
/// Models the hardware path that terminates Mode 3 rendering. WODU
/// fires combinationally when the pixel counter reaches 167 and no
/// sprite is matching. VOGA (DFF17, ALET falling) captures WODU,
/// producing WEGO which clears the XYMU rendering latch.
///
/// Hardware clock: VOGA is DFF17 on ALET (falling, depth 5).
/// WODU is combinational: AND2(XUGU, !FEPO). WODU is purely a
/// function of the pixel counter decode and sprite match — it does
/// not depend on XYMU. During HBlank, WODU stays high (PX frozen
/// at 167, FEPO=0), which is correct: CLKPIPE stays frozen via
/// VYBO, and VOGA is already set keeping XYMU cleared.
///
/// XYMU is a NOR latch cleared by WEGO = OR2(VID_RST, VOGA).
///
/// Race pair data (mode3-race-pairs.md):
///   VOGA: depth 7, diff 13 -- WODU_old at depth 0 (registered, earliest)
///   XYMU: depth 1 from VOGA, fan-out 25
pub(in crate::ppu) struct HblankPipeline {
    /// XYMU rendering latch (page 21). SET by AVAP (Mode 2→3),
    /// CLEAR by WEGO = OR2(VID_RST, VOGA).
    xymu: bool,
    /// VOGA DFF17: captures WODU on ALET falling edge (half-cycle
    /// delay). Feeds WEGO. Reset by TADY (line reset).
    voga: bool,
    /// FEPO captured at start of falling phase. Feeds wodu() and
    /// TYFA computation. Latched because FEPO changes mid-fall but
    /// WODU needs the value from the start of the falling phase.
    fepo: bool,
    /// Whether XYMU was true before settle_alet() cleared it.
    /// fall() needs this to gate mode3_falling() — on the dot
    /// VOGA fires, XYMU clears in settle_alet but mode3_falling
    /// still needs to run for the final fetcher/TYFA work.
    xymu_before_settle: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            xymu: false,
            voga: false,
            fepo: false,
            xymu_before_settle: false,
        }
    }

    /// WODU: combinational hblank gate. AND2(XUGU, !FEPO).
    /// On hardware, WODU is purely combinational — it does not
    /// depend on XYMU. TARU (STAT mode 0) reads WODU directly.
    pub(in crate::ppu) fn wodu(&self, xugu: bool) -> bool {
        xugu && !self.fepo
    }

    /// ALET falling edge: VOGA captures WODU, WEGO clears XYMU.
    ///
    /// On hardware, ALET falls at the boundary between sub-phases (e.g.
    /// F->G), before the CPU's BUKE window opens. This method models
    /// just the DFF capture and its combinational consequences.
    ///
    /// Called from the executor between PPU rise and CPU bus read, so
    /// the CPU sees post-XYMU state. The remaining fall() work
    /// (fetcher, cascade, TYFA) runs later in the falling phase.
    pub(in crate::ppu) fn settle_alet(&mut self, xugu: bool) {
        // Capture XYMU state before any clearing — fall() uses this to
        // gate mode3_falling() on the transition dot.
        self.xymu_before_settle = self.xymu;

        // WODU is purely combinational (no XYMU dependency), so it's
        // valid at any point during the dot.
        let wodu_now = self.wodu(xugu);

        // VOGA DFF17 captures WODU on ALET falling. On hardware this is
        // the same dot WODU fires (half-cycle delay, not full-dot).
        let voga_will_fire = self.voga || wodu_now;

        // WEGO = OR2(VID_RST, VOGA) clears XYMU. Apply early.
        if voga_will_fire {
            self.xymu = false;
        }
    }

    /// Falling edge (ALET clock): VOGA captures WODU, WEGO clears XYMU.
    ///
    /// settle_alet() has already cleared XYMU for the CPU's benefit and
    /// set xymu_before_settle. This method does the full VOGA/wodu
    /// computation — the XYMU clear is idempotent.
    ///
    /// Returns wodu_current (live combinational value for STAT, LCD last_pixel).
    pub(in crate::ppu) fn fall(&mut self, xugu: bool) -> bool {
        // WODU is purely combinational — no XYMU dependency, so always valid.
        let wodu_now = self.wodu(xugu);

        // VOGA DFF17 captures CURRENT dot's WODU (half-cycle delay).
        // On hardware, WODU's inputs are valid before ALET falls.
        if wodu_now {
            self.voga = true;
        }

        // WEGO = OR2(VID_RST, VOGA) clears XYMU (idempotent if
        // settle_alet already cleared it).
        if self.voga {
            self.xymu = false;
        }

        wodu_now
    }

    /// Latch FEPO for the next dot's wodu() evaluation. Called in
    /// mode3_falling after FEPO is evaluated but before it changes.
    pub(in crate::ppu) fn latch_fepo(&mut self, fepo: bool) {
        self.fepo = fepo;
    }

    /// AVAP: Mode 2→3 transition, set XYMU.
    pub(in crate::ppu) fn set_xymu(&mut self) {
        self.xymu = true;
    }

    pub(in crate::ppu) fn xymu(&self) -> bool {
        self.xymu
    }

    /// Whether XYMU was true before settle_alet() ran this dot.
    /// Used by rendering.fall() to gate mode3_falling() — on the
    /// dot VOGA fires, XYMU is already cleared but the final
    /// mode3 falling work still needs to run.
    pub(in crate::ppu) fn xymu_before_settle(&self) -> bool {
        self.xymu_before_settle
    }

    pub(in crate::ppu) fn voga(&self) -> bool {
        self.voga
    }

    pub(in crate::ppu) fn reset(&mut self) {
        self.xymu = false;
        self.voga = false;
        self.fepo = false;
        self.xymu_before_settle = false;
    }
}
