/// Hblank pipeline: FEPO → WODU → VOGA → WEGO → clears XYMU.
///
/// Models the hardware path that terminates Mode 3 rendering. WODU
/// fires combinationally when the pixel counter reaches 167 and no
/// sprite is matching. VOGA (DFF17, ALET falling) captures WODU,
/// producing WEGO which clears the XYMU rendering latch.
///
/// Hardware clock: VOGA is DFF17 on ALET (falling, depth 5).
/// WODU is combinational (AND3(XYMU, XUGU, !FEPO)).
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
    /// wodu_pre_settle() for VOGA capture. Also feeds TYFA
    /// computation. Latched because FEPO changes mid-fall but WODU
    /// needs the value from the start of the falling phase.
    fepo: bool,
    /// Whether XYMU was true before settle_alet() cleared it.
    /// fall() needs this to gate mode3_falling() — on the dot
    /// VOGA fires, XYMU clears in settle_alet but mode3_falling
    /// still needs to run for the final fetcher/TYFA work.
    xymu_before_settle: bool,
    /// Whether settle_alet() ran this dot. When false (e.g. during
    /// the Mode 2→3 transition where scanning was still true during
    /// settle_alet), fall() must compute values itself.
    settled: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            xymu: false,
            voga: false,
            fepo: false,
            xymu_before_settle: false,
            settled: false,
        }
    }

    /// WODU: combinational hblank gate. AND3(XYMU, XUGU, !FEPO).
    /// On hardware, WODU is not a latch — it's valid whenever its
    /// inputs are valid. TARU (STAT mode 0) reads WODU directly.
    pub(in crate::ppu) fn wodu(&self, xugu: bool) -> bool {
        self.xymu && xugu && !self.fepo
    }

    /// Compute WODU using pre-settle XYMU state. On the VOGA dot,
    /// settle_alet() has already cleared XYMU, but WODU's inputs were
    /// valid before the clear. This recovers the correct WODU value.
    fn wodu_pre_settle(&self, xugu: bool) -> bool {
        self.xymu_before_settle && xugu && !self.fepo
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
        self.settled = true;

        // Compute WODU from current state — pixel_counter was already
        // incremented on the rising phase, so xugu reflects this dot.
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
    /// settle_alet() may have already cleared XYMU for the CPU's benefit.
    /// This method does the full VOGA/wodu computation — the XYMU clear
    /// is idempotent.
    ///
    /// Returns wodu_current (live combinational value for STAT, LCD last_pixel).
    pub(in crate::ppu) fn fall(&mut self, xugu: bool) -> bool {
        // xymu_before_settle was set by settle_alet if it ran. If settle_alet
        // didn't run (scanning was true), capture it now before fall mutates.
        if !self.settled {
            self.xymu_before_settle = self.xymu;
        }

        // WODU is combinational from XYMU. If settle_alet() already ran,
        // XYMU was cleared by VOGA, so self.wodu(xugu) would return false.
        // Use pre-settle XYMU to recover the correct WODU value.
        let wodu_now = if self.settled {
            self.wodu_pre_settle(xugu)
        } else {
            self.wodu(xugu)
        };
        self.settled = false;

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
        self.settled = false;
    }
}
