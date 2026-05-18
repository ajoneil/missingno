//! Mode 2 OAM scan state machine.

use crate::dma::OamBusOwner;
use crate::ppu::{PipelineRegisters, memory::Oam};

use super::oam_scan::{ScanCounter, SpriteStore};

/// Scan counter, BESU latch, BYBA/DOBA pipeline, and 10-entry sprite store. AVAP signals Mode 2→3.
pub(in crate::ppu) struct SpriteScanner {
    /// YFEL-FONY 6-bit scan counter + Y comparator.
    counter: ScanCounter,
    /// Active on all lines including LCD-on line 0.
    scanning: bool,
    /// BESU drives ACYL for STAT mode bits and OAM bus locking; set by CATU only when catu_enabled.
    besu: bool,
    /// NOT(VID_RST) gate for CATU; false at LCD-on, set by enable_catu() after the first scanline.
    catu_enabled: bool,
    /// CATU_LINE_ENDp DFF17 (XUPY-rising, D = ABOV_LINE_ENDp).
    catu: bool,
    /// RUTU nor_latch: set at scanline boundary by reset(), cleared by tick_catu on capture.
    rutu: bool,
    /// BYBA (dffr, XUPY-clocked).
    scan_done_flag: bool,
    /// DOBA (dffr, ALET-clocked); pairs with BYBA for AVAP = BYBA && !DOBA.
    scan_done_prev: bool,
    sprites: SpriteStore,
}

pub(in crate::ppu) struct ScanSignals {
    /// AVAP — scan complete (Mode 2→3).
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
            sprites: SpriteStore::new(),
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            counter: ScanCounter::post_boot(),
            scanning: false,
            besu: false,
            catu_enabled: true,
            catu: false,
            rutu: false,
            scan_done_flag: true,
            scan_done_prev: true,
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

    pub(in crate::ppu) fn besu(&self) -> bool {
        self.besu
    }

    /// RUTU has been set at the scanline boundary but CATU hasn't fired yet — used to lock OAM pre-BESU.
    pub(in crate::ppu) fn catu_pending(&self) -> bool {
        self.rutu
    }

    pub(in crate::ppu) fn catu_enabled(&self) -> bool {
        self.catu_enabled
    }

    /// Release VID_RST blocking on CATU after the first scanline completes.
    pub(in crate::ppu) fn enable_catu(&mut self) {
        self.catu_enabled = true;
    }

    pub(in crate::ppu) fn scan_counter_entry(&self) -> u8 {
        self.counter.entry()
    }

    pub(in crate::ppu) fn scan_done_flag(&self) -> bool {
        self.scan_done_flag
    }

    pub(in crate::ppu) fn scan_done_prev(&self) -> bool {
        self.scan_done_prev
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

    /// Runs every XUPY cycle regardless of POPU (so the DFF advances across the 153→0 boundary).
    /// CATU captures atomically here; the first compare+tick runs in `advance_scan` next xupy_rising.
    pub(in crate::ppu) fn tick_catu(&mut self, xupy_rising: bool, ly: u8) {
        if !xupy_rising {
            return;
        }

        let xyvo = ly & 0x90 == 0x90;
        let catu_captures = self.rutu && !xyvo;

        if catu_captures {
            // Capture deasserts RUTU; XYVO-gated edges must not lose RUTU.
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

    /// XUPY rising: counter tick + BYBA/DOBA capture + AVAP detection.
    pub(in crate::ppu) fn advance_scan(
        &mut self,
        xupy_rising: bool,
        ly: u8,
        regs: &PipelineRegisters,
        oam: &Oam,
        oam_bus: OamBusOwner,
    ) -> ScanSignals {
        if !xupy_rising {
            return ScanSignals { avap: false };
        }

        if self.scanning {
            self.counter
                .compare_and_store(ly, &mut self.sprites, regs, oam, oam_bus);
        }

        // DOBA captures OLD BYBA before BYBA captures FETO below.
        self.scan_done_prev = self.scan_done_flag;

        // BYBA captures FETO from the pre-tick counter (FETO's NAND4 depth exceeds BYBA's clock-to-Q).
        self.scan_done_flag = self.counter.scan_done();

        self.counter.tick_clock();

        // AVAP detection + reaction co-locate (AVAP↑ and Mode 3 init on the same alet-falling edge).
        let avap = self.scan_done_flag && !self.scan_done_prev && self.scanning;
        if avap {
            self.scanning = false;
            self.besu = false;
        }
        ScanSignals { avap }
    }

    /// Scanline boundary reset. RUTU is set here; tick_catu captures on the next XUPY rising.
    pub(in crate::ppu) fn reset(&mut self) {
        self.counter.reset();
        self.scanning = false;
        self.besu = false;
        self.sprites = SpriteStore::new();
        self.scan_done_flag = false;
        self.scan_done_prev = false;
        self.catu = false;
        self.rutu = true;
    }
}
