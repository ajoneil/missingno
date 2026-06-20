//! Register clock-domain crossing descriptors.
//!
//! A crossing is where a CPU-domain register value hands off to the PPU pixel
//! pipeline. The handoff has two independent components:
//!
//! - **the capture edge** — which clock edge the PPU samples the value on. On
//!   DMG the register couples combinationally (the PPU reads it the moment it is
//!   written); on the CGB it crosses into the PPU on a named M-cycle-clock edge.
//!   This is shared silicon: it is a function of the clock model, never of CGB
//!   data.
//! - **the CGB register-path offset** — a speed-independent extra hold the CGB
//!   silicon adds on top of the capture edge, present at every CGB speed
//!   (including single speed, ratio=1). This is CGB *data*, authored behind the
//!   `missingno-gbc` wall; it is **not** derived from the CPU:dot ratio.
//!
//! [`CaptureSpec`] composes the two by addition, never by ratio-derivation. The
//! ratio is deliberately not in scope of this descriptor — the (ii) clock-domain
//! phase skew rides the capture edge, and the (iv) offset rides `cgb_extra_falls`.

/// Which clock edge the PPU captures a crossed register on.
///
/// Today both variants resolve through the existing
/// `cpu_steps_per_dot()`/`HAS_CLOCK_DOMAIN_SYNC` phase machinery; a future
/// dual-clock-domain primitive will resolve `MCycleLastFall` to a concrete
/// master edge under the prevailing CPU:dot ratio. Naming the edge — rather than
/// carrying a raw "is synced" bool — is what lets that primitive drive the
/// crossing without the descriptor knowing the ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureEdge {
    /// The PPU reads the register on the same edge it is written — no clock-domain
    /// crossing. The DMG case, and the ratio=1 collapse of every crossing.
    Combinational,

    /// The register crosses into the PPU on the last PPU fall of the writing
    /// M-cycle. The base CGB capture edge before any register-path offset.
    MCycleLastFall,
}

/// A register clock-domain crossing: its capture edge plus the CGB
/// register-path offset layered on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSpec {
    /// Which clock edge the PPU samples the value on (shared silicon / clock
    /// model).
    pub capture: CaptureEdge,
    /// Extra falls the CGB register path holds the value beyond the capture edge
    /// (CGB data; 0 on DMG). **Not** a function of `cpu_steps_per_dot()`.
    pub cgb_extra_falls: u8,
}

impl CaptureSpec {
    /// A combinational crossing with no register-path offset — the DMG case and
    /// the default every crossing collapses to at ratio=1.
    pub const COMBINATIONAL: Self = Self {
        capture: CaptureEdge::Combinational,
        cgb_extra_falls: 0,
    };

    /// The total fall count to hand [`DffLatch::write_delayed`] for this
    /// crossing, reproducing the pre-descriptor `*_LAG_FALLS` constant exactly.
    ///
    /// The capture edge contributes the base "next fall" hold; the CGB offset
    /// rides on top. `write_delayed` itself takes the maximum with 1, so a
    /// combinational crossing returns 0 here and folds to the immediate-write
    /// path at the call site (the `> 0` guard goes `false`, DCE'ing the delayed
    /// arm on DMG).
    ///
    /// [`DffLatch::write_delayed`]: super::DffLatch::write_delayed
    pub const fn write_delayed_falls(&self) -> u8 {
        match self.capture {
            CaptureEdge::Combinational => self.cgb_extra_falls,
            // The CGB register-path offset already carries the base "next fall"
            // hold — `cgb_extra_falls` is the total, matching the historical
            // `SCY_WRITE_LAG_FALLS = 2` (a `base + offset` reading would
            // over-delay by one fall).
            CaptureEdge::MCycleLastFall => self.cgb_extra_falls,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ppu::DffLatch;

    /// Drive the real `DffLatch::write_delayed` path with a crossing and count
    /// the falls (ticks) until the staged value commits — the `commit_in` the
    /// latch actually emits.
    fn observed_commit_in(spec: CaptureSpec) -> u8 {
        let mut cell = DffLatch::new(0);
        cell.write_delayed(0xAB, spec.write_delayed_falls());
        let mut falls = 0u8;
        while !cell.tick() {
            falls += 1;
            assert!(falls < 16, "crossing never committed");
        }
        falls + 1
    }

    /// The DMG crossing is combinational: `write_delayed_falls()` is 0, so the
    /// `> 0` guard at the call site folds to false and the write takes the
    /// immediate path — never reaching `write_delayed`.
    #[test]
    fn combinational_folds_to_zero() {
        assert_eq!(CaptureSpec::COMBINATIONAL.write_delayed_falls(), 0);
    }

    /// A CGB-shaped SCY crossing (capture on the M-cycle last fall + a 2-fall
    /// register-path offset) must emit `commit_in == 2`, bit-identical to the
    /// pre-migration `SCY_WRITE_LAG_FALLS = 2`. A `base + offset = 1 + 2 = 3`
    /// reading would be a fidelity regression.
    #[test]
    fn cgb_scy_crossing_emits_commit_in_two() {
        let cgb_scy = CaptureSpec {
            capture: CaptureEdge::MCycleLastFall,
            cgb_extra_falls: 2,
        };
        assert_eq!(cgb_scy.write_delayed_falls(), 2);
        assert_eq!(observed_commit_in(cgb_scy), 2);
    }
}
