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
    /// CATU_LINE_ENDp DFF17: clocked by XUPY rising, D = ABOV_LINE_ENDp_old.
    /// On hardware, CATU captures RUTU_old — the line-end signal from the
    /// PREVIOUS XUPY cycle. Two-stage pipeline: boundary sets `rutu` →
    /// first XUPY rise shifts to `rutu_old` → second XUPY rise CATU fires.
    /// Total delay: 2 XUPY cycles (4 dots) from boundary to scan-start.
    catu: bool,
    /// RUTU signal: set true at the scanline boundary. Shifted to
    /// rutu_old on the next XUPY rising edge.
    rutu: bool,
    /// RUTU_old: CATU's D input. Set from rutu on XUPY rise, consumed
    /// by CATU on the following XUPY rise. This two-stage shift models
    /// GateBoy's `_old` evaluation — CATU reads the value RUTU had at
    /// the start of the previous XUPY cycle, not the current one.
    rutu_old: bool,
    /// BYBA_SCAN_DONEp_odd: DFF17 capturing scan_done() on XUPY rising edge.
    /// Because rise() runs before fall() (which ticks the counter), BYBA
    /// sees the counter state from the previous XUPY cycle — matching
    /// GateBoy's `reg_old.FETO.out_old()` without an explicit pipeline
    /// register. Single DFF delay = 2 dots.
    byba: bool,
    /// DOBA_SCAN_DONEp_evn: DFF17 capturing BYBA on falling edge.
    doba: bool,
    /// AVAP signal from the most recent rise(), consumed by fall()
    /// to gate scanning termination on the correct (falling) edge.
    last_avap: bool,
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
            catu: false,
            rutu: false,
            rutu_old: false,
            byba: false,
            doba: false,
            last_avap: false,
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

    /// Whether CATU is enabled (NOT first line after LCD-on).
    pub(super) fn catu_enabled(&self) -> bool {
        self.catu_enabled
    }

    /// Release VID_RST's blocking effect on CATU. Called after the first
    /// scanline completes (reset_scanline), enabling BESU on subsequent lines.
    pub(super) fn enable_catu(&mut self) {
        self.catu_enabled = true;
    }

    /// Current scan counter entry (0-39).
    pub(super) fn scan_counter_entry(&self) -> u8 {
        self.counter.entry()
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

    /// Read access to the sprite store for debug snapshots.
    pub(super) fn sprites_ref(&self) -> &SpriteStore {
        &self.sprites
    }

    /// Mutable access to the sprite store for X matching and marking fetched slots.
    pub(super) fn sprites_mut(&mut self) -> &mut SpriteStore {
        &mut self.sprites
    }

    /// Rising edge (DELTA_ODD): BYBA captures scan_done(), AVAP evaluated,
    /// and CATU scan-start fires.
    ///
    /// Hardware: BYBA_SCAN_DONEp_odd has _odd suffix → latches on rising edge.
    /// BYBA captures scan_done() directly. Because rise() runs before fall()
    /// (which ticks the counter), BYBA sees the counter state from the
    /// previous XUPY cycle — matching GateBoy's `reg_old.FETO.out_old()`.
    pub(super) fn rise(&mut self, xupy_rising: bool) -> ScanSignals {
        // BYBA: DFF capturing scan_done() on rising edge. rise() runs
        // before fall(), so the counter hasn't ticked yet — BYBA naturally
        // sees the previous XUPY cycle's value.
        if xupy_rising {
            self.byba = self.counter.scan_done();
        }

        // AVAP: combinational scan-done trigger.
        let avap = self.byba && !self.doba;

        // Store AVAP for fall() to consume. On hardware, BESU has _evn
        // suffix and clears via AVAP → EPOR on the falling edge, not
        // the rising edge. Deferring the clear to fall() ensures entry
        // 39's comparison still runs (scanning is true through fall()).
        self.last_avap = avap;

        // CATU_LINE_ENDp DFF17: clocked by XUPY rising edge.
        // D = ABOV_old = AND(RUTU_old, !y144_old). On each XUPY rise,
        // shift rutu → rutu_old, then CATU captures rutu_old. This
        // two-stage pipeline means CATU fires 2 XUPY cycles (4 dots)
        // after the scanline boundary, matching hardware phase_lx timing.
        if xupy_rising {
            // Shift: current rutu becomes rutu_old for CATU's D input.
            // Clear rutu after shift — it's a one-shot from the boundary.
            let was_rutu = self.rutu;
            self.rutu = false;
            self.catu = self.rutu_old;
            self.rutu_old = was_rutu;
        }

        // CATU output drives BESU (scanning latch) and counter reset.
        // Suppressed on LCD turn-on first line (catu_enabled is false).
        if self.catu && !self.scanning {
            self.scanning = true;
            if self.catu_enabled {
                self.besu = true;
            }
            self.counter.reset();
            self.catu = false;
        }

        ScanSignals { avap }
    }

    /// Falling edge (DELTA_EVEN): scanner tick, DOBA capture, and scanning
    /// termination on AVAP.
    ///
    /// Hardware: DOBA_SCAN_DONEp_evn has _evn suffix → latches on falling edge.
    /// FETO is sampled after tick_clock(), matching hardware's combinational gate.
    /// BESU clears on the falling edge via AVAP → EPOR.
    pub(super) fn fall(&mut self, xupy_rising: bool, ly: u8, regs: &PipelineRegisters, oam: &Oam) {
        // OAM comparison and sprite store population only happen during scanning.
        // Must run before tick_clock() — compare uses current entry, then clock advances.
        // Scanning is still true here even on the AVAP dot, because rise() no
        // longer clears it — matching hardware where BESU clears on the falling
        // edge via AVAP → EPOR → BESU.
        if self.scanning && xupy_rising {
            self.counter
                .compare_and_store(ly, &mut self.sprites, regs, oam);
        }

        if xupy_rising {
            self.counter.tick_clock();
        }

        // Clear scanning on AVAP (falling edge). On hardware, BESU has
        // _evn suffix and clears via AVAP → EPOR on the falling edge.
        if self.last_avap && self.scanning {
            self.scanning = false;
            self.besu = false;
        }

        // DOBA_SCAN_DONEp_evn: captures BYBA on falling edge.
        self.doba = self.byba;
    }

    /// Reset at scanline boundary. Sets rutu = true so the CATU DFF
    /// pipeline will fire after 2 XUPY cycles (4 dots), matching
    /// hardware's RUTU → RUTU_old → CATU propagation.
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
        self.last_avap = false;
        self.catu = false;
        self.rutu_old = false;
        // RUTU fires at the scanline boundary. It will shift to rutu_old
        // on the first XUPY rise, then CATU captures it on the second.
        self.rutu = true;
    }
}
