//! LYRY → NYKA → PORY → PYGO → POKY DFF chain propagating fetcher-idle through pipeline stages.
//!
//! NYKA/PYGO are ALET-clocked (PPU rise); PORY is MYVO-clocked (PPU fall); POKY is a NOR-latch fed by PYGO.
//!
//! Consumers: POKY → TYFA pixel-clock; PYGO → sprite wait exit + window trigger;
//! PORY → RYDY clear; NYKA+PORY → TAVE preload.
//!
//! Downstream TEVO feeds (PANY drain-detector, SUZU window-restart, TAVE startup, temp-latch
//! enable) are collapsed and fired behaviourally from `rendering.rs` / `window_control.rs`;
//! observation-equivalent at the TEVO→NYXU→load-into consumer boundary.
pub(in crate::ppu) struct FetchCascade {
    /// NYKA, ALET-clocked DFF.
    fetch_complete: bool,
    /// PORY, MYVO-clocked DFF.
    fetch_complete_stage_2: bool,
    /// PYGO, ALET-clocked DFF.
    fetch_complete_stage_3: bool,
    /// POKY NOR-latch: S=PYGO, R=LOBY=NOT(mode3).
    pixel_data_ready: bool,
}

impl FetchCascade {
    pub(in crate::ppu) fn new() -> Self {
        FetchCascade {
            fetch_complete: false,
            fetch_complete_stage_2: false,
            fetch_complete_stage_3: false,
            pixel_data_ready: false,
        }
    }

    /// ALET rising: NYKA captures LYRY, PYGO captures PORY, POKY settles. POKY's R input is asserted outside Mode 3 (handled by `reset()`).
    pub(in crate::ppu) fn advance_cascade(&mut self, bg_fetch_done: bool) {
        self.fetch_complete = bg_fetch_done;
        self.fetch_complete_stage_3 = self.fetch_complete_stage_2;
        if self.fetch_complete_stage_3 {
            self.pixel_data_ready = true;
        }
    }

    /// MYVO rising: PORY captures NYKA.
    pub(in crate::ppu) fn capture_fetch_complete_stage_2(&mut self) {
        self.fetch_complete_stage_2 = self.fetch_complete;
    }

    /// Mode 3 exit reset (XYMU↑). Also called defensively at scanline reset.
    pub(in crate::ppu) fn reset(&mut self) {
        self.fetch_complete = false;
        self.fetch_complete_stage_2 = false;
        self.fetch_complete_stage_3 = false;
        self.pixel_data_ready = false;
    }

    /// NAFY window-trigger reset clears NYKA and PORY only.
    pub(in crate::ppu) fn reset_window(&mut self) {
        self.fetch_complete = false;
        self.fetch_complete_stage_2 = false;
    }

    /// NYKA.
    pub(in crate::ppu) fn fetch_complete(&self) -> bool {
        self.fetch_complete
    }
    /// PORY.
    pub(in crate::ppu) fn fetch_complete_stage_2(&self) -> bool {
        self.fetch_complete_stage_2
    }
    /// PYGO.
    pub(in crate::ppu) fn fetch_complete_stage_3(&self) -> bool {
        self.fetch_complete_stage_3
    }
    /// POKY.
    pub(in crate::ppu) fn pixel_data_ready(&self) -> bool {
        self.pixel_data_ready
    }
}
