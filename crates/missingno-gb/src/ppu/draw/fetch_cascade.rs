//! Fetch-done cascade module.
//!
//! DFF chain that propagates the fetcher-idle signal through alet/myvo
//! pipeline stages. NYKA and PYGO are alet-clocked (capture on alet
//! rising = master-clock falling); PORY is myvo-clocked (captures on
//! myvo rising = alet falling = master-clock rising).

/// Fetch-done cascade: LYRY → NYKA → PORY → PYGO → POKY.
///
/// A DFF chain that propagates the fetcher-idle signal (LYRY) through four
/// stages, adding pipeline delay before the pixel clock enables. Not a
/// processing block — just a small state machine you clock and query.
///
/// - NYKA (DFF17, falling/ALET): captures LYRY
/// - PORY (DFF17, rising/MYVO): captures NYKA
/// - PYGO (DFF17, falling/ALET): captures PORY
/// - POKY (NOR latch, falling): fires from PYGO
///
/// LYRY fires on the rising edge (the fetcher counter reaches 5 during
/// advance_rising). NYKA captures on the next falling edge — the natural
/// 1 half-phase DFF delay. No extra storage is needed because LYRY is
/// combinational on fetch_counter, which persists between half-phases.
///
/// Consumers read DFF state via accessors:
/// - `poky()` → TYFA (pixel clock enable)
/// - `pygo()` → sprite wait exit, TAVE guard, window trigger gate
/// - `pory()` → RYDY clear
/// - `nyka()` + `pory()` → TAVE preload
///
/// # §6.1 collapsed cascade signals (not modelled as explicit state here)
///
/// The spec's §6.1 BG fetch counter subsystem includes several downstream
/// signals feeding TEVO (the NYXU reset trigger for the per-tile fetch
/// boundary). This module models NYKA / PORY / PYGO / POKY directly; the
/// following §6.1 signals are collapsed or behaviourally derived elsewhere:
///
/// - Drain-detector path: `PANY → RYFA → RENE → SEKO → TEVO`.
///   RYFA (dffr, SEGU-clocked) captures PANY = NOR2(ROZE, wxy_match).
///   RENE (dffr, ALET-clocked) captures RYFA. SEKO = NOR2(RENE, RYFA)
///   goes high when both drain to 0 (the spec's 8-dot tile-cycle 2-dot
///   hold period, per §6.1 "SEKO drain-detector waveform"). Emulator
///   fires SEKO behaviourally via the `fine_scroll.count == 7`
///   condition in `rendering.rs::mode3_pixel_pipeline` — fires on the
///   dot that completes an 8-pixel shift cycle, which matches the
///   dot at which hardware's RENE/RYFA drain detector would fire
///   (both trigger at each tile boundary during steady-state
///   rendering, and both freeze in lockstep during SACU-suppressed
///   sprite-fetch windows). Observation-equivalent at the
///   TEVO→NYXU→load-into consumer boundary.
/// - Window-trigger path: `TUXY → SUZU → TEVO`. TUXY = NAND2(SOVY, SYLO)
///   is a RYDY falling-edge detector; SUZU = NOT(TUXY) fires one half-
///   cycle per RYDY 1→0 transition. Emulator fires the SUZU path on
///   PORY-driven RYDY clear in `window_control.rs` / `rendering.rs`.
/// - Startup / window-restart path: `ROMO → SUVU → TAVE → TEVO`.
///   ROMO = NOT(POKY); SUVU = NAND4(NYKA, PORY, ROMO, XYMU); TAVE =
///   NOT(SUVU). Emulator's TAVE one-shot in `rendering.rs` fires from
///   `NYKA && PORY && !PYGO` — `!PYGO` substitutes for `ROMO = !POKY`
///   using PYGO as the POKY precursor during the startup window.
/// - Counter-bit sample: `LAXU → LYZU → MYSO → NYDY` (§6.6 temp-latch
///   enable). LYZU (dffr, ALET-clocked) samples LAXU; its only consumer
///   is `MYSO = NOR3(mode3_n, LAXE, LYZU)` where `LAXE = NOT(LAXU)`,
///   making MYSO a LAXU rising-edge detector (fires for the half-dot
///   window between LAXU↑ and LYZU catching up). MYSO feeds §6.6's
///   `NYDY = NAND3(NOFU, MESU, MYSO)` temp-latch enable decode,
///   gating the LUNA/LOMA pulse that captures the low-byte VRAM
///   read at fetch counter = 3. Emulator collapses the temp-latch
///   chain by reading VRAM directly into `fetcher.tile_data_low` /
///   `tile_data_high` at counter 2 / 4 falling edges — observation-
///   equivalent at the parallel-load consumer boundary for the
///   clean-ROM case (mid-fetch LCDC writes fall under §6.14's
///   already-flagged ambiguity territory).
///
/// # Honest-abstraction synthesis (§6.1 emulator-alignment arc, 2026-04-21)
///
/// All four collapsed paths above were verified observation-equivalent
/// at their named consumer boundaries during the §6.1 emulator-alignment
/// step. The collapsed-vs-modelled split preserves hardware fidelity at
/// the observable boundaries (TEVO firing dot, BG shifter parallel-load
/// content, window-trigger timing) while abstracting internals that
/// would add state without adding observable behaviour. The LYRY → NYKA
/// → PORY → PYGO → POKY chain is modelled directly because its DFFs
/// gate the pixel-clock (POKY → TYFA) and the window check (PORY → RYDY
/// clear) — consumers that read the pipeline's intermediate state on
/// specific edges. The collapsed chains terminate at combinational
/// outputs whose boundary-observable value is reproduced exactly by
/// the behavioural conditions in `rendering.rs` / `window_control.rs`.
pub(in crate::ppu) struct FetchCascade {
    /// NYKA: DFF17, clocked by alet (captures on master-clock fall).
    nyka: bool,
    /// PORY: DFF17, clocked by myvo (captures on master-clock rise).
    pory: bool,
    /// PYGO: DFF17, clocked by alet (captures on master-clock fall).
    pygo: bool,
    /// POKY NOR latch: fires from PYGO on falling edge.
    poky: bool,
}

impl FetchCascade {
    pub(in crate::ppu) fn new() -> Self {
        FetchCascade {
            nyka: false,
            pory: false,
            pygo: false,
            poky: false,
        }
    }

    /// Advance the ALET-rising-clocked stages (PPU-clock-rise phase):
    /// NYKA captures LYRY, PYGO captures PORY, POKY NOR latch settles.
    ///
    /// NYKA and PYGO are dffr — true edge-triggered DFFs that track both
    /// 0→1 and 1→0 transitions of their D inputs each ALET edge. POKY is
    /// a NOR-latch (S=PYGO, R=mode3_n): set when PYGO=1, hold when both
    /// inputs are 0 (= during Mode 3 with PYGO=0). Reset (R=1) outside
    /// Mode 3 is handled by `reset()` at the scanline boundary.
    pub(in crate::ppu) fn advance_cascade(&mut self, lyry: bool) {
        // NYKA dffr: captures LYRY on ALET rise.
        self.nyka = lyry;

        // PYGO dffr: captures PORY on ALET rise.
        self.pygo = self.pory;

        // POKY nor_latch: S=PYGO sets the latch; S=R=0 holds.
        if self.pygo {
            self.poky = true;
        }
    }

    /// PORY captures NYKA on MYVO rising (= PPU clock fall, master-clock rise).
    pub(in crate::ppu) fn capture_pory(&mut self) {
        // PORY dffr: captures NYKA on MYVO rise.
        self.pory = self.nyka;
    }

    /// Scanline reset: clear all DFFs.
    pub(in crate::ppu) fn reset(&mut self) {
        self.nyka = false;
        self.pory = false;
        self.pygo = false;
        self.poky = false;
    }

    /// NAFY window-trigger reset: clear NYKA and PORY.
    /// PYGO and POKY are not reset by window triggers.
    pub(in crate::ppu) fn reset_window(&mut self) {
        self.nyka = false;
        self.pory = false;
    }

    pub(in crate::ppu) fn nyka(&self) -> bool {
        self.nyka
    }
    pub(in crate::ppu) fn pory(&self) -> bool {
        self.pory
    }
    pub(in crate::ppu) fn pygo(&self) -> bool {
        self.pygo
    }
    pub(in crate::ppu) fn poky(&self) -> bool {
        self.poky
    }
}
