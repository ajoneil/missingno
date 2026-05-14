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
    /// AJUJ permit pulse (NOR3 gate, spec §4.9.1 / §10.5.6). High for the
    /// ~2,100 ps window between BESU.q↓ and the buffered mode3 net↑ during
    /// the AVAP cascade. In the emulator's half-dot edge granularity this
    /// pulse spans one emulator edge: asserted at AVAP-fall together with
    /// mode3↑, deasserted at the next master-clock rise.
    ///
    /// Modelled as an explicit first-class signal — rather than derived
    /// from `!besu && !rendering_active` — so that mode3↑ can fire on
    /// AVAP-fall (the spec-correct edge per §7.1) without collapsing the
    /// AJUJ window. Consumed by `oam_write_locked` / `vram_write_locked`
    /// as a write-permit override.
    ajuj_pulse: bool,
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            rendering_active: false,
            voga: false,
            voga_pending: false,
            ajuj_pulse: false,
        }
    }

    /// Boot-ROM-handoff hblank state: VOGA persists
    /// from the prior scanline's Mode 3 WODU capture — it is a `dffr`
    /// only reset by TADY, which next fires at LX=113 (15 M-cycles after
    /// handoff). All other latches are at their power-on defaults
    /// (XYMU cleared; FEPO is combinational on the sprite store, which
    /// is empty at handoff).
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            rendering_active: false,
            voga: true,
            voga_pending: false,
            ajuj_pulse: false,
        }
    }

    /// WODU: combinational hblank gate. Hardware chain (spec §8.2):
    ///
    ///   WODU = AND2(XENA, XANO)
    ///   XENA = NOT(FEPO)   — "no sprite match"
    ///   XANO = NOT(XUGU)   — "PX at terminal count 167"
    ///   XUGU = NAND5(SYBE, SAVY, TUKY, XEHO, XODU)  — PX=167 decode
    ///
    /// Zero registered cells between FEPO and WODU on hardware — caller
    /// passes the combinational FEPO value computed from the current
    /// pixel_counter and scan store.
    pub(in crate::ppu) fn wodu(xano: bool, fepo: bool) -> bool {
        xano && !fepo
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
    /// `fepo` is the combinational sprite-match aggregate at this fall
    /// edge, computed by the caller from the current scan store and
    /// pixel_counter. Hardware FEPO→WODU is purely combinational
    /// (spec §8.2), so the value passed here must reflect the post-
    /// advance pixel_counter — not a value latched from a prior dot.
    ///
    /// Returns the combinational WODU value at this fall edge.
    pub(in crate::ppu) fn evaluate_wodu_on_fall(&mut self, xano: bool, fepo: bool) -> bool {
        let wodu_now = Self::wodu(xano, fepo);
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

    /// AVAP: Mode 2→3 transition. Sets XYMU.q clear (= rendering_active
    /// true) at the AVAP-fall edge per spec §7.1, and asserts the AJUJ
    /// permit pulse for the upcoming write-lock samples. The pulse
    /// represents the 2,100 ps window between BESU.q↓ and the buffered
    /// mode3 net↑ — both events that hardware sees within the same dot
    /// near AVAP↑, collapsed in our half-dot edge resolution to a
    /// single-edge transient signal cleared on the next master-clock rise.
    pub(in crate::ppu) fn pulse_ajuj_on_avap_fall(&mut self) {
        self.rendering_active = true;
        self.ajuj_pulse = true;
    }

    /// Clear the AJUJ permit pulse at this master-clock rise. Closes the
    /// 2,100 ps write-permit window opened on the prior AVAP-fall.
    pub(in crate::ppu) fn tick_ajuj_pulse_on_rise(&mut self) {
        self.ajuj_pulse = false;
    }

    /// AJUJ permit pulse (spec §10.5.6). True during the single emulator
    /// edge between AVAP-fall and the next master-clock rise. Consumers:
    /// `oam_write_locked` / `vram_write_locked` use it as a write-permit
    /// override.
    pub(in crate::ppu) fn ajuj_pulse(&self) -> bool {
        self.ajuj_pulse
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
        self.ajuj_pulse = false;
    }
}
