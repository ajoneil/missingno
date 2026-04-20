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
    /// WODU becomes true in the PPU-clock-fall phase (rise() when PX
    /// advances to 167); VOGA captures at the following PPU-clock-rise
    /// (fall(), ALET rising).
    voga: bool,
    /// WEGO pipeline stage. Captures VOGA on the master-clock edge
    /// following VOGA's own capture, driving XYMU's set input on the
    /// next rise(). The two-edge separation between VOGA capture and
    /// XYMU clear models the hardware's sub-dot WODU→XYMU propagation
    /// in the emulator's integer-dot representation, keeping Mode 3
    /// observable to the CPU on the transition dot's fall().
    wego: bool,
    /// Sprite X priority aggregate, latched at start of falling phase.
    /// FEPO (or2 of FOVE, FEFY).
    ///
    /// Feeds `hblank_condition()` and TYFA computation. Latched because
    /// FEPO changes mid-fall but WODU needs the value from the start
    /// of the falling phase.
    fepo: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            rendering_active: false,
            voga: false,
            wego: false,
            fepo: false,
        }
    }

    /// WODU: combinational hblank gate. AND2(XUGU, !FEPO).
    /// On hardware, WODU is purely combinational — it does not
    /// depend on XYMU. TARU (STAT mode 0) reads WODU directly.
    pub(in crate::ppu) fn wodu(&self, xugu: bool) -> bool {
        xugu && !self.fepo
    }

    /// VOGA (DFF17) captures WODU on PPU clock rise (gate: ALET rising).
    /// Does NOT clear XYMU directly — the WEGO pipeline stage defers
    /// XYMU clear by one master-clock edge.
    ///
    /// The first edge of the chain is the half-dot from the preceding
    /// PPU clock fall (WODU becomes true when PX=167 via CLKPIPE) to
    /// this PPU clock rise (VOGA captures).
    ///
    /// Returns wodu_current (live combinational value for STAT, LCD last_pixel).
    pub(in crate::ppu) fn capture_voga(&mut self, xugu: bool) -> bool {
        // WODU is purely combinational — no XYMU dependency, so always valid.
        let wodu_now = self.wodu(xugu);

        // VOGA DFF17 captures CURRENT dot's WODU.
        if wodu_now {
            self.voga = true;
        }

        wodu_now
    }

    /// WEGO pipeline stage: captures VOGA and drives XYMU set input.
    ///
    /// WEGO captures VOGA at the master-clock edge after VOGA captures
    /// (= next rise(), the PPU-clock-fall / ALET-falling edge). On
    /// that edge, XYMU set asserts and rendering_active clears.
    ///
    /// Called at the start of Rendering::on_ppu_clock_fall (executor's
    /// rise() edge) so downstream pixel-output-phase work sees the
    /// post-clear XYMU state.
    pub(in crate::ppu) fn propagate_wego(&mut self) {
        // WEGO DFF captures VOGA's current output.
        self.wego = self.voga;

        // WEGO=1 drives XYMU set input (NOR latch async set).
        if self.wego {
            self.rendering_active = false;
        }
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

    pub(in crate::ppu) fn voga(&self) -> bool {
        self.voga
    }

    pub(in crate::ppu) fn reset(&mut self) {
        self.rendering_active = false;
        self.voga = false;
        self.wego = false;
        self.fepo = false;
    }
}
