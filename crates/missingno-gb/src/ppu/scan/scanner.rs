use crate::ppu::{PipelineRegisters, memory::Oam};

use super::oam_scan::{ScanCounter, SpriteStore};

/// Sprite scanner — owns all OAM scan (Mode 2) state.
///
/// Encapsulates the scan counter, scanning latch (BESU), BYBA/DOBA
/// scan-done pipeline, and the sprite store that bridges Mode 2 and
/// Mode 3. Communicates with the rest of the pipeline through explicit
/// signals: AVAP (scan complete) triggers the Mode 2→3 transition,
/// and the populated `SpriteStore` is consumed by the X matchers.
pub(in crate::ppu) struct SpriteScanner {
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
    /// CATU_LINE_ENDp DFF17: clocked by XUPY rising, D = ABOV_LINE_ENDp.
    /// Single-stage: boundary sets `rutu` → next XUPY rise CATU fires.
    catu: bool,
    /// RUTU signal: set true at the scanline boundary. Consumed by
    /// tick_catu on the next XUPY rising edge.
    rutu: bool,
    /// BYBA: DFF17 clocked by XUPY (captures in fall).
    byba: bool,
    /// DOBA: DFF17 clocked by alet (captures in fall).
    doba: bool,
    /// AVAP result from fall(), consumed by rise(). On hardware AVAP
    /// is combinational (valid as soon as BYBA/DOBA settle in fall()),
    /// but the rendering pipeline reacts in rise().
    avap_pending: bool,
    /// ANEL propagation delay: suppresses the first scan tick after CATU
    /// starts scanning. On hardware, CATU.Q propagates through ANEL
    /// (clocked by NOT(XUPY)) → BYHA → ATEJ → ANOM before the counter
    /// reset reaches the scan counter. This chain takes long enough that
    /// the counter reset arrives AFTER the first XUPY rising edge (N+1),
    /// so the first compare+tick doesn't happen until XUPY N+2.
    /// dmg-sim confirms AVAP at BD=0, which requires 41 XUPY cycles from
    /// CATU capture (not 40).
    scan_start_delay: bool,
    /// Ten-entry sprite register file (page 30). Populated during Mode 2,
    /// consumed by X matchers during Mode 3.
    sprites: SpriteStore,
}

/// Signals produced by `SpriteScanner::fall()` for the rest of the pipeline.
pub(in crate::ppu) struct ScanSignals {
    /// AVAP fired this dot — scan complete (Mode 2 → 3 transition).
    pub(in crate::ppu) avap: bool,
}

impl SpriteScanner {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            counter: ScanCounter::new(),
            scanning: false,
            besu: false,
            catu_enabled: false,
            catu: false,
            rutu: false,
            byba: false,
            doba: false,
            avap_pending: false,
            scan_start_delay: false,
            sprites: SpriteStore::new(),
        }
    }

    /// Set scanning active for LCD-on initialization. On hardware, VID_RST
    /// deassertion releases the scan counter and comparison logic
    /// simultaneously — there is no separate CATU "start scanning" event
    /// on the first line. The counter is already at 0 from async reset.
    pub(in crate::ppu) fn start_scanning(&mut self) {
        self.scanning = true;
    }

    /// Whether the scan machinery is currently active.
    pub(in crate::ppu) fn scanning(&self) -> bool {
        self.scanning
    }

    /// BESU scanning latch — drives ACYL for STAT mode and OAM bus locking.
    pub(in crate::ppu) fn besu(&self) -> bool {
        self.besu
    }

    /// Whether CATU is pending — RUTU has been set at the scanline boundary
    /// but CATU hasn't fired yet. On hardware, the OAM bus is gated by the
    /// scan machinery even before BESU formally asserts. We use this to lock
    /// OAM during the pre-BESU dots at scanline boundaries.
    pub(in crate::ppu) fn catu_pending(&self) -> bool {
        self.rutu
    }

    /// Whether CATU is enabled (NOT first line after LCD-on).
    pub(in crate::ppu) fn catu_enabled(&self) -> bool {
        self.catu_enabled
    }

    /// Release VID_RST's blocking effect on CATU. Called after the first
    /// scanline completes (reset_scanline), enabling BESU on subsequent lines.
    pub(in crate::ppu) fn enable_catu(&mut self) {
        self.catu_enabled = true;
    }

    /// Current scan counter entry (0-39).
    pub(in crate::ppu) fn scan_counter_entry(&self) -> u8 {
        self.counter.entry()
    }

    /// BYBA state, for debug snapshot.
    pub(in crate::ppu) fn byba(&self) -> bool {
        self.byba
    }

    /// DOBA state, for debug snapshot.
    pub(in crate::ppu) fn doba(&self) -> bool {
        self.doba
    }

    /// Whether AVAP fired in the last fall(). Consumed by rise().
    pub(in crate::ppu) fn avap_pending(&self) -> bool {
        self.avap_pending
    }

    /// The OAM address the scanner is currently driving, if scanning.
    pub(in crate::ppu) fn oam_address(&self) -> Option<u8> {
        if self.scanning {
            Some(self.counter.oam_address())
        } else {
            None
        }
    }

    /// Read access to the sprite store for debug snapshots.
    pub(in crate::ppu) fn sprites_ref(&self) -> &SpriteStore {
        &self.sprites
    }

    /// Mutable access to the sprite store for X matching and marking fetched slots.
    pub(in crate::ppu) fn sprites_mut(&mut self) -> &mut SpriteStore {
        &mut self.sprites
    }

    /// Rising edge (master clock rises): BYBA captures scan_done(), AVAP
    /// evaluated, CATU scan-start fires, and AVAP-triggered scanning
    /// termination.
    ///
    /// BYBA captures scan_done() directly. Because rise() runs before
    /// fall() (which ticks the counter), BYBA sees the counter state
    /// from the previous XUPY cycle.
    ///
    /// On hardware, AVAP fires and BESU clears atomically in the same
    /// simulation pass. We match this by clearing scanning/besu here in
    /// rise(), alongside XYMU assertion (handled by the caller). Entry 39's
    /// OAM comparison runs before the clear, matching hardware where the
    /// comparison logic (COTA/WUDA) operates on separate clocks not gated
    /// by BESU.
    pub(in crate::ppu) fn rise(
        &mut self,
        _xupy_rising: bool,
        _ly: u8,
        _regs: &PipelineRegisters,
        _oam: &Oam,
    ) -> ScanSignals {
        // AVAP was already evaluated in fall() (after BYBA/DOBA captured).
        // Return the stored result so rendering.rise() can react to it.
        let avap = self.avap_pending;
        self.avap_pending = false;
        ScanSignals { avap }
    }

    /// Advance the CATU DFF — runs every dot regardless of VBlank.
    ///
    /// Single-stage: CATU captures RUTU directly on XUPY rising edge,
    /// gated by XYVO (VBlank suppression). RUTU is cleared after capture.
    ///
    /// On hardware, CATU has no POPU gate — it evaluates on every XUPY
    /// cycle. This method must be called unconditionally so the DFF
    /// can advance during the 153->0 frame boundary while POPU is
    /// still high.
    pub(in crate::ppu) fn tick_catu(&mut self, xupy_rising: bool, ly: u8) {
        // CATU DFF captures RUTU on XUPY rising edge, then propagates
        // through ANEL (clocked by NOT(XUPY)) → BYHA → ATEJ → ANOM
        // to drive the scan counter reset. This chain takes 1 full
        // XUPY cycle (2 dots) to complete.
        //
        // dmg-sim confirms: first XUPY rising after LCD-on is at BD=2
        // (120ns after VID_RST). AVAP fires at BD=0 (40 XUPY cycles
        // later). This means the scan counter starts on the SECOND
        // XUPY rising edge after CATU captures — the BD=0 edge.
        //
        // Model: CATU captures RUTU on XUPY N. Processing (scan start)
        // is gated on XUPY N+1, giving the 1-XUPY-cycle propagation delay.
        if xupy_rising && self.catu && !self.scanning {
            self.scanning = true;
            if self.catu_enabled {
                self.besu = true;
            }
            self.counter.reset();
            self.catu = false;
            // ANEL propagation delay: the counter reset arrives after
            // this XUPY edge, so the first compare+tick is suppressed
            // until the next XUPY rising.
            self.scan_start_delay = true;
        }

        // CATU DFF captures RUTU on XUPY rising.
        if xupy_rising {
            let xyvo = ly & 0x90 == 0x90;
            self.catu = self.rutu && !xyvo;
            self.rutu = false;
        }
    }

    /// Falling edge (master clock falls → alet rises): scanner tick and
    /// DOBA capture.
    pub(in crate::ppu) fn fall(
        &mut self,
        xupy_rising: bool,
        ly: u8,
        regs: &PipelineRegisters,
        oam: &Oam,
    ) {
        // OAM comparison and counter tick. Gated by scan_start_delay:
        // on the XUPY edge where CATU starts scanning, the ANEL
        // propagation delay prevents the counter from ticking. The
        // first compare+tick happens on the NEXT XUPY rising edge.
        if self.scan_start_delay && xupy_rising {
            self.scan_start_delay = false;
        } else {
            if self.scanning && xupy_rising {
                self.counter
                    .compare_and_store(ly, &mut self.sprites, regs, oam);
            }
            if xupy_rising {
                self.counter.tick_clock();
            }
        }

        // DOBA captures OLD BYBA (alet clock arrives after XUPY).
        self.doba = self.byba;

        // BYBA captures scan_done AFTER counter advance/freeze.
        // On hardware, BYBA and the counter share the XUPY clock.
        // When the counter reaches 39, FETO freezes it on the same
        // tick_clock call, and BYBA reads scan_done(39)=true on the
        // same XUPY edge — no extra cycle needed.
        if xupy_rising {
            self.byba = self.counter.scan_done();
        }

        // AVAP: combinational. new BYBA && !DOBA (which has old BYBA).
        let avap = self.byba && !self.doba;
        if avap && self.scanning {
            self.scanning = false;
            self.besu = false;
            self.avap_pending = true;
        }
    }

    /// Reset at scanline boundary. Sets rutu = true so the CATU DFF
    /// fires on the next XUPY rising edge (1 XUPY cycle = 2 dots).
    pub(in crate::ppu) fn reset(&mut self) {
        self.counter.reset();
        self.scanning = false;
        self.besu = false;
        self.sprites = SpriteStore::new();
        // BYBA/DOBA are not explicitly reset at line boundaries on hardware —
        // they naturally clear because FETO is false after counter reset.
        // But we reset them for cleanliness.
        self.byba = false;
        self.doba = false;
        self.avap_pending = false;
        self.scan_start_delay = false;
        self.catu = false;
        // RUTU fires at the scanline boundary. CATU captures it on the
        // next XUPY rising edge.
        self.rutu = true;
    }
}
