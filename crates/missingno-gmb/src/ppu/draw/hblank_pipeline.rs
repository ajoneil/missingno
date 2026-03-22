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
    /// VOGA DFF17: captures WODU on ALET falling edge. Feeds WEGO.
    /// Reset by TADY (line reset).
    voga: bool,
    /// WODU value from the previous dot. VOGA's D input is
    /// reg_old.WODU_HBLANK_GATEp_odd.out_old() -- the WODU that was
    /// valid during the preceding dot. Also consumed by VYBO (pixel
    /// clock gate) which reads WODU_old for the same reason.
    /// Updated by settle_alet() with the current dot's WODU.
    wodu: bool,
    /// FEPO captured at start of falling phase. Feeds wodu() for
    /// VOGA capture and TYFA computation. Persists across one dot
    /// because wodu() is evaluated at the start of the NEXT falling
    /// phase, before mode3_falling writes the new value.
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
            wodu: false,
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

    /// ALET falling edge: VOGA captures WODU_old, WEGO clears XYMU.
    ///
    /// On hardware, ALET falls at the boundary between sub-phases (e.g.
    /// F->G), before the CPU's BUKE window opens. This method models
    /// just the DFF capture and its combinational consequences.
    ///
    /// Called from the executor between PPU rise and CPU bus read, so
    /// the CPU sees post-XYMU state. The remaining fall() work
    /// (fetcher, cascade, TYFA) runs later in the falling phase.
    pub(in crate::ppu) fn settle_alet(&mut self) {
        // Capture XYMU state before any clearing — fall() uses this to
        // gate mode3_falling() on the transition dot.
        self.xymu_before_settle = self.xymu;
        self.settled = true;

        // Preview what VOGA/XYMU will do this dot, and apply the XYMU
        // clear early so the CPU sees post-XYMU state. The full VOGA
        // capture + wodu update happens later in fall().
        let wodu_old = self.wodu;

        // Preview VOGA: will it capture wodu_old this dot?
        let voga_will_fire = self.voga || wodu_old;

        // WEGO = OR2(VID_RST, VOGA) clears XYMU. Apply early.
        if voga_will_fire {
            self.xymu = false;
        }
    }

    /// Falling edge (ALET clock): VOGA captures WODU_old, WEGO clears XYMU.
    ///
    /// settle_alet() may have already cleared XYMU for the CPU's benefit.
    /// This method does the full VOGA/wodu computation — the XYMU clear
    /// is idempotent.
    ///
    /// Returns (wodu_current, wodu_old).
    /// - wodu_current: live combinational value for STAT, LCD last_pixel
    /// - wodu_old: previous dot's value for TYFA/VYBO pixel clock gate
    pub(in crate::ppu) fn fall(&mut self, xugu: bool) -> (bool, bool) {
        // xymu_before_settle was set by settle_alet if it ran. If settle_alet
        // didn't run (scanning was true), capture it now before fall mutates.
        if !self.settled {
            self.xymu_before_settle = self.xymu;
        }
        self.settled = false;

        let wodu_now = self.wodu(xugu);
        let wodu_old = self.wodu;

        // VOGA DFF17 captures WODU_old (previous dot's value).
        if wodu_old {
            self.voga = true;
        }

        // WEGO = OR2(VID_RST, VOGA) clears XYMU (idempotent if
        // settle_alet already cleared it).
        if self.voga {
            self.xymu = false;
        }

        // Store current WODU for next dot's VOGA capture.
        self.wodu = wodu_now;

        (wodu_now, wodu_old)
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
        self.wodu = false;
        self.fepo = false;
        self.xymu_before_settle = false;
        self.settled = false;
    }
}
