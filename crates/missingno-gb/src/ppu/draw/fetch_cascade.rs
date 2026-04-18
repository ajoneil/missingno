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

    /// Advance the alet-rising-clocked stages: NYKA captures LYRY, PYGO
    /// captures PORY, POKY NOR latch fires from PYGO.
    ///
    /// LYRY fires on the preceding alet-falling edge (fetcher counter reaches
    /// 5 in advance_rising). NYKA captures live LYRY here — the half-phase
    /// separation provides the 1 half-phase DFF delay.
    pub(in crate::ppu) fn advance_cascade(&mut self, lyry: bool) {
        // NYKA DFF17: captures live LYRY on falling edge (ALET clock).
        if lyry && !self.nyka {
            self.nyka = true;
        }

        // PYGO captures PORY on falling edge (ALET clock).
        if self.pory && !self.pygo {
            self.pygo = true;
        }

        // POKY NOR latch fires on falling, reading the just-updated PYGO.
        if self.pygo && !self.poky {
            self.poky = true;
        }
    }

    /// PORY captures NYKA on myvo rising (= alet falling = master-clock rising).
    pub(in crate::ppu) fn capture_pory(&mut self) {
        if self.nyka && !self.pory {
            self.pory = true;
        }
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
