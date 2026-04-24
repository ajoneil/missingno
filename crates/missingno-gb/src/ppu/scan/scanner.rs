//! Sprite scanner — OAM scan (Mode 2) state machine.
//!
//! Netlist gate names (BYBA, DOBA, CATU, RUTU, BESU, from the dmgcpu
//! netlist, msinger/dmg-schematics) appear in doc comments for
//! traceability to the hardware signal chain.

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
    /// Controls counter ticking, OAM comparisons, and mode 3 gating in advance_scan().
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
    /// Scan-done flag. BYBA (dffr, clocked by XUPY — captures in fall).
    ///
    /// Combined with `scan_done_prev` for AVAP rising-edge detection:
    /// AVAP = BYBA && !DOBA.
    scan_done_flag: bool,
    /// Scan-done flag from the previous XUPY cycle. DOBA (dffr, clocked
    /// by ALET — captures in fall, one half-cycle after BYBA).
    ///
    /// Forms the rising-edge detector with `scan_done_flag` that
    /// produces AVAP.
    scan_done_prev: bool,
    /// AVAP result from fall(), consumed by rise(). On hardware AVAP
    /// is combinational (valid as soon as BYBA/DOBA settle in advance_scan()),
    /// but the rendering pipeline reacts in apply_pending_avap().
    avap_pending: bool,
    /// Ten-entry sprite register file (page 30). Populated during Mode 2,
    /// consumed by X matchers during Mode 3.
    sprites: SpriteStore,
}

/// Signals produced by `SpriteScanner::advance_scan()` for the rest of the pipeline.
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
            scan_done_flag: false,
            scan_done_prev: false,
            avap_pending: false,
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

    /// Scan-done flag (BYBA) state, for debug snapshot.
    pub(in crate::ppu) fn scan_done_flag(&self) -> bool {
        self.scan_done_flag
    }

    /// Previous scan-done flag (DOBA) state, for debug snapshot.
    pub(in crate::ppu) fn scan_done_prev(&self) -> bool {
        self.scan_done_prev
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

    /// Consume AVAP detection from the preceding `advance_scan()`; clear
    /// scanning/besu when AVAP fires. Called on the PPU-clock-fall phase
    /// (master-clock rise; gate: ALET falling), co-located with the
    /// rendering-side XYMU set.
    ///
    /// AVAP was evaluated combinationally when BYBA/DOBA settled. BYBA
    /// captures on XUPY rising, which in the WUVU/XOTA divider chain
    /// transitions shortly after PPU-clock-fall — both AVAP's rising
    /// edge and the BESU clear land here.
    pub(in crate::ppu) fn apply_pending_avap(
        &mut self,
        _xupy_rising: bool,
        _ly: u8,
        _regs: &PipelineRegisters,
        _oam: &Oam,
    ) -> ScanSignals {
        let avap = self.avap_pending;
        self.avap_pending = false;
        if avap {
            self.scanning = false;
            self.besu = false;
        }
        ScanSignals { avap }
    }

    /// Advance the CATU DFF — runs every dot regardless of VBlank.
    ///
    /// On the XUPY rising edge where RUTU is asserted (scanline
    /// boundary), CATU captures RUTU and the counter reset pulse
    /// asserts combinationally on the same edge via the
    /// NOT(CATU) → BYHA → ATEJ → ANOM path. ANEL's subsequent capture
    /// on the following XUPY falling edge releases the reset via
    /// BYHA's ANEL input path within the cycle. The counter sees
    /// `r_n = 1` by the next XUPY rising edge and ticks 0 → 1 there.
    ///
    /// Net: one XUPY cycle between CATU capture and the first counter
    /// tick. Modeled by doing capture + scan-start atomically on this
    /// edge; the first compare+tick runs in `advance_scan` on the
    /// following xupy_rising.
    ///
    /// CATU has no POPU gate on hardware — it evaluates on every XUPY
    /// cycle. This method must be called unconditionally so the DFF
    /// can advance during the 153→0 frame boundary while POPU is
    /// still high.
    pub(in crate::ppu) fn tick_catu(&mut self, xupy_rising: bool, ly: u8) {
        if !xupy_rising {
            return;
        }

        let xyvo = ly & 0x90 == 0x90;
        let catu_captures = self.rutu && !xyvo;
        self.rutu = false;

        if catu_captures && !self.scanning {
            self.scanning = true;
            if self.catu_enabled {
                self.besu = true;
            }
            self.counter.reset();
        }

        self.catu = catu_captures;
    }

    /// Advance one scan cycle: counter tick, BYBA/DOBA capture, AVAP
    /// combinational detection. Called on the PPU-clock-rise phase
    /// (master-clock fall; gate: ALET rising).
    pub(in crate::ppu) fn advance_scan(
        &mut self,
        xupy_rising: bool,
        ly: u8,
        regs: &PipelineRegisters,
        oam: &Oam,
    ) {
        // OAM comparison and counter tick. One XUPY after CATU's
        // same-edge capture+reset, the counter ticks 0 → 1 here;
        // subsequent edges walk 1..39.
        if self.scanning && xupy_rising {
            self.counter
                .compare_and_store(ly, &mut self.sprites, regs, oam);
        }
        if xupy_rising {
            self.counter.tick_clock();
        }

        // DOBA captures OLD BYBA (ALET clock arrives after XUPY).
        self.scan_done_prev = self.scan_done_flag;

        // BYBA captures scan_done AFTER counter advance/freeze.
        // On hardware, BYBA and the counter share the XUPY clock.
        // When the counter reaches 39, FETO freezes it on the same
        // tick_clock call, and BYBA reads scan_done(39)=true on the
        // same XUPY edge — no extra cycle needed.
        if xupy_rising {
            self.scan_done_flag = self.counter.scan_done();
        }

        // AVAP: combinational. new BYBA && !DOBA (which has old BYBA).
        // Detection fires here in advance_scan() (when BYBA captures and DOBA settles).
        // The scanning/besu clear is deferred to the next rise(), where
        // it co-locates with the rendering-side AVAP reaction — matching
        // hardware where XYMU set and BESU clear both occur at the alet
        // falling edge that follows BYBA's XUPY-rising capture.
        let avap = self.scan_done_flag && !self.scan_done_prev;
        if avap && self.scanning {
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
        self.scan_done_flag = false;
        self.scan_done_prev = false;
        self.avap_pending = false;
        self.catu = false;
        // RUTU fires at the scanline boundary. CATU captures it on the
        // next XUPY rising edge.
        self.rutu = true;
    }
}
