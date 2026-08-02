//! Machine-checked half of the dot-span claim (debug builds only).
//!
//! On every dot the span calls inert, the real edge bodies still run — on the
//! real state, since a fixed point cannot move it — and this module asserts
//! that they produced nothing beyond the sleeping-dot values the callers
//! consume. The skip is still taken afterwards, so the suites exercise the fast
//! path's plumbing while proving its premise.
//!
//! The full-state comparison behind that claim rides a fingerprint over
//! everything a skipped body can reach, built from `Hash` derives so a new
//! state field joins it automatically. In an unoptimised build — which is what
//! the suites run — that walk costs some 35× the emulation it guards, so it
//! samples: every stretch's first dot, then one dot in
//! [`DEEP_CHECK_STRIDE`]. The stride is prime, so successive samples land on
//! every divider and M-cycle phase rather than one.

use std::hash::{Hash, Hasher};

use crate::dma::OamBusOwner;

use super::{Ppu, PpuModel, PpuTickResult};

/// Slept dots between full-state comparisons.
pub(super) const DEEP_CHECK_STRIDE: u16 = 61;

impl<P: PpuModel> Ppu<P> {
    pub(super) fn verify_sleeping_rise(&mut self, vram: &P::Vram, oam_bus: OamBusOwner) {
        let before = self.span.deep_check().then(|| self.span_fingerprint());
        let result = self.run_rise(vram, oam_bus);
        assert_inert(&result, "rise");
        assert!(
            !self.check_stat_edge_body(),
            "sleeping dot raised a STAT edge"
        );
        if let Some(before) = before {
            assert_eq!(
                before,
                self.span_fingerprint(),
                "sleeping rise moved PPU state"
            );
        }
    }

    pub(super) fn verify_sleeping_fall(
        &mut self,
        is_mcycle: bool,
        mcycle_last_fall: bool,
        oam_bus: OamBusOwner,
        scan_clock_rising: bool,
        talu_rising: bool,
    ) {
        let before = self.span.deep_check().then(|| self.span_fingerprint());
        let mut result = PpuTickResult::default();
        self.run_fall(
            is_mcycle,
            mcycle_last_fall,
            oam_bus,
            scan_clock_rising,
            talu_rising,
            &mut result,
        );
        assert_inert(&result, "fall");
        if let Some(before) = before {
            assert_eq!(
                before,
                self.span_fingerprint(),
                "sleeping fall moved PPU state"
            );
        }
    }

    /// Everything a skipped body can reach. OAM is excluded: the scan chain is
    /// parked, so a sleeping dot neither reads nor writes it.
    fn span_fingerprint(&self) -> u64 {
        let mut hasher = Fnv1a(0xcbf2_9ce4_8422_2325);
        let Self {
            pixel_pipeline,
            registers,
            video,
            oam: _,
            frame_number,
            lcd_on_init_pending,
            oam_corruption,
            onset_settles,
            drawing_fall_stage,
            span: _,
            model,
        } = self;
        pixel_pipeline.hash(&mut hasher);
        registers.hash(&mut hasher);
        video.hash(&mut hasher);
        frame_number.hash(&mut hasher);
        lcd_on_init_pending.hash(&mut hasher);
        oam_corruption.hash(&mut hasher);
        onset_settles.hash(&mut hasher);
        drawing_fall_stage.hash(&mut hasher);
        model.hash(&mut hasher);
        hasher.finish()
    }
}

/// FNV-1a. `DefaultHasher`'s SipHash costs twice as much again in the
/// unoptimised build this only ever runs in.
struct Fnv1a(u64);

impl Hasher for Fnv1a {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ *byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// The sleeping-dot values every caller of this edge consumes: no pixel, no
/// frame, no LCD-off blank, and no interrupt request beyond the ones the
/// divider chain raised before the guard.
fn assert_inert<Pix>(result: &PpuTickResult<Pix>, edge: &str) {
    assert!(result.pixel.is_none(), "sleeping {edge} emitted a pixel");
    assert!(!result.new_frame, "sleeping {edge} ended a frame");
    assert!(!result.lcd_disabled, "sleeping {edge} disabled the LCD");
    assert!(
        !result.request_vblank,
        "sleeping {edge} requested a VBlank interrupt"
    );
    assert!(
        !result.request_stat,
        "sleeping {edge} requested a STAT interrupt"
    );
}
