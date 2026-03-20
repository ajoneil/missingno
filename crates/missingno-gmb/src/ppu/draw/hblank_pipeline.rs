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
///   VOGA: depth 7, diff 15 — mode transition boundary shifted one dot
///   XYMU: depth 1 from VOGA, fan-out 25
pub(in crate::ppu) struct HblankPipeline {
    /// XYMU rendering latch (page 21). SET by AVAP (Mode 2→3),
    /// CLEAR by WEGO = OR2(VID_RST, VOGA).
    xymu: bool,
    /// VOGA DFF17: captures WODU on ALET falling edge. Feeds WEGO.
    /// Reset by TADY (line reset).
    voga: bool,
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
            fepo: false,
        }
    }

    /// WODU: combinational hblank gate. AND3(XYMU, XUGU, !FEPO).
    /// On hardware, WODU is not a latch — it's valid whenever its
    /// inputs are valid. TARU (STAT mode 0) reads WODU directly.
    pub(in crate::ppu) fn wodu(&self, xugu: bool) -> bool {
        self.xymu && xugu && !self.fepo
    }

    /// Falling edge: evaluate WODU, capture into VOGA, apply WEGO.
    ///
    /// VOGA captures WODU on ALET. WEGO = OR2(VID_RST, VOGA) clears
    /// XYMU. VID_RST is handled separately in reset(); here we model
    /// the VOGA path.
    ///
    /// Returns the WODU value for callers that need it (TYFA, LCD).
    pub(in crate::ppu) fn fall(&mut self, xugu: bool) -> bool {
        let wodu = self.wodu(xugu);
        if wodu {
            self.voga = true;
        }
        if self.voga {
            self.xymu = false;
        }
        wodu
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
        self.fepo = false;
    }
}
