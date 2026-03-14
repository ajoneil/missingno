use crate::game_boy::ppu::{PipelineRegisters, memory::Oam};

use super::oam_scan::{ScanCounter, SpriteStore};

/// Sprite scanner — owns all OAM scan (Mode 2) state.
///
/// Encapsulates the scan counter, scanning latch (BESU), BYBA/DOBA
/// scan-done pipeline, and the sprite store that bridges Mode 2 and
/// Mode 3. Communicates with the rest of the pipeline through explicit
/// signals: AVAP (scan complete) triggers the Mode 2→3 transition,
/// and the populated `SpriteStore` is consumed by the X matchers.
pub(super) struct SpriteScanner {
    /// 6-bit scan counter + Y comparator (YFEL-FONY).
    counter: ScanCounter,
    /// BESU scanning latch. Set when OAM scan starts, cleared by AVAP.
    scanning: bool,
    /// BYBA_SCAN_DONEp_odd: captures FETO (scan_done) on XUPY rising edges.
    byba: bool,
    /// DOBA_SCAN_DONEp_evn: captures BYBA on every rising edge.
    doba: bool,
    /// Ten-entry sprite register file (page 30). Populated during Mode 2,
    /// consumed by X matchers during Mode 3.
    sprites: SpriteStore,
}

/// Signals produced by `SpriteScanner::fall()` for the rest of the pipeline.
pub(super) struct ScanSignals {
    /// CATU fired this dot — scan just started (Mode 2 entry).
    pub(super) scan_started: bool,
    /// AVAP fired this dot — scan complete (Mode 2 → 3 transition).
    pub(super) avap: bool,
}

impl SpriteScanner {
    pub(super) fn new() -> Self {
        Self {
            counter: ScanCounter::new(),
            scanning: false,
            byba: false,
            doba: false,
            sprites: SpriteStore::new(),
        }
    }

    /// Whether the scanner is currently active (BESU/ACYL).
    pub(super) fn scanning(&self) -> bool {
        self.scanning
    }

    /// BYBA state, for debug snapshot.
    pub(super) fn byba(&self) -> bool {
        self.byba
    }

    /// DOBA state, for debug snapshot.
    pub(super) fn doba(&self) -> bool {
        self.doba
    }

    /// The OAM address the scanner is currently driving, if scanning.
    pub(super) fn oam_address(&self) -> Option<u8> {
        if self.scanning {
            Some(self.counter.oam_address())
        } else {
            None
        }
    }

    /// Mutable access to the sprite store for X matching and marking fetched slots.
    pub(super) fn sprites_mut(&mut self) -> &mut SpriteStore {
        &mut self.sprites
    }

    /// Rising edge: DOBA captures BYBA.
    pub(super) fn rise(&mut self) {
        self.doba = self.byba;
    }

    /// Falling edge: scanner tick, CATU scan-start, BYBA capture, AVAP check.
    ///
    /// Takes explicit inputs from the video control and pipeline state.
    /// Returns `ScanSignals` indicating whether AVAP fired.
    pub(super) fn fall(
        &mut self,
        xupy_rising: bool,
        lx: u8,
        wuvu: bool,
        ly: u8,
        lcd_turning_on: bool,
        regs: &PipelineRegisters,
        oam: &Oam,
    ) -> ScanSignals {
        // Capture FETO *before* the scanner tick — models DFF pre-edge capture.
        let feto_old = self.counter.scan_done();

        if self.scanning && xupy_rising {
            self.counter.tick(ly, &mut self.sprites, regs, oam);
        }

        // CATU_LINE_ENDp: at dot 1, CATU fires, setting BESU and resetting
        // the scan counter. Suppressed on LCD turn-on first line.
        let scan_started = lx == 0 && wuvu && !lcd_turning_on && !self.scanning;
        if scan_started {
            self.scanning = true;
            self.counter.reset();
        }

        // BYBA_SCAN_DONEp_odd: capture pre-tick FETO on XUPY rising edge.
        if xupy_rising {
            self.byba = feto_old;
        }

        // AVAP: combinational scan-done trigger.
        let avap = self.byba && !self.doba;

        if avap && self.scanning && !lcd_turning_on {
            self.scanning = false;
        }

        ScanSignals { scan_started, avap }
    }

    /// Reset at scanline boundary.
    pub(super) fn reset(&mut self) {
        self.counter.reset();
        self.scanning = false;
        self.sprites = SpriteStore::new();
        // BYBA/DOBA are not explicitly reset at line boundaries on hardware —
        // they naturally clear because FETO is false after counter reset.
        // But we reset them for cleanliness.
        self.byba = false;
        self.doba = false;
    }
}
