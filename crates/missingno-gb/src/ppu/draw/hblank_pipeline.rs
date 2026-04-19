//! Hblank pipeline — the Mode 3→0 termination path.
//!
//! Signal naming follows the project's PPU timing spec. Netlist gate
//! names (XYMU, VOGA, WEGO, TOFU, etc. from the dmgcpu netlist,
//! msinger/dmg-schematics) appear in doc comments for traceability.
//! See `receipts/ppu-overhaul/ppu-timing-model-spec.md` §2.4 and §6.3
//! for the hardware reference; see
//! `receipts/ppu-overhaul/spec-conventions.md` for the conventions.

/// Hblank pipeline: FEPO → WODU → VOGA → WEGO → clears XYMU.
///
/// Models the hardware path that terminates Mode 3 rendering. WODU
/// fires combinationally when the pixel counter reaches 167 and no
/// sprite is matching. VOGA (DFF17, ALET rising) captures WODU,
/// producing WEGO which clears the XYMU rendering latch.
///
/// Hardware clock: VOGA is DFF17 on ALET (rising; same-dot capture,
/// Question C).
/// WODU is combinational: AND2(XUGU, !FEPO). WODU is purely a
/// function of the pixel counter decode and sprite match — it does
/// not depend on XYMU. During HBlank, WODU stays high (PX frozen
/// at 167, FEPO=0), which is correct: CLKPIPE stays frozen via
/// VYBO, and VOGA is already set keeping XYMU cleared.
///
/// XYMU is a NOR latch cleared by WEGO = OR2(VID_RST, VOGA).
///
/// **Polarity note**: spec XYMU = 0 during Mode 3 (active-low "not
/// rendering"). The emulator's `rendering_active: bool` is `true` during
/// Mode 3 — semantic polarity, opposite sign from the spec's XYMU.
/// Set by AVAP (Mode 2→3), cleared by WEGO.
///
/// Race pair data (mode3-race-pairs.md):
///   VOGA: depth 7, diff 13 -- WODU_old at depth 0 (registered, earliest)
///   XYMU: depth 1 from VOGA, fan-out 25
pub(in crate::ppu) struct HblankPipeline {
    /// Rendering-mode latch. XYMU (nor_latch) — hardware uses active-low.
    ///
    /// `true` when Mode 3 rendering is active (opposite polarity from
    /// spec XYMU). SET by AVAP (Mode 2→3), CLEAR by WEGO = OR2(VID_RST, VOGA).
    rendering_active: bool,
    /// HBLANK capture DFF. VOGA (dffr, captures WODU on ALET rising edge).
    ///
    /// Feeds WEGO. Reset by TADY (line reset).
    ///
    /// Spec §6.3: "Mode 3 ends one alet-edge after the H-blank
    /// condition is combinationally true." The one-edge delay is the
    /// half-dot from rise() (where WODU goes true when PX=167) to
    /// fall() (where VOGA captures). This is naturally modeled by the
    /// rise-then-fall execution order.
    voga: bool,
    /// Sprite X priority aggregate, latched at start of falling phase.
    /// FEPO (or2 of FOVE, FEFY).
    ///
    /// Feeds `hblank_condition()` and TYFA computation. Latched because
    /// FEPO changes mid-fall but WODU needs the value from the start
    /// of the falling phase.
    fepo: bool,
    /// Snapshot of `rendering_active` before settle_alet() may have
    /// cleared it. Corresponds to XYMU's pre-settle state.
    ///
    /// fall() needs this to gate mode3_falling() — on the dot VOGA
    /// fires, XYMU clears in settle_alet but mode3_falling still
    /// needs to run for the final fetcher/TYFA work.
    rendering_before_settle: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            rendering_active: false,
            voga: false,
            fepo: false,
            rendering_before_settle: false,
        }
    }

    /// WODU: combinational hblank gate. AND2(XUGU, !FEPO).
    /// On hardware, WODU is purely combinational — it does not
    /// depend on XYMU. TARU (STAT mode 0) reads WODU directly.
    pub(in crate::ppu) fn wodu(&self, xugu: bool) -> bool {
        xugu && !self.fepo
    }

    /// ALET rising edge: VOGA captures WODU, WEGO clears XYMU.
    ///
    /// On hardware, ALET falls at the boundary between sub-phases (e.g.
    /// F->G), before the CPU's BUKE window opens. This method models
    /// just the DFF capture and its combinational consequences.
    ///
    /// Called from the executor between PPU rise and CPU bus read, so
    /// the CPU sees post-XYMU state. The remaining fall() work
    /// (fetcher, cascade, TYFA) runs later in the falling phase.
    pub(in crate::ppu) fn settle_alet(&mut self, xugu: bool) {
        // Capture rendering_active state before any clearing — fall() uses
        // this to gate mode3_falling() on the transition dot.
        self.rendering_before_settle = self.rendering_active;

        // WODU is combinational from PX and FEPO, valid here.
        let wodu_now = self.wodu(xugu);

        // VOGA will capture WODU on the upcoming fall().
        let voga_will_fire = self.voga || wodu_now;

        // WEGO = OR2(VID_RST, VOGA) clears XYMU. Apply early for CPU
        // STAT readback visibility.
        if voga_will_fire {
            self.rendering_active = false;
        }
    }

    /// VOGA (DFF17) captures WODU on PPU clock rise (gate: ALET rising);
    /// WEGO = OR2(VID_RST, VOGA) then clears the XYMU rendering latch.
    /// One unit of work — the Mode 3→0 trigger.
    ///
    /// The one-edge pipeline delay (spec §6.3) is the half-dot from the
    /// preceding PPU clock fall (where WODU goes true when PX=167 via
    /// CLKPIPE) to this PPU clock rise (where VOGA captures). WODU's
    /// inputs (PX, FEPO) settled during the preceding edge, so VOGA
    /// captures the current dot's WODU value.
    ///
    /// Returns wodu_current (live combinational value for STAT, LCD last_pixel).
    pub(in crate::ppu) fn capture_voga(&mut self, xugu: bool) -> bool {
        // WODU is purely combinational — no XYMU dependency, so always valid.
        let wodu_now = self.wodu(xugu);

        // VOGA DFF17 captures CURRENT dot's WODU. The one-edge delay
        // is from rise (WODU settles) to fall (VOGA captures).
        if wodu_now {
            self.voga = true;
        }

        // WEGO = OR2(VID_RST, VOGA) clears XYMU.
        if self.voga {
            self.rendering_active = false;
        }

        wodu_now
    }

    /// Latch FEPO for the next dot's wodu() evaluation. Called in
    /// mode3_falling after FEPO is evaluated but before it changes.
    pub(in crate::ppu) fn latch_fepo(&mut self, fepo: bool) {
        self.fepo = fepo;
    }

    /// AVAP: Mode 2→3 transition. Sets the rendering-mode latch (XYMU set).
    pub(in crate::ppu) fn begin_rendering(&mut self) {
        self.rendering_active = true;
    }

    /// Rendering-mode latch (XYMU). True during Mode 3.
    pub(in crate::ppu) fn rendering_active(&self) -> bool {
        self.rendering_active
    }

    /// Whether `rendering_active` was true before settle_alet() ran this dot.
    /// Used by Rendering::on_ppu_clock_rise() to gate mode3_falling() — on the dot
    /// VOGA fires, rendering_active is already cleared but the final
    /// mode3 falling work still needs to run.
    pub(in crate::ppu) fn rendering_before_settle(&self) -> bool {
        self.rendering_before_settle
    }

    pub(in crate::ppu) fn voga(&self) -> bool {
        self.voga
    }

    pub(in crate::ppu) fn reset(&mut self) {
        self.rendering_active = false;
        self.voga = false;
        self.fepo = false;
        self.rendering_before_settle = false;
    }
}
