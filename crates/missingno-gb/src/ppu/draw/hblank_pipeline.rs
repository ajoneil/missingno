//! Hblank pipeline — the Mode 3→0 termination path.
//!
//! Signal naming follows the project's PPU timing spec. Netlist gate
//! names (XYMU, VOGA, WEGO, TOFU, etc. from the dmgcpu netlist,
//! msinger/dmg-schematics) appear in doc comments for traceability.

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
    /// spec XYMU). NOR-latch async-set by WEGO, async-reset by AVAP.
    /// Clear is combinational — XYMU responds to WEGO rising without
    /// a clock.
    rendering_active: bool,
    /// HBLANK capture DFF. VOGA (dffr, captures WODU on ALET rising edge).
    ///
    /// Only clocked element in the Mode 3→0 chain. Feeds WEGO
    /// combinationally. Reset by TADY (line reset chain).
    voga: bool,
    /// VOGA's pending input: set on the master-clock fall when WODU
    /// first rises (sub-dot 2.515 of the LDH M-cycle for sprite-class
    /// scanlines, per spec §10.5.4). Committed to `voga` on the next
    /// master-clock rise (= next ALET rising, sub-dot ≈ 3.0) by
    /// `tick_voga_on_rise`, modelling the ~0.479-dot WODU→VOGA.q DFF
    /// capture delay. Replaces the prior model that collapsed WODU
    /// sample + VOGA capture + WEGO + XYMU clear onto a single rise.
    voga_pending: bool,
    /// Sprite X priority aggregate, latched at start of falling phase.
    /// FEPO = OR2(FOVE, FEFY).
    ///
    /// Collapses the 16-cell SACU-clocked DFFSR chain that carries
    /// per-sprite match state through the pixel pipe on hardware:
    /// the chain's sole consumer-visible effect is a 1-dot FEPO→WODU
    /// delay, modelled here by a single latch — `Rendering::fepo()`
    /// recomputes the combinational match; `latch_fepo()` captures
    /// it for the next dot's `wodu()`.
    fepo: bool,
    /// XYMU set (mode3↑) deferred by one emulator edge after AVAP-fall.
    /// Models the AJUJ-glitch window (spec §10.5.6): mode2 falls at AVAP-fall
    /// (BESU clear), mode3 rises at the next master-clock rise via
    /// `tick_pending_begin_rendering`. Between the two edges, mode2=0 AND
    /// mode3=0 → the OAM-write enable AJUJ is briefly high, allowing OAM
    /// writes whose CUPA strobe straddles the AVAP boundary to land.
    pending_begin_rendering: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            rendering_active: false,
            voga: false,
            voga_pending: false,
            fepo: false,
            pending_begin_rendering: false,
        }
    }

    /// Boot-ROM-handoff hblank state: VOGA persists
    /// from the prior scanline's Mode 3 WODU capture — it is a `dffr`
    /// only reset by TADY, which next fires at LX=113 (15 M-cycles after
    /// handoff). All other latches are at their power-on defaults
    /// (XYMU cleared, FEPO=0 with the sprite store empty).
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            rendering_active: false,
            voga: true,
            voga_pending: false,
            fepo: false,
            pending_begin_rendering: false,
        }
    }

    /// WODU: combinational hblank gate. Hardware chain:
    ///
    ///   WODU = AND2(XENA, XANO)
    ///   XENA = NOT(FEPO)   — "no sprite match"
    ///   XANO = NOT(XUGU)   — "PX at terminal count 167"
    ///   XUGU = NAND5(SYBE, SAVY, TUKY, XEHO, XODU)  — PX=167 decode
    ///
    /// Collapsed cascade: `FEPO, XUGU → XENA, XANO → WODU`.
    ///
    /// The emulator collapses the XENA and XANO inverters into their
    /// consumers. `!self.fepo` is the XENA term (inverted inline from
    /// the `fepo` field rather than storing XENA separately). The
    /// `xano` parameter is `PixelCounter::terminal()`'s
    /// positive-at-PX=167 output — which matches XANO's polarity
    /// (NOT of netlist XUGU's active-low NAND5).
    ///
    /// So `xano && !self.fepo` = XANO AND XENA = WODU, matching the
    /// AND2(XENA, XANO) netlist definition. WODU is purely
    /// combinational on hardware — it does not depend on XYMU. TARU
    /// (STAT mode 0) reads WODU directly.
    pub(in crate::ppu) fn wodu(&self, xano: bool) -> bool {
        xano && !self.fepo
    }

    /// WEGO: OR2(TOFU, VOGA). Combinational — no clock. Drives XYMU's
    /// NOR-latch set input.
    ///
    /// TOFU (video reset path; NOT(XAPO)) is not first-class in the
    /// emulator — VID_RST is handled via pipeline reset and scanner
    /// clears on LCD-off, so during active rendering WEGO reduces to
    /// VOGA.
    pub(in crate::ppu) fn wego(&self) -> bool {
        self.voga
    }

    /// Evaluate WODU on the master-clock fall and pend VOGA when WODU
    /// first rises. Called from `Rendering::on_ppu_clock_fall` after
    /// `PixelCounter::advance` updates XANO. WODU↑ in hardware is at
    /// sub-dot 2.515 of the LDH M-cycle for sprite-class scanlines
    /// (§10.5.4); ALET rising at sub-dot ≈ 3.0 then captures VOGA.q.
    /// In emulator edges that maps to: WODU↑ on fall of dot 2, VOGA.q↑
    /// on rise of dot 3.
    ///
    /// `voga_pending` is set on the first WODU rise per scanline only —
    /// once VOGA.q is set (or already pending), further fall-side WODU
    /// observations during HBlank are no-ops (WODU stays high during
    /// HBlank but VOGA is already captured).
    ///
    /// Returns the combinational WODU value at this fall edge.
    pub(in crate::ppu) fn evaluate_wodu_on_fall(&mut self, xano: bool) -> bool {
        let wodu_now = self.wodu(xano);
        if wodu_now && !self.voga && !self.voga_pending {
            self.voga_pending = true;
        }
        wodu_now
    }

    /// Commit any pending VOGA capture on the master-clock rise and fire
    /// the WEGO → XYMU.q clear cascade. Called from
    /// `Rendering::on_ppu_clock_rise`. The pending flag is set by the
    /// prior fall's `evaluate_wodu_on_fall`; ticking it here on the
    /// next rise reproduces the ~0.479-dot WODU↑ → VOGA.q↑ DFF capture
    /// delay (§7.2). WEGO fires combinationally from VOGA; XYMU's
    /// NOR-latch async-sets, clearing rendering_active on the same edge.
    ///
    /// Returns `true` iff VOGA.q just committed from pending on this
    /// rise — used by the LCD to push screen_x=159 (post-fall-shift
    /// shifter MSB) once per scanline.
    pub(in crate::ppu) fn tick_voga_on_rise(&mut self) -> bool {
        let was_pending = self.voga_pending;
        if self.voga_pending {
            self.voga = true;
            self.voga_pending = false;
        }
        if self.wego() {
            self.rendering_active = false;
        }
        was_pending
    }

    /// Latch FEPO for the next dot's wodu() evaluation. Called in
    /// mode3_falling after FEPO is evaluated but before it changes.
    pub(in crate::ppu) fn latch_fepo(&mut self, fepo: bool) {
        self.fepo = fepo;
    }

    /// AVAP: Mode 2→3 transition. Marks XYMU.q clear (mode3↑) as pending —
    /// it fires on the next master-clock rise via
    /// `tick_pending_begin_rendering`. Models the §10.5.6 AJUJ-glitch window:
    /// BESU.q clears at AVAP-fall (mode2↓), but mode3 net↑ is buffered to
    /// +2,655 ps after AVAP↑ in hardware. In the emulator's half-dot edge
    /// granularity, the deferral places mode3↑ on the next rise so the
    /// discretized window `mode2=0 AND mode3=0` is representable — the
    /// 2,100 ps AJUJ-high gap that gates OAM-write strobes during a
    /// Mode 2→3 straddle.
    pub(in crate::ppu) fn pend_begin_rendering(&mut self) {
        self.pending_begin_rendering = true;
    }

    /// Fire any pending begin_rendering at this master-clock rise. Mirrors
    /// hardware's mode3 net↑ at +2,655 ps after AVAP↑.
    pub(in crate::ppu) fn tick_pending_begin_rendering(&mut self) {
        if self.pending_begin_rendering {
            self.rendering_active = true;
            self.pending_begin_rendering = false;
        }
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
        self.voga_pending = false;
        self.fepo = false;
    }
}
