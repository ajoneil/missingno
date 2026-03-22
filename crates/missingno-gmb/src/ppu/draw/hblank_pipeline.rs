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
    /// Updated at the end of fall() with the current dot's WODU.
    wodu: bool,
    /// FEPO captured at start of falling phase. Feeds wodu() for
    /// VOGA capture and TYFA computation. Persists across one dot
    /// because wodu() is evaluated at the start of the NEXT falling
    /// phase, before mode3_falling writes the new value.
    fepo: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            xymu: false,
            voga: false,
            wodu: false,
            fepo: false,
        }
    }

    /// WODU: combinational hblank gate. AND3(XYMU, XUGU, !FEPO).
    /// On hardware, WODU is not a latch — it's valid whenever its
    /// inputs are valid. TARU (STAT mode 0) reads WODU directly.
    pub(in crate::ppu) fn wodu(&self, xugu: bool) -> bool {
        self.xymu && xugu && !self.fepo
    }

    /// Falling edge (ALET clock): VOGA captures WODU_old, WEGO clears XYMU.
    ///
    /// WODU is combinational: AND3(XYMU, XUGU, !FEPO). It fires on the
    /// dot when pixel_counter reaches 167 and no sprite is matching.
    ///
    /// VOGA is DFF17 (ALET falling edge, depth 5). It captures WODU_old --
    /// the previous dot's WODU value. This introduces a 1-dot delay:
    /// WODU fires on dot N, VOGA captures it on dot N+1.
    ///
    /// WEGO = OR2(VID_RST, VOGA) clears XYMU immediately when VOGA
    /// goes high. VID_RST handled separately in reset().
    ///
    /// Race pair data (mode3-race-pairs.md):
    ///   VOGA: depth 7, diff 13 -- WODU_old at depth 0 (registered, earliest)
    ///   XYMU: depth 1 from VOGA, fan-out 25
    ///
    /// Returns (wodu_current, wodu_old).
    /// - wodu_current: live combinational value for STAT, LCD last_pixel
    /// - wodu_old: previous dot's value for TYFA/VYBO pixel clock gate
    pub(in crate::ppu) fn fall(&mut self, xugu: bool) -> (bool, bool) {
        let wodu_now = self.wodu(xugu);
        let wodu_old = self.wodu;

        // VOGA DFF17 captures WODU_old (previous dot's value), not
        // the current dot's WODU. Models the 1-dot propagation delay.
        if wodu_old {
            self.voga = true;
        }

        // WEGO = OR2(VID_RST, VOGA) clears XYMU.
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

    pub(in crate::ppu) fn voga(&self) -> bool {
        self.voga
    }

    pub(in crate::ppu) fn reset(&mut self) {
        self.xymu = false;
        self.voga = false;
        self.wodu = false;
        self.fepo = false;
    }
}
