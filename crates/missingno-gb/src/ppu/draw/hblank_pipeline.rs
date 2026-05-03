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
    /// STAT-readout mirror of `rendering_active`. Captured at the start
    /// of each PPU-clock-fall from the pre-fall internal value, so
    /// `Ppu::mode()` sees the pre-transition value across the AVAP
    /// integer dot — matching GateBoy's gbtrace adapter sampling phase.
    /// Not a hardware DFF; this models the CPU's T-cycle STAT sampling
    /// window, which observes XYMU before AVAP↑ sets it within the
    /// AVAP integer dot. Only read by `Ppu::mode()` via
    /// `rendering_active_stat()`; all pipeline consumers (was_rendering
    /// snapshot, VRAM/OAM lock) continue to read internal
    /// `rendering_active`.
    rendering_active_stat: bool,
    /// HBLANK capture DFF. VOGA (dffr, captures WODU on ALET rising edge).
    ///
    /// Only clocked element in the Mode 3→0 chain. Feeds WEGO
    /// combinationally. Reset by TADY (line reset chain).
    voga: bool,
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
}

impl HblankPipeline {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            rendering_active: false,
            rendering_active_stat: false,
            voga: false,
            fepo: false,
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
            rendering_active_stat: false,
            voga: true,
            fepo: false,
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

    /// VOGA (DFF17) captures WODU on PPU clock rise (ALET rising).
    /// WEGO = OR2(TOFU, VOGA) then fires combinationally; XYMU's
    /// NOR-latch async-sets, clearing rendering_active within the same
    /// master-clock edge — matching the chain's hardware structure
    /// where only VOGA is clocked, WEGO and XYMU's set path being
    /// combinational.
    ///
    /// Returns wodu_current (live combinational value for STAT, LCD last_pixel).
    pub(in crate::ppu) fn capture_voga(&mut self, xano: bool) -> bool {
        // WODU is purely combinational — no XYMU dependency, so always valid.
        let wodu_now = self.wodu(xano);

        // VOGA DFF captures CURRENT dot's WODU on this ALET rising edge.
        if wodu_now {
            self.voga = true;
        }

        // WEGO fires combinationally from VOGA; XYMU NOR-latch
        // async-sets, clearing the rendering latch.
        if self.wego() {
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

    /// STAT-readout mirror of XYMU (see `rendering_active_stat` field).
    /// Lags the internal `rendering_active` by one PPU-clock-fall. Read
    /// only by `Ppu::mode()` for the T-cycle STAT sampling window.
    pub(in crate::ppu) fn rendering_active_stat(&self) -> bool {
        self.rendering_active_stat
    }

    /// Capture the pre-fall `rendering_active` into the STAT-readout
    /// mirror. Called at the start of every PPU-clock-fall, before any
    /// writer touches `self.rendering_active` in that fall.
    pub(in crate::ppu) fn capture_rendering_stat(&mut self) {
        self.rendering_active_stat = self.rendering_active;
    }

    pub(in crate::ppu) fn voga(&self) -> bool {
        self.voga
    }

    pub(in crate::ppu) fn reset(&mut self) {
        self.rendering_active = false;
        self.rendering_active_stat = false;
        self.voga = false;
        self.fepo = false;
    }
}
