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
    /// STAT-readout mirror of `besu`. Captured at the start of each
    /// PPU-clock-fall from the pre-fall internal value, so `Ppu::mode()`
    /// sees the pre-transition value across the AVAP integer dot —
    /// matching GateBoy's gbtrace adapter sampling phase. Not a hardware
    /// DFF; this models the CPU's T-cycle STAT sampling window, which
    /// observes BESU before AVAP↑ clears it within the AVAP integer dot.
    /// Only read by `Ppu::mode()` via `besu_stat()`; all pipeline
    /// consumers (OAM lock, scan gate) continue to read internal `besu`.
    besu_stat: bool,
    /// Models NOT(VID_RST) for CATU gating. Starts false at LCD-on (VID_RST
    /// blocks CATU). Set to true by enable_catu() after the first scanline
    /// completes. Persists across scanline resets.
    catu_enabled: bool,
    /// Set by `start_scanning` to fold the post-XODO↓ first-XUPY phase
    /// offset (WUVU/VENA/TALU/XUPY divider ramp) onto the same fall.
    /// Consumed by the first `advance_scan` after start.
    first_line_xupy_shortcut: bool,
    /// CATU_LINE_ENDp DFF17: clocked by XUPY rising, D = ABOV_LINE_ENDp.
    /// Single-stage: boundary sets `rutu` → next XUPY rise CATU fires.
    catu: bool,
    /// RUTU pending input: written true by the scanline-boundary reset.
    /// Promoted to `rutu` by `tick_rutu`, which runs after `tick_catu`
    /// within the same PPU clock fall phase. This separation makes the
    /// one-XUPY-cycle latency between RUTU assertion and CATU capture
    /// explicit, independent of dispatch ordering within a fall phase.
    rutu_pending: bool,
    /// RUTU signal (output of the latch): set true by `tick_rutu` after
    /// the scanline-boundary write; cleared by `tick_catu` only on
    /// capture. Consumed by tick_catu on the next XUPY rising edge.
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
            besu_stat: false,
            catu_enabled: false,
            first_line_xupy_shortcut: false,
            catu: false,
            rutu_pending: false,
            rutu: false,
            scan_done_flag: false,
            scan_done_prev: false,
            avap_pending: false,
            sprites: SpriteStore::new(),
        }
    }

    /// Boot-ROM-handoff scanner state (spec §11.1): scan counter at
    /// terminal value 39 with GAVA frozen. Other latches stay at their
    /// power-on defaults (BESU=0, AVAP=0).
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            counter: ScanCounter::post_boot(),
            scanning: false,
            besu: false,
            besu_stat: false,
            catu_enabled: false,
            first_line_xupy_shortcut: false,
            catu: false,
            rutu_pending: false,
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
    /// Arms `first_line_xupy_shortcut` to absorb the divider ramp's
    /// sub-dot first-XUPY phase offset.
    pub(in crate::ppu) fn start_scanning(&mut self) {
        self.scanning = true;
        self.first_line_xupy_shortcut = true;
    }

    /// Whether the scan machinery is currently active.
    pub(in crate::ppu) fn scanning(&self) -> bool {
        self.scanning
    }

    /// BESU scanning latch — drives ACYL for STAT mode and OAM bus locking.
    pub(in crate::ppu) fn besu(&self) -> bool {
        self.besu
    }

    /// STAT-readout mirror of BESU (see `besu_stat` field). Lags the
    /// internal `besu` by one PPU-clock-fall. Read only by
    /// `Ppu::mode()` for the T-cycle STAT sampling window.
    pub(in crate::ppu) fn besu_stat(&self) -> bool {
        self.besu_stat
    }

    /// Capture the pre-fall `besu` into the STAT-readout mirror. Called
    /// at the start of every PPU-clock-fall, before any writer touches
    /// `self.besu` in that fall.
    pub(in crate::ppu) fn capture_besu_stat(&mut self) {
        self.besu_stat = self.besu;
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

        if catu_captures {
            // Hardware: CATU capturing + the downstream reset path
            // deasserts RUTU. Clear on capture only — an XYVO-gated XUPY
            // edge must not lose RUTU, since the latched model relies on
            // RUTU persisting until a non-blocked edge actually captures.
            self.rutu = false;
        }

        if catu_captures && !self.scanning {
            self.scanning = true;
            if self.catu_enabled {
                self.besu = true;
            }
            self.counter.reset();
        }

        self.catu = catu_captures;
    }

    /// Promote `rutu_pending` → `rutu` for the RUTU latch. Called after
    /// `tick_catu` within the same PPU clock fall phase, so the CATU
    /// reader at this fall sees the pre-promotion output and the first
    /// capture happens on the next XUPY rising edge.
    pub(in crate::ppu) fn tick_rutu(&mut self) {
        if self.rutu_pending {
            self.rutu = true;
            self.rutu_pending = false;
        }
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
        // First post-XODO↓ tick: divider-ramp shortcut absorbs the
        // sub-dot first-XUPY phase offset.
        let xupy_rising = xupy_rising || self.first_line_xupy_shortcut;
        self.first_line_xupy_shortcut = false;

        // OAM comparison. One XUPY after CATU's same-edge capture+reset,
        // the counter ticks 0 → 1 below; subsequent edges walk 1..39.
        if self.scanning && xupy_rising {
            self.counter
                .compare_and_store(ly, &mut self.sprites, regs, oam);
        }

        // DOBA captures OLD BYBA (ALET clock arrives after XUPY).
        self.scan_done_prev = self.scan_done_flag;

        // BYBA captures FETO sampled from the *pre-tick* counter.
        // FETO is a combinational AND4 over the counter bits; its
        // propagation depth exceeds BYBA's clock-to-Q path, so at an
        // XUPY rising edge BYBA's D-input reflects FETO over the
        // counter's pre-tick value. We model this by reading
        // `counter.scan_done()` before `counter.tick_clock()`. The
        // XUPY rising edge that ticks the counter to 39 captures
        // FETO(38) = 0; the next XUPY rising edge captures
        // FETO(39) = 1.
        if xupy_rising {
            self.scan_done_flag = self.counter.scan_done();
        }

        if xupy_rising {
            self.counter.tick_clock();
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

    /// Reset at scanline boundary. Writes the RUTU latch's pending input;
    /// `tick_rutu` promotes it to the visible `rutu` output after
    /// `tick_catu` has already read the pre-promotion value on this fall
    /// phase. The first CATU capture therefore lands on the *next* XUPY
    /// rising edge (1 XUPY cycle = 2 dots later).
    pub(in crate::ppu) fn reset(&mut self) {
        self.counter.reset();
        self.scanning = false;
        self.besu = false;
        self.besu_stat = false;
        self.sprites = SpriteStore::new();
        // BYBA/DOBA are not explicitly reset at line boundaries on hardware —
        // they naturally clear because FETO is false after counter reset.
        // But we reset them for cleanliness.
        self.scan_done_flag = false;
        self.scan_done_prev = false;
        self.avap_pending = false;
        self.catu = false;
        // Defensive: the shortcut is consumed on the first advance_scan
        // after start, so it should already be false here.
        self.first_line_xupy_shortcut = false;
        // RUTU_LINE_ENDp fires at the scanline boundary. Writing
        // `rutu_pending` (not `rutu`) keeps the latch input separate
        // from its output, so `tick_catu` at this fall reads the
        // pre-promotion `rutu` (false) and the first capture lands on
        // the next XUPY rising edge. `rutu` itself is intentionally not
        // overwritten — it may still be asserted from a previous
        // XYVO-gated line boundary, and must survive until captured.
        self.rutu_pending = true;
    }
}
