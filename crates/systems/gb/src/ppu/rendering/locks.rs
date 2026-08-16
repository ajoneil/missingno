//! What the CPU may reach while the pipeline runs: the OAM and VRAM read and
//! write predicates, and the onset-contention signal set they share.

use super::Rendering;
use crate::ppu::{OnsetSignals, PpuModel};

impl<P: PpuModel> Rendering<P> {
    pub(in crate::ppu) fn oam_locked(&self) -> bool {
        // ACYL (BESU) or XYMU, plus the scan_capture_pending (RUTU set, BESU not yet asserted) pre-onset.
        self.oam_mode_locked() || self.scan.scan_capture_pending()
    }
    /// The pipeline's legs of the onset-contention signal set, read together.
    /// The vertical-blank leg belongs to the line counter and is not set here.
    pub(in crate::ppu) fn onset_signals(&self) -> OnsetSignals {
        let rendering = self.hblank.rendering_active();
        let mode2_bit = rendering || self.scan.mode2_active();
        let mut signals = OnsetSignals::empty();
        signals.set(OnsetSignals::RENDERING, rendering);
        signals.set(OnsetSignals::MODE2_BIT, mode2_bit);
        signals.set(
            OnsetSignals::OAM_LOCK,
            mode2_bit || self.scan.scan_capture_pending(),
        );
        signals
    }
    /// OAM locked by an active blocking mode (BESU/ACYL or XYMU), without the
    /// RUTU pre-onset window — a write landing before BESU is not blocked.
    pub(in crate::ppu) fn oam_mode_locked(&self) -> bool {
        self.scan.mode2_active() || self.hblank.rendering_active()
    }
    pub(in crate::ppu) fn vram_locked(&self) -> bool {
        self.hblank.rendering_active()
    }
    pub(in crate::ppu) fn oam_write_locked(&self) -> bool {
        // AJUJ = NOR3(dma_run, mode2, mode3) — write-permit override during the AVAP-cascade window.
        !self.hblank.access_permit_pulse()
            && (self.scan.mode2_active() || self.hblank.rendering_active())
    }
    pub(in crate::ppu) fn vram_write_locked(&self) -> bool {
        !self.hblank.access_permit_pulse() && self.hblank.rendering_active()
    }
}
