//! Mode 2 OAM scan state machine.

use crate::dma::OamBusOwner;
use crate::ppu::DffBit;
use crate::ppu::memory::Oam;

use super::oam_scan::{ScanCounter, SpriteStore};

/// Scan counter, BESU latch, BYBA/DOBA pipeline, and 10-entry sprite store. AVAP signals Mode 2→3.
pub(in crate::ppu) struct SpriteScanner {
    /// YFEL-FONY 6-bit scan counter + Y comparator.
    counter: ScanCounter,
    /// Active on all lines including LCD-on line 0.
    scanning: bool,
    /// BESU: Mode 2 OAM-scan + locks asserted (drives ACYL → STAT mode bits, OAM bus lock).
    /// Set by CATU only when scan_capture_armed; cleared on AVAP.
    mode2_active: bool,
    /// NOT(VID_RST) gate for CATU; false at LCD-on, set by arm_scan_capture() after the first scanline.
    scan_capture_armed: bool,
    /// CATU_LINE_ENDp DFF17 (XUPY-rising, D = ABOV_LINE_ENDp).
    scan_boundary_trigger: bool,
    /// RUTU nor_latch: set at scanline boundary by reset(), cleared by tick_scan_capture on capture.
    line_end: bool,
    /// DOBA DFF (dffr, ALET-clocked). D = BYBA (dffr, XUPY-clocked) scan-done;
    /// Q = the delayed copy. AVAP = BYBA && !DOBA, read via `pending`/`output`.
    scan_done: DffBit,
    sprites: SpriteStore,
}

pub(in crate::ppu) struct ScanSignals {
    /// AVAP — scan complete (Mode 2→3).
    pub(in crate::ppu) scan_complete: bool,
}

impl ScanSignals {
    /// XUPY low: the scan chain holds, so no AVAP.
    pub(in crate::ppu) const HELD: Self = Self {
        scan_complete: false,
    };
}

impl SpriteScanner {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            counter: ScanCounter::new(),
            scanning: false,
            mode2_active: false,
            scan_capture_armed: false,
            scan_boundary_trigger: false,
            line_end: false,
            scan_done: DffBit::new(false, false),
            sprites: SpriteStore::new(),
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            counter: ScanCounter::post_boot(),
            scanning: false,
            mode2_active: false,
            scan_capture_armed: true,
            scan_boundary_trigger: false,
            line_end: false,
            scan_done: DffBit::new(true, true),
            sprites: SpriteStore::new(),
        }
    }

    /// VID_RST deassertion releases the scan counter; no separate first-line CATU event.
    pub(in crate::ppu) fn start_scanning(&mut self) {
        self.scanning = true;
    }

    pub(in crate::ppu) fn scanning(&self) -> bool {
        self.scanning
    }

    pub(in crate::ppu) fn mode2_active(&self) -> bool {
        self.mode2_active
    }

    /// RUTU has been set at the scanline boundary but CATU hasn't fired yet — used to lock OAM
    /// pre-BESU.
    pub(in crate::ppu) fn scan_capture_pending(&self) -> bool {
        self.line_end
    }

    pub(in crate::ppu) fn scan_capture_armed(&self) -> bool {
        self.scan_capture_armed
    }

    /// Release VID_RST blocking on CATU after the first scanline completes.
    pub(in crate::ppu) fn arm_scan_capture(&mut self) {
        self.scan_capture_armed = true;
    }

    pub(in crate::ppu) fn scan_counter_entry(&self) -> u8 {
        self.counter.entry()
    }

    /// The whole chain is parked: no capture pending (RUTU low, CATU clear),
    /// BESU deasserted, the counter frozen on its FETO decode, and BYBA/DOBA
    /// both holding scan-done — so a scan tick rewrites what is already there.
    pub(in crate::ppu) fn chain_idle(&self) -> bool {
        !self.scanning
            && !self.mode2_active
            && !self.line_end
            && !self.scan_boundary_trigger
            && self.counter.frozen()
            && self.scan_done.pending()
            && self.scan_done.output()
    }

    pub(in crate::ppu) fn scan_done_flag(&self) -> bool {
        self.scan_done.pending()
    }

    pub(in crate::ppu) fn scan_done_prev(&self) -> bool {
        self.scan_done.output()
    }

    pub(in crate::ppu) fn oam_address(&self) -> Option<u8> {
        if self.scanning {
            Some(self.counter.oam_address())
        } else {
            None
        }
    }

    pub(in crate::ppu) fn sprites_ref(&self) -> &SpriteStore {
        &self.sprites
    }

    pub(in crate::ppu) fn sprites_mut(&mut self) -> &mut SpriteStore {
        &mut self.sprites
    }

    /// Overwrite the held Stage-1 byte-pair (a Mode-3 sprite fetch latching its
    /// (tile-index, attribute) into the dlatches shared with the Mode-2 scan).
    pub(in crate::ppu) fn set_stage1_held(&mut self, y: u8, x: u8) {
        self.counter.set_stage1_held(y, x);
    }

    /// Runs every XUPY cycle regardless of POPU (so the DFF advances across the 153→0 boundary).
    /// CATU captures atomically here; the first compare+tick runs in `advance_scan` next scan_clock_rising.
    /// Returns true on the XUPY edge where CATU captures RUTU — the ATEJ-pulse-rising event that
    /// asynchronously resets the shared `h_reset_n` consumers (PX bits, VOGA, scan counter via ANOM).
    pub(in crate::ppu) fn tick_scan_capture(&mut self, scan_clock_rising: bool, ly: u8) -> bool {
        if !scan_clock_rising {
            return false;
        }

        // XYVO = LY bit 7 & bit 4 — true for LY 144..=153 in practice (i.e. VBlank lines).
        let in_vblank_line = ly & 0x90 == 0x90;
        let scan_boundary_fires = self.line_end && !in_vblank_line;

        if scan_boundary_fires {
            // Capture deasserts RUTU; XYVO-gated edges must not lose RUTU.
            self.line_end = false;
        }

        if scan_boundary_fires && !self.scanning {
            self.scanning = true;
            if self.scan_capture_armed {
                self.mode2_active = true;
            }
            self.counter.reset();
        }

        self.scan_boundary_trigger = scan_boundary_fires;
        scan_boundary_fires
    }

    /// XUPY rising: counter tick + BYBA/DOBA capture + AVAP detection. The
    /// caller gates the call on that edge; off it the chain holds (`HELD`).
    pub(in crate::ppu) fn advance_scan(
        &mut self,
        ly: u8,
        sprite_height: u8,
        oam: &Oam,
        oam_bus: OamBusOwner,
    ) -> ScanSignals {
        // CARE (sprite save) requires BESU; on the LCD-on first line BESU never
        // sets, so the store stays empty though the counter still advances.
        if self.mode2_active {
            self.counter
                .compare_and_store(ly, &mut self.sprites, sprite_height, oam, oam_bus);
        }

        // DOBA captures OLD BYBA before BYBA captures FETO below.
        self.scan_done.tick();

        // BYBA captures FETO from the pre-tick counter (FETO's NAND4 depth exceeds BYBA's clock-to-Q).
        self.scan_done.write(self.counter.scan_done());

        self.counter.tick_clock();

        // AVAP detection + reaction co-locate (AVAP↑ and Mode 3 init on the same alet-falling edge).
        let scan_complete = self.scan_done.pending() && !self.scan_done.output() && self.scanning;
        if scan_complete {
            self.scanning = false;
            self.mode2_active = false;
        }
        ScanSignals { scan_complete }
    }

    /// Scanline boundary reset. RUTU is set here; tick_scan_capture captures on the next XUPY rising.
    pub(in crate::ppu) fn reset(&mut self) {
        self.counter.reset();
        self.scanning = false;
        self.mode2_active = false;
        self.sprites = SpriteStore::new();
        self.scan_done = DffBit::new(false, false);
        self.scan_boundary_trigger = false;
        self.line_end = true;
    }
}
