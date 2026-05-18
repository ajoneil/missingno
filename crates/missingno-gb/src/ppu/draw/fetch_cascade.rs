//! LYRY → NYKA → PORY → PYGO → POKY DFF chain propagating fetcher-idle through pipeline stages.
//!
//! NYKA/PYGO are ALET-clocked (PPU rise); PORY is MYVO-clocked (PPU fall); POKY is a NOR-latch fed by PYGO.
//!
//! Consumers: poky() → TYFA pixel-clock; pygo() → sprite wait exit + window trigger; pory() → RYDY clear;
//! nyka()+pory() → TAVE preload.
//!
//! Downstream TEVO feeds (PANY drain-detector, SUZU window-restart, TAVE startup, temp-latch
//! enable) are collapsed and fired behaviourally from `rendering.rs` / `window_control.rs`;
//! observation-equivalent at the TEVO→NYXU→load-into consumer boundary.
pub(in crate::ppu) struct FetchCascade {
    /// ALET-clocked DFF.
    nyka: bool,
    /// MYVO-clocked DFF.
    pory: bool,
    /// ALET-clocked DFF.
    pygo: bool,
    /// NOR-latch: S=PYGO, R=LOBY=NOT(mode3).
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

    /// ALET rising: NYKA captures LYRY, PYGO captures PORY, POKY settles. POKY's R input is asserted outside Mode 3 (handled by `reset()`).
    pub(in crate::ppu) fn advance_cascade(&mut self, lyry: bool) {
        self.nyka = lyry;
        self.pygo = self.pory;
        if self.pygo {
            self.poky = true;
        }
    }

    /// MYVO rising: PORY captures NYKA.
    pub(in crate::ppu) fn capture_pory(&mut self) {
        self.pory = self.nyka;
    }

    /// Mode 3 exit reset (XYMU↑). Also called defensively at scanline reset.
    pub(in crate::ppu) fn reset(&mut self) {
        self.nyka = false;
        self.pory = false;
        self.pygo = false;
        self.poky = false;
    }

    /// NAFY window-trigger reset clears NYKA and PORY only.
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
