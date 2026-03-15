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
    /// Scan machinery active — set on all lines including LCD-on line 0.
    /// Controls counter ticking, OAM comparisons, and mode 3 gating in fall().
    scanning: bool,
    /// BESU scanning latch — drives ACYL for STAT mode bits and OAM bus locking.
    /// Set by CATU only when catu_enabled is true (NOT set on LCD-on line 0).
    besu: bool,
    /// Models NOT(VID_RST) for CATU gating. Starts false at LCD-on (VID_RST
    /// blocks CATU). Set to true by enable_catu() after the first scanline
    /// completes. Persists across scanline resets.
    catu_enabled: bool,
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
    /// AVAP fired this dot — scan complete (Mode 2 → 3 transition).
    pub(super) avap: bool,
}

impl SpriteScanner {
    pub(super) fn new() -> Self {
        Self {
            counter: ScanCounter::new(),
            scanning: false,
            besu: false,
            catu_enabled: false,
            byba: false,
            doba: false,
            sprites: SpriteStore::new(),
        }
    }

    /// Set scanning active for LCD-on initialization. On hardware, VID_RST
    /// deassertion releases the scan counter and comparison logic
    /// simultaneously — there is no separate CATU "start scanning" event
    /// on the first line. The counter is already at 0 from async reset.
    pub(super) fn start_scanning(&mut self) {
        self.scanning = true;
    }

    /// Whether the scan machinery is currently active.
    pub(super) fn scanning(&self) -> bool {
        self.scanning
    }

    /// BESU scanning latch — drives ACYL for STAT mode and OAM bus locking.
    pub(super) fn besu(&self) -> bool {
        self.besu
    }

    /// Release VID_RST's blocking effect on CATU. Called after the first
    /// scanline completes (reset_scanline), enabling BESU on subsequent lines.
    pub(super) fn enable_catu(&mut self) {
        self.catu_enabled = true;
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
        regs: &PipelineRegisters,
        oam: &Oam,
    ) -> ScanSignals {
        // Capture FETO *before* the scanner tick — models DFF pre-edge capture.
        let feto_old = self.counter.scan_done();

        // OAM comparison and sprite store population only happen during scanning.
        // Must run before tick_clock() — compare uses current entry, then clock advances.
        if self.scanning && xupy_rising {
            self.counter
                .compare_and_store(ly, &mut self.sprites, regs, oam);
        }

        // Counter ticks on XUPY regardless of scanning state. On hardware,
        // the counter clock is XUPY gated by !VID_RST, not by BESU.
        if xupy_rising {
            self.counter.tick_clock();
        }

        // CATU_LINE_ENDp: at dot 1, CATU fires, setting BESU and resetting
        // the scan counter. Suppressed on LCD turn-on first line.
        let scan_started = lx == 0 && wuvu && !self.scanning;
        if scan_started {
            self.scanning = true;
            if self.catu_enabled {
                self.besu = true;
            }
            self.counter.reset();
        }

        // BYBA_SCAN_DONEp_odd: capture pre-tick FETO on XUPY rising edge.
        if xupy_rising {
            self.byba = feto_old;
        }

        // AVAP: combinational scan-done trigger.
        let avap = self.byba && !self.doba;

        if avap && self.scanning {
            self.scanning = false;
            self.besu = false;
        }

        ScanSignals { avap }
    }

    /// Reset at scanline boundary.
    pub(super) fn reset(&mut self) {
        self.counter.reset();
        self.scanning = false;
        self.besu = false;
        self.sprites = SpriteStore::new();
        // BYBA/DOBA are not explicitly reset at line boundaries on hardware —
        // they naturally clear because FETO is false after counter reset.
        // But we reset them for cleanliness.
        self.byba = false;
        self.doba = false;
    }
}
